//! Pi Dashboard Rust rewrite entry point.
//!
//! - tokio `current_thread` single-thread runtime.
//! - No monitor-specific logic lives here; page registration, touch routing and
//!   IPC switch_mode are all handled by `pages::PageManager`.

// Several constants/helper functions are intentionally reserved for future pages.
#![allow(dead_code)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::{interval, MissedTickBehavior};
use tracing::{info, warn};

use crate::fb::Framebuffer;
use crate::ipc::{IpcCommand, IpcServer};
use crate::metrics::Metrics;
use crate::pages::cpu::CpuPage;
use crate::pages::disk::DiskPage;
use crate::pages::mem::MemPage;
use crate::pages::monitor::MonitorPage;
use crate::pages::net::NetPage;
use crate::pages::temp::TempPage;
use crate::pages::PageManager;
use crate::text::{draw_boot_screen, Fonts};
use crate::touch::TouchEvent;

mod chart;
mod config;
mod fb;
mod ipc;
mod label;
mod metrics;
mod pages;
mod render;
mod screenshot;
mod text;
mod touch;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    info!("Pi Dashboard Rust starting");

    let fb = Arc::new(Mutex::new(Framebuffer::open().await));
    let fonts = match Fonts::load() {
        Some(f) => f,
        None => {
            warn!("Failed to load fonts; aborting");
            return;
        }
    };

    // Boot screen while metrics warms up.
    {
        let mut guard = fb.lock().unwrap();
        draw_boot_screen(&mut *guard, &fonts);
        if let Err(e) = guard.full_flush() {
            warn!("Boot screen flush failed: {}", e);
        }
    }

    let metrics = Arc::new(Metrics::new());
    // Give the slow channel one tick to fetch initial data.
    tokio::time::sleep(Duration::from_millis(200)).await;

    let mut pages = PageManager::new();
    pages.register(Box::new(MonitorPage::new(fonts.clone())));
    pages.register(Box::new(TempPage::new(fonts.clone())));
    pages.register(Box::new(CpuPage::new(fonts.clone())));
    pages.register(Box::new(MemPage::new(fonts.clone())));
    pages.register(Box::new(DiskPage::new(fonts.clone())));
    pages.register(Box::new(NetPage::new(fonts.clone())));
    {
        let mut guard = fb.lock().unwrap();
        pages.switch("monitor", &mut *guard);
    }

    let (touch_tx, mut touch_rx) = mpsc::unbounded_channel::<TouchEvent>();
    tokio::spawn(touch::touch_task(touch_tx));

    let (ipc_tx, mut ipc_rx) = mpsc::channel::<IpcCommand>(16);
    IpcServer::new(fb.clone(), metrics.clone(), ipc_tx).start();

    let mut ticker = interval(Duration::from_millis(config::REFRESH_INTERVAL_MS));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        ticker.tick().await;

        // Drain touch events.
        while let Ok(ev) = touch_rx.try_recv() {
            match pages.route_touch(ev) {
                crate::pages::PageAction::Switch(id) => {
                    let mut guard = fb.lock().unwrap();
                    pages.switch(id, &mut *guard);
                }
                crate::pages::PageAction::None => {}
            }
        }

        // Drain IPC control commands.
        while let Ok(cmd) = ipc_rx.try_recv() {
            match cmd {
                IpcCommand::SwitchMode(id) => {
                    let mut guard = fb.lock().unwrap();
                    pages.switch(id, &mut *guard);
                }
                IpcCommand::ScrollContainers(respond) => {
                    let snap = metrics.snapshot();
                    let result = pages
                        .scroll_containers(snap.containers.len())
                        .ok_or_else(|| "not in monitor mode".to_string());
                    let _ = respond.send(result);
                }
            }
        }

        // Auto-return detail pages to monitor after inactivity.
        if let crate::pages::PageAction::Switch(home) = pages.check_idle_timeout() {
            let mut guard = fb.lock().unwrap();
            pages.switch(home, &mut *guard);
        }

        // Build snapshot for this frame.
        let mut snapshot = metrics.snapshot();
        snapshot.temp = metrics.temp();
        snapshot.mem = metrics.mem();
        snapshot.cpu = metrics.cpu();

        let mut guard = fb.lock().unwrap();
        if let Err(e) = pages.render_active(&mut *guard, &snapshot) {
            warn!("Render failed: {:?}", e);
            continue;
        }

        if let Err(e) = guard.flush_dirty() {
            warn!("Framebuffer flush failed: {}", e);
        }
    }
}

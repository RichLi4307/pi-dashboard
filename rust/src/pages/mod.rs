//! Page abstraction and manager.
//!
//! Mirrors `panel.py`'s mode framework. All page registration, touch routing
//! and IPC `switch_mode` go through `PageManager`; `main.rs` never contains
//! page-specific logic.

use std::collections::HashMap;

use anyhow::Result;

use crate::fb::Framebuffer;
use crate::metrics::MetricsSnapshot;
use crate::touch::TouchEvent;

pub mod cpu;
pub mod detail_common;
pub mod disk;
pub mod mem;
pub mod monitor;
pub mod net;
pub mod temp;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PageAction {
    None,
    /// Switch to the page with the given id.
    Switch(&'static str),
}

pub trait Page {
    fn id(&self) -> &'static str;

    /// Draw the page onto the framebuffer. Implementations must mark dirty
    /// regions for anything they change.
    fn render(&mut self, fb: &mut Framebuffer, snapshot: &MetricsSnapshot) -> Result<()>;

    /// Handle a touch event. Return a page action if the event should cause
    /// a page switch or other global effect.
    fn on_touch(&mut self, ev: TouchEvent) -> PageAction;

    /// Scroll the container list, returning `(offset, total)` if this page
    /// supports it. Default returns `None`.
    fn scroll_containers(&mut self, _total: usize) -> Option<(usize, usize)> {
        None
    }

    /// Called when this page becomes active. Default implementation marks the
    /// whole framebuffer dirty so the next render refreshes everything.
    fn on_enter(&mut self, fb: &mut Framebuffer) {
        fb.mark_full_dirty();
    }

    /// Called when this page is leaving. Default is a no-op.
    fn on_leave(&mut self, _fb: &mut Framebuffer) {}
}

/// Auto-return timeout for detail pages.
pub const DETAIL_IDLE_TIMEOUT_SECS: f32 = 60.0;

pub struct PageManager {
    pages: HashMap<&'static str, Box<dyn Page>>,
    active: &'static str,
    home_id: &'static str,
    last_activity: std::time::Instant,
}

impl PageManager {
    pub fn new() -> Self {
        Self {
            pages: HashMap::new(),
            active: "",
            home_id: "",
            last_activity: std::time::Instant::now(),
        }
    }

    pub fn register(&mut self, page: Box<dyn Page>) {
        let id = page.id();
        if self.active.is_empty() {
            self.active = id;
            self.home_id = id;
        }
        self.pages.insert(id, page);
    }

    pub fn switch(&mut self, id: &'static str, fb: &mut Framebuffer) -> bool {
        if id == self.active {
            self.last_activity = std::time::Instant::now();
            return true;
        }
        if !self.pages.contains_key(id) {
            return false;
        }
        if let Some(current) = self.pages.get_mut(self.active) {
            current.on_leave(fb);
        }
        self.active = id;
        self.last_activity = std::time::Instant::now();
        if let Some(page) = self.pages.get_mut(self.active) {
            page.on_enter(fb);
        }
        true
    }

    pub fn active_id(&self) -> &'static str {
        self.active
    }

    pub fn active(&mut self) -> Option<&mut Box<dyn Page>> {
        self.pages.get_mut(self.active)
    }

    pub fn route_touch(&mut self, ev: TouchEvent) -> PageAction {
        self.last_activity = std::time::Instant::now();
        if let Some(page) = self.pages.get_mut(self.active) {
            page.on_touch(ev)
        } else {
            PageAction::None
        }
    }

    /// Check idle timeout. Returns a Switch action if a detail page has been
    /// inactive longer than DETAIL_IDLE_TIMEOUT_SECS.
    pub fn check_idle_timeout(&mut self) -> PageAction {
        if self.active == self.home_id || self.home_id.is_empty() {
            return PageAction::None;
        }
        let elapsed = self.last_activity.elapsed().as_secs_f32();
        if elapsed >= DETAIL_IDLE_TIMEOUT_SECS {
            PageAction::Switch(self.home_id)
        } else {
            PageAction::None
        }
    }

    /// Refresh activity timestamp (e.g. on any render frame that is not a touch).
    pub fn bump_activity(&mut self) {
        self.last_activity = std::time::Instant::now();
    }

    pub fn render_active(&mut self, fb: &mut Framebuffer, snapshot: &MetricsSnapshot) -> Result<()> {
        if let Some(page) = self.pages.get_mut(self.active) {
            page.render(fb, snapshot)?;
        }
        Ok(())
    }

    pub fn scroll_containers(&mut self, total: usize) -> Option<(usize, usize)> {
        if let Some(page) = self.pages.get_mut(self.active) {
            page.scroll_containers(total)
        } else {
            None
        }
    }

    pub fn is_registered(&self, id: &str) -> bool {
        self.pages.contains_key(id)
    }
}

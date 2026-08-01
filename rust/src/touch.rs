//! Touch input: high-level gesture state machine on top of raw input events.
//!
//! Mirrors `pi_dashboard/touch.py` semantics but replaces the flood of raw
//! `pressed=true` events with discrete gestures:
//!
//! - `Press`  : finger went down (use for visual feedback only).
//! - `Release`: finger went up without qualifying as a tap.
//! - `Tap`    : short press (< TAP_MAX_MS) with little movement.
//! - `LongPress`: held still for LONG_PRESS_MS while pressed.
//! - `Drag`   : finger moved more than DRAG_THRESHOLD_PX while pressed.
//!
//! All timing is in **milliseconds**. The output event rate is decoupled from
//! the hardware report rate: drag events are coalesced and the main loop
//! consumes the queue at the display refresh interval (see `REFRESH_INTERVAL_MS`).
//!
//! Calibration: `touch-fix.service` already maps the physical panel to a
//! virtual device named `Touchscreen-Fixed` (`/dev/input/event1`). When this
//! node is used we must NOT apply `/etc/pointercal` a second time; the raw
//! coordinates are already screen-space. For any other device the legacy
//! `/etc/pointercal` matrix is applied and then clamped.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::io::{AsRawFd, RawFd};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::io::unix::AsyncFd;
use tokio::sync::mpsc::{self, UnboundedSender};
use tokio::time::{interval, sleep, MissedTickBehavior};
use tracing::{debug, warn};

use crate::config::{H, TOUCH_DEVICES, W};

pub const EVENT_SIZE: usize = 24;
pub const EV_KEY: u16 = 1;
pub const EV_ABS: u16 = 3;
pub const BTN_TOUCH: u16 = 330;
pub const ABS_X: u16 = 0;
pub const ABS_Y: u16 = 1;

/// High-level touch action produced by the gesture state machine.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TouchKind {
    /// Finger went down. Pages should use this only for visual feedback.
    Press,
    /// Finger went up and did not qualify as a tap.
    Release,
    /// Short press + little movement + release. This is the "click" action.
    Tap,
    /// Held still for `LONG_PRESS_MS` while pressed.
    LongPress,
    /// Finger moved while pressed. `dx`/`dy` are in screen pixels since the
    /// previously delivered `Drag` for this gesture.
    Drag { dx: i32, dy: i32 },
}

#[derive(Clone, Copy, Debug)]
pub struct TouchEvent {
    pub x: i32,
    pub y: i32,
    pub kind: TouchKind,
    /// Monotonic millisecond timestamp from the input subsystem.
    pub timestamp_ms: u64,
}

impl TouchEvent {
    /// Convenience helper for hit tests that want to accept both tap and
    /// long-press as activation.
    pub fn is_activate(&self) -> bool {
        matches!(self.kind, TouchKind::Tap | TouchKind::LongPress)
    }
}

// ---------------------------------------------------------------------------
// Timing and gesture thresholds (all milliseconds / pixels).
// ---------------------------------------------------------------------------

const TAP_MAX_MS: u64 = 200;
const LONG_PRESS_MS: u64 = 500;
/// Movement larger than this converts a press into a drag.
const DRAG_THRESHOLD_PX: i32 = 10;
/// Once dragging, only emit a Drag event after accumulating this many pixels.
const DRAG_SAMPLE_PX: i32 = 8;
/// Polling interval for long-press detection and for keeping the channel warm.
const STATE_MACHINE_TICK_MS: u64 = 50;

struct TouchDevice {
    file: File,
}

impl AsRawFd for TouchDevice {
    fn as_raw_fd(&self) -> RawFd {
        self.file.as_raw_fd()
    }
}

#[derive(Clone, Copy, Debug)]
struct DeviceInfo {
    path: &'static str,
    is_fixed: bool,
}

fn find_device() -> Option<DeviceInfo> {
    // 1. Prefer well-known symlinks (e.g. udev-created fixed node).
    for dev in TOUCH_DEVICES {
        if Path::new(dev).exists() {
            // The symlink name tells us whether this is the already-calibrated
            // virtual device. Legacy `/dev/input/event*` entries are checked
            // by name below.
            let is_fixed = dev.contains("touchscreen-fixed");
            return Some(DeviceInfo { path: dev, is_fixed });
        }
    }

    // 2. Fall back: scan /sys/class/input for the virtual calibrated device.
    if let Ok(entries) = std::fs::read_dir("/sys/class/input") {
        for entry in entries.flatten() {
            let name_file = entry.path().join("device/name");
            if let Ok(name) = std::fs::read_to_string(&name_file) {
                if name.trim() == "Touchscreen-Fixed" {
                    let node = format!("/dev/input/{}", entry.file_name().to_string_lossy());
                    // Safety: the string is short-lived but we need 'static for
                    // DeviceInfo. Leak it; this runs once at startup.
                    let path = Box::leak(node.into_boxed_str());
                    if Path::new(path).exists() {
                        return Some(DeviceInfo { path, is_fixed: true });
                    }
                }
            }
        }
    }

    None
}

fn load_pointercal() -> Option<[f32; 7]> {
    let text = std::fs::read_to_string("/etc/pointercal").ok()?;
    let parts: Vec<&str> = text.split_whitespace().collect();
    if parts.len() < 7 {
        return None;
    }
    let mut cal = [0.0f32; 7];
    for (i, p) in parts.iter().take(7).enumerate() {
        cal[i] = p.parse().ok()?;
    }
    Some(cal)
}

fn apply_calibration(raw_x: i32, raw_y: i32, is_fixed: bool, cal: Option<&[f32; 7]>) -> (i32, i32) {
    if is_fixed {
        // touch-fix.service already produced screen-space coordinates.
        return (raw_x.clamp(0, W as i32 - 1), raw_y.clamp(0, H as i32 - 1));
    }

    let (sx, sy) = match cal {
        Some(c) => {
            let a = c[0];
            let b = c[1];
            let cc = c[2];
            let d = c[3];
            let e = c[4];
            let f = c[5];
            let s = c[6];
            if s == 0.0 {
                (raw_x as f32, raw_y as f32)
            } else {
                (
                    (a * raw_x as f32 + b * raw_y as f32 + cc) / s,
                    (d * raw_x as f32 + e * raw_y as f32 + f) / s,
                )
            }
        }
        None => (raw_x as f32, raw_y as f32),
    };
    (sx.round() as i32, sy.round() as i32)
}

fn clamp_screen(x: i32, y: i32) -> (i32, i32) {
    (x.clamp(0, W as i32 - 1), y.clamp(0, H as i32 - 1))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn set_nonblock(fd: RawFd) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL, 0) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    let res = unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
    if res < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

async fn open_device() -> Option<(TouchDevice, DeviceInfo)> {
    loop {
        if let Some(info) = find_device() {
            match OpenOptions::new().read(true).open(info.path) {
                Ok(file) => {
                    let fd = file.as_raw_fd();
                    if set_nonblock(fd).is_ok() {
                        debug!(path = info.path, is_fixed = info.is_fixed, "Touch device opened");
                        return Some((TouchDevice { file }, info));
                    }
                }
                Err(e) => {
                    warn!("Failed to open touch device {}: {}", info.path, e);
                }
            }
        }
        warn!("No touch device available; retrying in 2s");
        sleep(Duration::from_secs(2)).await;
    }
}

fn parse_event(buf: &[u8]) -> Option<(u64, u16, u16, i32)> {
    if buf.len() < EVENT_SIZE {
        return None;
    }
    let sec = i64::from_le_bytes([buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7]]);
    let usec = i64::from_le_bytes([buf[8], buf[9], buf[10], buf[11], buf[12], buf[13], buf[14], buf[15]]);
    let ev_type = u16::from_le_bytes([buf[16], buf[17]]);
    let code = u16::from_le_bytes([buf[18], buf[19]]);
    let value = i32::from_le_bytes([buf[20], buf[21], buf[22], buf[23]]);
    let timestamp_ms = (sec * 1000 + usec / 1000) as u64;
    Some((timestamp_ms, ev_type, code, value))
}

/// Raw hardware events delivered from the kernel node.
#[derive(Clone, Copy, Debug)]
enum HwEvent {
    Down { x: i32, y: i32, timestamp_ms: u64 },
    Move { x: i32, y: i32, timestamp_ms: u64 },
    Up { x: i32, y: i32, timestamp_ms: u64 },
}

async fn read_loop(
    device: TouchDevice,
    is_fixed: bool,
    cal: Option<[f32; 7]>,
    tx: UnboundedSender<HwEvent>,
) -> io::Result<()> {
    let async_fd = AsyncFd::new(device)?;
    let mut raw_x = 0i32;
    let mut raw_y = 0i32;
    let mut pressed = false;
    let mut buf = [0u8; EVENT_SIZE];

    let calibrate = |rx, ry| {
        let (cx, cy) = apply_calibration(rx, ry, is_fixed, cal.as_ref());
        clamp_screen(cx, cy)
    };

    loop {
        let mut guard = async_fd.readable().await?;
        match guard.try_io(|_| {
            let fd = async_fd.get_ref().as_raw_fd();
            let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut _, buf.len()) };
            if n < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(n as usize)
            }
        }) {
            Ok(Ok(EVENT_SIZE)) => {
                if let Some((timestamp_ms, ev_type, code, value)) = parse_event(&buf) {
                    if ev_type == EV_KEY && code == BTN_TOUCH {
                        pressed = value != 0;
                        let (x, y) = calibrate(raw_x, raw_y);
                        if pressed {
                            let _ = tx.send(HwEvent::Down { x, y, timestamp_ms });
                        } else {
                            let _ = tx.send(HwEvent::Up { x, y, timestamp_ms });
                        }
                    } else if ev_type == EV_ABS {
                        if code == ABS_X {
                            raw_x = value;
                        } else if code == ABS_Y {
                            raw_y = value;
                        }
                        if pressed {
                            let (x, y) = calibrate(raw_x, raw_y);
                            let _ = tx.send(HwEvent::Move { x, y, timestamp_ms });
                        }
                    }
                }
            }
            Ok(Ok(0)) => {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "touch device closed"));
            }
            Ok(Ok(_)) => {}
            Ok(Err(e)) if e.kind() == io::ErrorKind::WouldBlock => {}
            Ok(Err(e)) => return Err(e),
            Err(_would_block) => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Gesture state machine.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
enum State {
    Idle,
    Pressed {
        origin_x: i32,
        origin_y: i32,
        last_x: i32,
        last_y: i32,
        start_ms: u64,
        long_sent: bool,
    },
    Dragging {
        last_sent_x: i32,
        last_sent_y: i32,
    },
}

struct StateMachine {
    state: State,
    tx: UnboundedSender<TouchEvent>,
}

impl StateMachine {
    fn new(tx: UnboundedSender<TouchEvent>) -> Self {
        Self { state: State::Idle, tx }
    }

    fn send(&self, x: i32, y: i32, kind: TouchKind, timestamp_ms: u64) {
        let _ = self.tx.send(TouchEvent { x, y, kind, timestamp_ms });
    }

    fn on_down(&mut self, x: i32, y: i32, timestamp_ms: u64) {
        self.state = State::Pressed {
            origin_x: x,
            origin_y: y,
            last_x: x,
            last_y: y,
            start_ms: timestamp_ms,
            long_sent: false,
        };
        self.send(x, y, TouchKind::Press, timestamp_ms);
    }

    fn on_move(&mut self, x: i32, y: i32, timestamp_ms: u64) {
        match self.state {
            State::Idle => {}
            State::Pressed { origin_x, origin_y, last_x, last_y, start_ms, long_sent } => {
                let dx = x - origin_x;
                let dy = y - origin_y;
                if dx.abs().max(dy.abs()) > DRAG_THRESHOLD_PX {
                    self.state = State::Dragging {
                        last_sent_x: x,
                        last_sent_y: y,
                    };
                    self.send(x, y, TouchKind::Drag { dx: x - last_x, dy: y - last_y }, timestamp_ms);
                } else {
                    self.state = State::Pressed {
                        origin_x,
                        origin_y,
                        last_x: x,
                        last_y: y,
                        start_ms,
                        long_sent,
                    };
                }
            }
            State::Dragging { last_sent_x, last_sent_y } => {
                if (x - last_sent_x).abs() >= DRAG_SAMPLE_PX || (y - last_sent_y).abs() >= DRAG_SAMPLE_PX {
                    self.state = State::Dragging { last_sent_x: x, last_sent_y: y };
                    self.send(x, y, TouchKind::Drag { dx: x - last_sent_x, dy: y - last_sent_y }, timestamp_ms);
                }
            }
        }
    }

    fn on_up(&mut self, x: i32, y: i32, timestamp_ms: u64) {
        match self.state {
            State::Idle => {}
            State::Pressed { origin_x, origin_y, start_ms, long_sent, .. } => {
                let duration = timestamp_ms.saturating_sub(start_ms);
                let moved = (x - origin_x).abs().max((y - origin_y).abs()) > DRAG_THRESHOLD_PX;
                if !moved && duration < TAP_MAX_MS {
                    self.send(x, y, TouchKind::Tap, timestamp_ms);
                } else if long_sent {
                    self.send(x, y, TouchKind::Release, timestamp_ms);
                } else {
                    self.send(x, y, TouchKind::Release, timestamp_ms);
                }
            }
            State::Dragging { .. } => {
                self.send(x, y, TouchKind::Release, timestamp_ms);
            }
        }
        self.state = State::Idle;
    }

    fn on_tick(&mut self, now_ms: u64) {
        if let State::Pressed { origin_x, origin_y, start_ms, long_sent, .. } = self.state {
            if !long_sent && now_ms.saturating_sub(start_ms) >= LONG_PRESS_MS {
                self.state = State::Pressed {
                    origin_x,
                    origin_y,
                    last_x: origin_x,
                    last_y: origin_y,
                    start_ms,
                    long_sent: true,
                };
                self.send(origin_x, origin_y, TouchKind::LongPress, now_ms);
            }
        }
    }
}

pub async fn touch_task(tx: UnboundedSender<TouchEvent>) {
    loop {
        if let Some((device, info)) = open_device().await {
            let cal = if info.is_fixed { None } else { load_pointercal() };
            let (hw_tx, mut hw_rx) = mpsc::unbounded_channel::<HwEvent>();

            let read_handle = tokio::spawn(read_loop(device, info.is_fixed, cal, hw_tx));

            let mut ticker = interval(Duration::from_millis(STATE_MACHINE_TICK_MS));
            ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
            let mut machine = StateMachine::new(tx.clone());

            loop {
                tokio::select! {
                    Some(ev) = hw_rx.recv() => {
                        match ev {
                            HwEvent::Down { x, y, timestamp_ms } => machine.on_down(x, y, timestamp_ms),
                            HwEvent::Move { x, y, timestamp_ms } => machine.on_move(x, y, timestamp_ms),
                            HwEvent::Up { x, y, timestamp_ms } => machine.on_up(x, y, timestamp_ms),
                        }
                    }
                    _ = ticker.tick() => {
                        machine.on_tick(now_ms());
                    }
                    else => break,
                }
            }

            read_handle.abort();
            warn!("Touch state machine loop ended; reconnecting...");
        }
        sleep(Duration::from_secs(1)).await;
    }
}

// ---------------------------------------------------------------------------
// Unit tests for the gesture state machine.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    fn make_machine() -> (StateMachine, mpsc::UnboundedReceiver<TouchEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (StateMachine::new(tx), rx)
    }

    #[test]
    fn tap_sequence() {
        let (mut m, mut rx) = make_machine();
        m.on_down(100, 200, 1000);
        m.on_up(101, 201, 1100);
        assert_eq!(rx.try_recv().unwrap().kind, TouchKind::Press);
        assert_eq!(rx.try_recv().unwrap().kind, TouchKind::Tap);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn drag_sequence() {
        let (mut m, mut rx) = make_machine();
        m.on_down(100, 200, 1000);
        // Small jitter stays as pressed.
        m.on_move(103, 200, 1010);
        assert_eq!(rx.try_recv().unwrap().kind, TouchKind::Press);
        assert!(rx.try_recv().is_err());
        // Cross threshold -> drag.
        m.on_move(115, 205, 1020);
        let ev = rx.try_recv().unwrap();
        assert!(matches!(ev.kind, TouchKind::Drag { .. }));
        // Coalesced until sampling threshold is exceeded.
        m.on_move(118, 207, 1030);
        assert!(rx.try_recv().is_err());
        m.on_move(125, 215, 1040);
        let ev = rx.try_recv().unwrap();
        assert!(matches!(ev.kind, TouchKind::Drag { dx, dy } if dx == 10 && dy == 10));
        m.on_up(125, 215, 1050);
        assert_eq!(rx.try_recv().unwrap().kind, TouchKind::Release);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn long_press() {
        let (mut m, mut rx) = make_machine();
        m.on_down(100, 200, 1000);
        assert_eq!(rx.try_recv().unwrap().kind, TouchKind::Press);
        m.on_tick(1499);
        assert!(rx.try_recv().is_err());
        m.on_tick(1500);
        assert_eq!(rx.try_recv().unwrap().kind, TouchKind::LongPress);
        m.on_up(100, 200, 1600);
        assert_eq!(rx.try_recv().unwrap().kind, TouchKind::Release);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn moving_prevents_long_press() {
        let (mut m, mut rx) = make_machine();
        m.on_down(100, 200, 1000);
        assert_eq!(rx.try_recv().unwrap().kind, TouchKind::Press);
        m.on_move(115, 200, 1200);
        assert!(matches!(rx.try_recv().unwrap().kind, TouchKind::Drag { .. }));
        m.on_tick(1600);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn touch_event_is_activate() {
        let tap = TouchEvent { x: 0, y: 0, kind: TouchKind::Tap, timestamp_ms: 0 };
        let long = TouchEvent { x: 0, y: 0, kind: TouchKind::LongPress, timestamp_ms: 0 };
        let press = TouchEvent { x: 0, y: 0, kind: TouchKind::Press, timestamp_ms: 0 };
        assert!(tap.is_activate());
        assert!(long.is_activate());
        assert!(!press.is_activate());
    }
}

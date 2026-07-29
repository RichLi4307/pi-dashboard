//! Touch input: bare `input_event` parsing, calibration, async delivery.
//!
//! Mirrors `pi_dashboard/touch.py`. No evdev crate; uses `tokio::io::unix::AsyncFd`
//! on an `O_NONBLOCK` fd.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::io::{AsRawFd, RawFd};
use std::path::Path;
use std::time::Duration;

use tokio::io::unix::AsyncFd;
use tokio::sync::mpsc::UnboundedSender;
use tokio::time::sleep;
use tracing::{debug, warn};

use crate::config::TOUCH_DEVICES;

pub const EVENT_SIZE: usize = 24;
pub const EV_KEY: u16 = 1;
pub const EV_ABS: u16 = 3;
pub const BTN_TOUCH: u16 = 330;
pub const ABS_X: u16 = 0;
pub const ABS_Y: u16 = 1;

#[derive(Clone, Copy, Debug)]
pub struct TouchEvent {
    pub x: i32,
    pub y: i32,
    pub pressed: bool,
    pub timestamp: f64,
}

struct TouchDevice {
    file: File,
}

impl AsRawFd for TouchDevice {
    fn as_raw_fd(&self) -> RawFd {
        self.file.as_raw_fd()
    }
}

fn find_device() -> Option<String> {
    for dev in TOUCH_DEVICES {
        if Path::new(dev).exists() {
            return Some(dev.to_string());
        }
    }
    None
}

fn load_calibration() -> Option<[f32; 7]> {
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

fn apply_calibration(raw_x: i32, raw_y: i32, cal: Option<&[f32; 7]>) -> (i32, i32) {
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
                (raw_x, raw_y)
            } else {
                (
                    ((a * raw_x as f32 + b * raw_y as f32 + cc) / s) as i32,
                    ((d * raw_x as f32 + e * raw_y as f32 + f) / s) as i32,
                )
            }
        }
        None => (raw_x, raw_y),
    };
    (sx.clamp(0, 479), sy.clamp(0, 319))
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

async fn open_device() -> Option<TouchDevice> {
    loop {
        if let Some(path) = find_device() {
            match OpenOptions::new().read(true).open(&path) {
                Ok(file) => {
                    let fd = file.as_raw_fd();
                    if set_nonblock(fd).is_ok() {
                        return Some(TouchDevice { file });
                    }
                }
                Err(e) => {
                    warn!("Failed to open touch device {}: {}", path, e);
                }
            }
        }
        warn!("No touch device available; retrying in 2s");
        sleep(Duration::from_secs(2)).await;
    }
}

fn parse_event(buf: &[u8]) -> Option<(f64, u16, u16, i32)> {
    if buf.len() < EVENT_SIZE {
        return None;
    }
    let sec = i64::from_le_bytes([buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7]]);
    let usec = i64::from_le_bytes([buf[8], buf[9], buf[10], buf[11], buf[12], buf[13], buf[14], buf[15]]);
    let ev_type = u16::from_le_bytes([buf[16], buf[17]]);
    let code = u16::from_le_bytes([buf[18], buf[19]]);
    let value = i32::from_le_bytes([buf[20], buf[21], buf[22], buf[23]]);
    let timestamp = sec as f64 + usec as f64 / 1_000_000.0;
    Some((timestamp, ev_type, code, value))
}

async fn read_loop(device: TouchDevice, tx: UnboundedSender<TouchEvent>) -> io::Result<()> {
    let async_fd = AsyncFd::new(device)?;
    let cal = load_calibration();
    let mut raw_x = 0i32;
    let mut raw_y = 0i32;
    let mut pressed = false;
    let mut buf = [0u8; EVENT_SIZE];

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
                if let Some((timestamp, ev_type, code, value)) = parse_event(&buf) {
                    if ev_type == EV_KEY && code == BTN_TOUCH {
                        pressed = value != 0;
                        if !pressed {
                            let (x, y) = apply_calibration(raw_x, raw_y, cal.as_ref());
                            let _ = tx.send(TouchEvent {
                                x,
                                y,
                                pressed: false,
                                timestamp,
                            });
                        }
                    } else if ev_type == EV_ABS {
                        if code == ABS_X {
                            raw_x = value;
                        } else if code == ABS_Y {
                            raw_y = value;
                        }
                        if pressed {
                            let (x, y) = apply_calibration(raw_x, raw_y, cal.as_ref());
                            let _ = tx.send(TouchEvent {
                                x,
                                y,
                                pressed: true,
                                timestamp,
                            });
                        }
                    }
                }
            }
            Ok(Ok(0)) => {
                // Device disconnected.
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "touch device closed"));
            }
            Ok(Ok(_)) => {
                // Partial event; wait for next readiness notification.
            }
            Ok(Err(e)) if e.kind() == io::ErrorKind::WouldBlock => {
                // No more data right now.
            }
            Ok(Err(e)) => return Err(e),
            Err(_would_block) => {
                // try_io already cleared readiness on EAGAIN; loop back.
            }
        }
    }
}

pub async fn touch_task(tx: UnboundedSender<TouchEvent>) {
    loop {
        if let Some(device) = open_device().await {
            debug!("Touch device opened");
            if let Err(e) = read_loop(device, tx.clone()).await {
                warn!("Touch read loop ended: {}; reconnecting...", e);
            }
        }
        sleep(Duration::from_secs(1)).await;
    }
}

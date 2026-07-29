//! Framebuffer abstraction.
//!
//! Maintains an in-memory RGB565 shadow buffer. Every draw call reports the
//! affected rectangle; at flush time dirty regions are merged and written to
//! `/dev/fb1` via `pwrite`-style `write_at`. No mmap, no nix.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::FileExt;
use std::path::Path;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{trace, warn};

use crate::config::{FB, H, W};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x1: usize,
    pub y1: usize,
    pub x2: usize, // exclusive
    pub y2: usize, // exclusive
}

impl Rect {
    pub fn new(x1: usize, y1: usize, x2: usize, y2: usize) -> Self {
        Self { x1, y1, x2, y2 }
    }

    pub fn width(&self) -> usize {
        self.x2.saturating_sub(self.x1)
    }

    pub fn height(&self) -> usize {
        self.y2.saturating_sub(self.y1)
    }

    pub fn area(&self) -> usize {
        self.width() * self.height()
    }

    pub fn clamp_to_screen(&mut self) {
        self.x1 = self.x1.min(W);
        self.y1 = self.y1.min(H);
        self.x2 = self.x2.min(W);
        self.y2 = self.y2.min(H);
        if self.x2 < self.x1 {
            self.x2 = self.x1;
        }
        if self.y2 < self.y1 {
            self.y2 = self.y1;
        }
    }

    pub fn union(&self, other: &Rect) -> Rect {
        Rect {
            x1: self.x1.min(other.x1),
            y1: self.y1.min(other.y1),
            x2: self.x2.max(other.x2),
            y2: self.y2.max(other.y2),
        }
    }
}

pub struct Framebuffer {
    pub width: usize,
    pub height: usize,
    buffer: Vec<u16>,
    dirty: Vec<Rect>,
    file: Option<File>,
}

impl Framebuffer {
    /// Open the framebuffer device. If it does not exist yet, retry with
    /// exponential back-off up to 5 s between attempts.
    pub async fn open() -> Self {
        let mut wait = Duration::from_millis(500);
        loop {
            match Self::try_open() {
                Ok(fb) => return fb,
                Err(e) => {
                    warn!("Framebuffer not ready: {e}; retrying in {:?}", wait);
                    sleep(wait).await;
                    wait = (wait * 2).min(Duration::from_secs(5));
                }
            }
        }
    }

    fn try_open() -> io::Result<Self> {
        let path = Path::new(FB);
        if !path.exists() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("{} does not exist", FB),
            ));
        }
        let file = OpenOptions::new().write(true).open(path)?;
        Ok(Self {
            width: W,
            height: H,
            buffer: vec![0u16; W * H],
            dirty: Vec::new(),
            file: Some(file),
        })
    }

    /// Create a headless framebuffer for tests (no device file).
    pub fn headless() -> Self {
        Self {
            width: W,
            height: H,
            buffer: vec![0u16; W * H],
            dirty: Vec::new(),
            file: None,
        }
    }

    /// Borrow the shadow buffer (read-only).
    pub fn buffer(&self) -> &[u16] {
        &self.buffer
    }

    /// Borrow the shadow buffer (mutable). Callers must mark dirty afterwards.
    pub fn buffer_mut(&mut self) -> &mut [u16] {
        &mut self.buffer
    }

    /// Mark the entire screen dirty.
    pub fn mark_full_dirty(&mut self) {
        self.dirty.clear();
        self.dirty.push(Rect::new(0, 0, W, H));
    }

    /// Mark a rectangular region dirty. Coordinates are clamped to the screen.
    pub fn mark_dirty(&mut self, mut rect: Rect) {
        rect.clamp_to_screen();
        if rect.area() == 0 {
            return;
        }
        self.dirty.push(rect);
    }

    /// Draw a single pixel in RGB565.
    pub fn set_pixel(&mut self, x: usize, y: usize, rgb565: u16) {
        if x >= W || y >= H {
            return;
        }
        let idx = y * W + x;
        if self.buffer[idx] != rgb565 {
            self.buffer[idx] = rgb565;
            self.mark_dirty(Rect::new(x, y, x + 1, y + 1));
        }
    }

    /// Clear the buffer to black and mark full dirty.
    pub fn clear(&mut self) {
        self.buffer.fill(0);
        self.mark_full_dirty();
    }

    /// Read-only view of current dirty regions (for tests).
    pub fn dirty_rects(&self) -> &[Rect] {
        &self.dirty
    }

    /// Clear the dirty list without flushing (for tests).
    pub fn clear_dirty(&mut self) {
        self.dirty.clear();
    }

    /// Write each dirty region to the framebuffer device.
    ///
    /// Regions are written individually (no forced merge) so that two distant
    /// small updates do not inflate into a giant bounding-box write. Overlapping
    /// regions are still fine: the later write simply overwrites the same pixels.
    /// Returns the total number of bytes written.
    pub fn flush_dirty(&mut self) -> io::Result<usize> {
        if self.dirty.is_empty() {
            return Ok(0);
        }

        // Merge only overlapping rects to keep write count low without inflating
        // distant updates into one giant box.
        let mut merged: Vec<Rect> = Vec::new();
        for r in self.dirty.drain(..) {
            let mut insert = r;
            let mut i = 0;
            while i < merged.len() {
                if Self::rects_overlap(&insert, &merged[i]) {
                    insert = insert.union(&merged.remove(i));
                } else {
                    i += 1;
                }
            }
            merged.push(insert);
        }

        let total_bytes: usize = merged.iter().map(|r| r.area() * 2).sum();

        let Some(file) = self.file.as_ref() else {
            return Ok(total_bytes);
        };

        for rect in &merged {
            let bytes_per_row = rect.width() * 2;
            for y in rect.y1..rect.y2 {
                let offset = (y * W + rect.x1) * 2;
                let start = y * W + rect.x1;
                let data = &self.buffer[start..start + rect.width()];
                // Safe: the slice is aligned to u16 and its length equals bytes_per_row/2.
                let bytes = unsafe {
                    std::slice::from_raw_parts(
                        data.as_ptr() as *const u8,
                        data.len() * 2,
                    )
                };
                match file.write_at(bytes, offset as u64) {
                    Ok(n) if n == bytes_per_row => {}
                    Ok(n) => {
                        warn!("Short framebuffer write at offset {}: {}/{} bytes", offset, n, bytes_per_row);
                    }
                    Err(e) => {
                        warn!("Framebuffer write failed at offset {}: {}", offset, e);
                        return Err(e);
                    }
                }
            }
        }

        trace!("flushed {} dirty rect(s) ({} bytes)", merged.len(), total_bytes);
        Ok(total_bytes)
    }

    fn rects_overlap(a: &Rect, b: &Rect) -> bool {
        a.x1 < b.x2 && a.x2 > b.x1 && a.y1 < b.y2 && a.y2 > b.y1
    }

    /// Force a full screen flush regardless of dirty state. Used for boot,
    /// page switches and recovery.
    pub fn full_flush(&mut self) -> io::Result<usize> {
        self.mark_full_dirty();
        self.flush_dirty()
    }
}

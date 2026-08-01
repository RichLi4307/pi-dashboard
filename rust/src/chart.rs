//! Line chart widget for detail pages.
//!
//! Renders a fixed-geometry chart with optional grid lines, threshold line,
//! and one or more data series. Follows the same redraw-on-change discipline as
//! Label/Bar: identical data = zero dirty pixels.

use crate::config::{ACCENT, BG, H, W};
use crate::fb::{Framebuffer, Rect};
use crate::render::{draw_line, fill_rect};

/// Vertical range mode.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RangeMode {
    /// Fixed min/max.
    Fixed { min: f32, max: f32 },
    /// Auto-scale from data max, rounded up via `nice_ceil`.
    Auto,
}

/// One series inside a chart.
#[derive(Clone, Debug, PartialEq)]
pub struct Series {
    pub data: Vec<f32>,
    pub color: u32,
}

/// Line chart widget. Rect and range are immutable after construction; data
/// changes through `set`.
pub struct LineChart {
    rect: Rect,
    range: RangeMode,
    threshold: Option<(f32, u32)>,
    last_series: Vec<Series>,
}

impl LineChart {
    /// Create a chart. The rectangle includes the plot area only (no labels).
    pub fn new(x: i32, y: i32, w: i32, h: i32, range: RangeMode) -> Self {
        let x1 = x.max(0) as usize;
        let y1 = y.max(0) as usize;
        let x2 = (x + w).min(W as i32).max(x1 as i32) as usize;
        let y2 = (y + h).min(H as i32).max(y1 as i32) as usize;
        Self {
            rect: Rect::new(x1, y1, x2, y2),
            range,
            threshold: None,
            last_series: Vec::new(),
        }
    }

    /// Set a horizontal threshold line (value, color). Use `None` to remove.
    pub fn set_threshold(&mut self, threshold: Option<(f32, u32)>) {
        self.threshold = threshold;
    }

    /// Update the chart data. Redraws only if the series changed.
    pub fn set(&mut self, fb: &mut Framebuffer, series: &[Series]) {
        if series == self.last_series {
            return;
        }
        self.last_series = series.to_vec();
        self.force_draw(fb);
    }

    /// Force a full redraw even if data is unchanged.
    pub fn force_draw(&mut self, fb: &mut Framebuffer) {
        let r = self.rect;
        // Erase background.
        fill_rect(fb, r.x1 as i32, r.y1 as i32, r.width() as i32, r.height() as i32, BG);

        // Grid: 25/50/75% horizontal lines.
        let w = r.width() as i32;
        let h = r.height() as i32;
        for frac in [0.25, 0.50, 0.75] {
            let gy = r.y2 as i32 - 1 - (h as f32 * frac) as i32;
            draw_line_h(fb, r.x1 as i32, r.x2 as i32, gy, ACCENT);
        }

        // Threshold line.
        if let Some((value, color)) = self.threshold {
            let (min, max) = self.effective_range(&[]);
            if value >= min && value <= max && max > min {
                let gy = self.value_to_y(value, min, max);
                draw_line_h(fb, r.x1 as i32, r.x2 as i32, gy, color);
            }
        }

        // Series lines.
        let (min, max) = self.effective_range(&self.last_series);
        for s in &self.last_series {
            let pts: Vec<(i32, i32)> = s
                .data
                .iter()
                .enumerate()
                .filter_map(|(i, &v)| {
                    if s.data.len() < 2 {
                        return None;
                    }
                    let x = r.x1 as i32 + (i as f32 * (w - 1) as f32 / (s.data.len() - 1) as f32) as i32;
                    let y = self.value_to_y(v.clamp(min, max), min, max);
                    Some((x, y))
                })
                .collect();
            for window in pts.windows(2) {
                draw_line(fb, window[0].0, window[0].1, window[1].0, window[1].1, s.color);
            }
        }

        // Mark whole rect dirty (background was fully erased).
        fb.mark_dirty(r);
    }

    fn effective_range(&self, series: &[Series]) -> (f32, f32) {
        match self.range {
            RangeMode::Fixed { min, max } => (min, max),
            RangeMode::Auto => {
                let mut max = 1.0f32;
                for s in series {
                    for &v in &s.data {
                        if v > max {
                            max = v;
                        }
                    }
                }
                (0.0, crate::metrics::nice_ceil(max))
            }
        }
    }

    fn value_to_y(&self, value: f32, min: f32, max: f32) -> i32 {
        let h = self.rect.height() as i32;
        if max <= min {
            return self.rect.y2 as i32 - 1;
        }
        let ratio = (value - min) / (max - min);
        self.rect.y2 as i32 - 1 - (ratio * (h - 1) as f32) as i32
    }
}

// Re-export for internal use without adding a render import elsewhere.
fn draw_line_h(fb: &mut Framebuffer, x1: i32, x2: i32, y: i32, color: u32) {
    crate::render::draw_line_h(fb, x1, x2, y, color);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (LineChart, Framebuffer) {
        let chart = LineChart::new(10, 10, 50, 30, RangeMode::Fixed { min: 0.0, max: 100.0 });
        let fb = Framebuffer::headless();
        (chart, fb)
    }

    #[test]
    fn chart_unchanged_zero_dirty() {
        let (mut chart, mut fb) = setup();
        let series = vec![Series {
            data: vec![10.0, 20.0, 30.0, 40.0],
            color: 0xffffff,
        }];
        chart.set(&mut fb, &series);
        fb.clear_dirty();
        chart.set(&mut fb, &series);
        assert!(fb.dirty_rects().is_empty());
    }

    #[test]
    fn chart_changed_marks_dirty() {
        let (mut chart, mut fb) = setup();
        chart.set(
            &mut fb,
            &[Series {
                data: vec![10.0, 20.0],
                color: 0xffffff,
            }],
        );
        fb.clear_dirty();
        chart.set(
            &mut fb,
            &[Series {
                data: vec![10.0, 30.0],
                color: 0xffffff,
            }],
        );
        assert!(!fb.dirty_rects().is_empty());
    }

    #[test]
    fn auto_range_uses_nice_ceil() {
        let mut chart = LineChart::new(10, 10, 50, 30, RangeMode::Auto);
        let mut fb = Framebuffer::headless();
        chart.set(
            &mut fb,
            &[Series {
                data: vec![12.0, 45.0, 8.0],
                color: 0xffffff,
            }],
        );
        // max data = 45, nice_ceil(45) = 50.
        assert_eq!(chart.last_series[0].data[1], 45.0);
    }
}

//! Field widgets: Label and Bar.
//!
//! A `Label` owns one mutable text field. It only redraws when its value
//! changes, and it erases the union of the old and new bounding boxes before
//! drawing. A `Bar` owns one percentage bar and redraws only when the value
//! changes.

use crate::config::{H, W};
use crate::fb::{Framebuffer, Rect};
use crate::render::fill_rect;
use crate::text::{Fonts, TextStyle};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// Compute the bounding box used for erasing `text` drawn at `(x, baseline_y)`.
/// Both width and height follow the real ink bounds (including left/right/overhang),
/// matching the pixels that `Fonts::draw` actually touches.
fn erase_bbox(text: &str, x: i32, baseline_y: i32, fonts: &Fonts, style: &TextStyle) -> Rect {
    let mut min_x = i32::MAX;
    let mut max_x = i32::MIN;
    let mut min_y = i32::MAX;
    let mut max_y = i32::MIN;
    let mut cursor = x as f32;
    let mut prev: Option<char> = None;

    for c in text.chars() {
        if let Some(p) = prev {
            cursor += fonts.kern(p, c, style);
        }
        let g = fonts.glyph_ref(c, style);
        let gx = (cursor + g.xmin as f32) as i32;
        // Match Fonts::draw: bitmap top on screen is baseline_y - (ymin + height) + 1.
        let gy = baseline_y - g.ymin - g.height as i32 + 1;
        min_x = min_x.min(gx);
        max_x = max_x.max(gx + g.width as i32);
        min_y = min_y.min(gy);
        max_y = max_y.max(gy + g.height as i32);
        cursor += g.advance;
        prev = Some(c);
    }

    if min_y >= max_y {
        // No visible glyphs (e.g. empty string): use a single-line placeholder.
        return Rect::new(x.max(0) as usize, baseline_y.max(0) as usize, x.max(0) as usize, (baseline_y + 1).max(0) as usize);
    }

    let x1 = min_x.max(0) as usize;
    let y1 = min_y.max(0) as usize;
    let x2 = max_x.min(W as i32).max(x1 as i32) as usize;
    let y2 = max_y.min(H as i32).max(y1 as i32) as usize;
    Rect::new(x1, y1, x2, y2)
}

pub struct Label {
    x: i32,
    baseline_y: i32,
    style: TextStyle,
    align: Align,
    bg: u32,
    last_text: String,
    last_bbox: Option<Rect>,
}

impl Label {
    /// Create a label. `top_y` is the nominal line top (ascender line); the
    /// internal `baseline_y` is computed from the font metrics and shared by all
    /// labels on the same row.
    pub fn new(
        x: i32,
        top_y: i32,
        style: TextStyle,
        align: Align,
        bg: u32,
        fonts: &Fonts,
    ) -> Self {
        let baseline_y = fonts.baseline_y(top_y, &style);
        Self {
            x,
            baseline_y,
            style,
            align,
            bg,
            last_text: String::new(),
            last_bbox: None,
        }
    }

    fn draw_x(&self, text: &str, fonts: &Fonts) -> i32 {
        match self.align {
            Align::Left => self.x,
            Align::Center => self.x - (fonts.measure(text, &self.style) as i32 / 2),
            Align::Right => self.x - fonts.measure(text, &self.style) as i32,
        }
    }

    /// Change the horizontal anchor. Takes effect on the next `set`/`force_draw`.
    pub fn set_x(&mut self, x: i32) {
        self.x = x;
    }

    /// Change the foreground colour. Takes effect on the next `set`/`force_draw`.
    pub fn set_style_color(&mut self, color: u32) {
        self.style.color = color;
    }

    /// Set the label text. Zero operation if unchanged.
    pub fn set(&mut self, fb: &mut Framebuffer, fonts: &Fonts, text: &str) {
        if text == self.last_text {
            return;
        }

        let draw_x = self.draw_x(text, fonts);
        let new_bbox = erase_bbox(text, draw_x, self.baseline_y, fonts, &self.style);
        let erase = self.last_bbox.map_or(new_bbox, |last| last.union(&new_bbox));
        fill_rect(fb, erase.x1 as i32, erase.y1 as i32, erase.width() as i32, erase.height() as i32, self.bg);
        fonts.draw(fb, text, draw_x, self.baseline_y, &self.style);

        self.last_text = text.to_string();
        self.last_bbox = Some(new_bbox);
    }

    /// Force a draw even if the text hasn't changed. Used for first render or
    /// after a full-screen clear.
    pub fn force_draw(&mut self, fb: &mut Framebuffer, fonts: &Fonts, text: &str) {
        self.last_text.clear();
        self.last_bbox = None;
        self.set(fb, fonts, text);
    }

    /// Explicitly clear the label area and reset state.
    pub fn clear(&mut self, fb: &mut Framebuffer) {
        if let Some(last) = self.last_bbox {
            fill_rect(fb, last.x1 as i32, last.y1 as i32, last.width() as i32, last.height() as i32, self.bg);
        }
        self.last_text.clear();
        self.last_bbox = None;
    }

    /// Direct access to the stored baseline for aligning sibling labels.
    pub fn baseline_y(&self) -> i32 {
        self.baseline_y
    }

    /// Direct access to the text style for external measurement alignment.
    pub fn style(&self) -> &TextStyle {
        &self.style
    }

    /// The bounding box of the currently displayed text, if any.
    pub fn bbox(&self) -> Option<Rect> {
        self.last_bbox
    }
}

pub struct Bar {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    track_color: u32,
    last_pct: i32,
    last_fill_color: Option<u32>,
}

impl Bar {
    pub fn new(x: i32, y: i32, w: i32, h: i32, track_color: u32) -> Self {
        Self {
            x,
            y,
            w,
            h,
            track_color,
            last_pct: -1,
            last_fill_color: None,
        }
    }

    fn fill_width(&self, pct: i32) -> i32 {
        ((self.w as f32 * pct as f32) / 100.0) as i32
    }

    /// Set the bar percentage (0–100). Redraws only on change.
    pub fn set(&mut self, fb: &mut Framebuffer, pct: f32, fill_color: u32) {
        let pct_i = pct.round() as i32;
        let color_changed = self.last_fill_color != Some(fill_color);
        if pct_i == self.last_pct && !color_changed {
            return;
        }

        let old_fill_w = self.fill_width(self.last_pct.max(0));
        let new_fill_w = self.fill_width(pct_i);

        let full_redraw = self.last_pct < 0 || color_changed;
        if full_redraw {
            // First draw or colour change: track + full fill.
            fill_rect(fb, self.x, self.y, self.w, self.h, self.track_color);
            if new_fill_w > 0 {
                fill_rect(fb, self.x, self.y, new_fill_w, self.h, fill_color);
            }
        } else if new_fill_w > old_fill_w {
            // Extend fill.
            fill_rect(fb, self.x + old_fill_w, self.y, new_fill_w - old_fill_w, self.h, fill_color);
        } else if new_fill_w < old_fill_w {
            // Shrink fill, reveal track.
            fill_rect(fb, self.x + new_fill_w, self.y, old_fill_w - new_fill_w, self.h, self.track_color);
        }
        // Equal: nothing changed.

        self.last_pct = pct_i;
        self.last_fill_color = Some(fill_color);
    }

    pub fn force_draw(&mut self, fb: &mut Framebuffer, pct: f32, fill_color: u32) {
        self.last_pct = -1;
        self.last_fill_color = None;
        self.set(fb, pct, fill_color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::{Fonts, TextStyle};

    fn setup() -> (Fonts, Framebuffer) {
        (Fonts::load().expect("fonts"), Framebuffer::headless())
    }

    #[test]
    fn label_unchanged_zero_dirty() {
        let (fonts, mut fb) = setup();
        let style = TextStyle::new(13, 0xffffff, false);
        let mut label = Label::new(8, 51, style, Align::Left, 0x0d1117, &fonts);
        label.set(&mut fb, &fonts, "TEMP");
        let dirty_after_first = fb.dirty_rects().len();
        assert!(dirty_after_first > 0);
        fb.clear_dirty();
        label.set(&mut fb, &fonts, "TEMP");
        assert!(fb.dirty_rects().is_empty());
    }

    #[test]
    fn label_change_erases_union() {
        let (fonts, mut fb) = setup();
        let style = TextStyle::new(13, 0xffffff, false);
        let mut label = Label::new(8, 51, style, Align::Left, 0x0d1117, &fonts);
        label.set(&mut fb, &fonts, "9%");
        fb.clear_dirty();
        label.set(&mut fb, &fonts, "63%");
        let r = fb.dirty_rects();
        assert!(!r.is_empty());
        // The reported dirty region(s) must cover at least the wider text.
        let union = r.iter().fold(r[0], |a, b| a.union(b));
        assert!(union.width() >= fonts.measure("63%", &style) as usize);
        assert!(union.height() > 0);
    }

    #[test]
    fn bar_unchanged_zero_dirty() {
        let (_, mut fb) = setup();
        let mut bar = Bar::new(52, 54, 130, 9, 0x30363d);
        bar.set(&mut fb, 50.0, 0x3fb950);
        fb.clear_dirty();
        bar.set(&mut fb, 50.0, 0x3fb950);
        assert!(fb.dirty_rects().is_empty());
    }

}

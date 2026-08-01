//! Shared geometry and helpers for v4 detail pages.

use crate::config::{ACCENT, BG, GRAY, H, PANEL, W, WHITE};
use crate::fb::{Framebuffer, Rect};
use crate::label::{Align, Label};
use crate::render::{draw_line_h, fill_rect, fill_rounded_rect};
use crate::text::{FontWeight, Fonts, TextStyle};
use crate::touch::TouchEvent;

// ---------------------------------------------------------------------------
// Layout constants (480×320).
// ---------------------------------------------------------------------------
pub const TOP_PANEL_H: i32 = 28;
pub const BACK_X: i32 = 12;
pub const BACK_Y: i32 = 4;
pub const BACK_W: i32 = 64;
pub const BACK_H: i32 = 20;
pub const BACK_R: i32 = 6;
pub const TITLE_X: i32 = 88;
pub const TITLE_Y: i32 = 6;
pub const BIG_VALUE_X: i32 = 12;
pub const BIG_VALUE_Y: i32 = 36;
pub const AUX_X: i32 = 250;
pub const AUX_Y: i32 = 44;
pub const CHART_X: i32 = 12;
pub const CHART_Y: i32 = 72;
pub const CHART_W: i32 = 456;
pub const CHART_H: i32 = 112;
pub const INFO_START_Y: i32 = 192;
pub const INFO_LINE_H: i32 = 14;
pub const INFO_LEFT_LABEL_X: i32 = 12;
pub const INFO_LEFT_VALUE_X: i32 = 130;
pub const INFO_RIGHT_LABEL_X: i32 = 246;
pub const INFO_RIGHT_VALUE_X: i32 = 364;

// ---------------------------------------------------------------------------
// Back button chip.
// ---------------------------------------------------------------------------
pub struct BackButton {
    bbox: Rect,
}

impl BackButton {
    pub fn new() -> Self {
        Self {
            bbox: Rect::new(
                BACK_X.max(0) as usize,
                BACK_Y.max(0) as usize,
                (BACK_X + BACK_W).min(W as i32) as usize,
                (BACK_Y + BACK_H).min(H as i32) as usize,
            ),
        }
    }

    pub fn draw(&self, fb: &mut Framebuffer, fonts: &Fonts) {
        fill_rounded_rect(fb, BACK_X, BACK_Y, BACK_W, BACK_H, BACK_R, PANEL);
        draw_chip_outline(fb, BACK_X, BACK_Y, BACK_W, BACK_H, BACK_R, ACCENT);
        let style = TextStyle::new(11, WHITE, false).with_weight(FontWeight::Regular);
        let text = "< BACK";
        let baseline_y = fonts.baseline_y(BACK_Y + (BACK_H - 11) / 2, &style);
        let w = fonts.measure(text, &style) as i32;
        let x = BACK_X + (BACK_W - w) / 2;
        fonts.draw(fb, text, x, baseline_y, &style);
    }

    pub fn hit(&self, ev: &TouchEvent) -> bool {
        ev.pressed
            && ev.x >= self.bbox.x1 as i32
            && ev.x < self.bbox.x2 as i32
            && ev.y >= self.bbox.y1 as i32
            && ev.y < self.bbox.y2 as i32
    }
}

fn draw_chip_outline(fb: &mut Framebuffer, x: i32, y: i32, w: i32, h: i32, r: i32, color: u32) {
    draw_line_h(fb, x + r, x + w - r, y, color);
    draw_line_h(fb, x + r, x + w - r, y + h - 1, color);
    fill_rect(fb, x, y + r, 1, h - 2 * r, color);
    fill_rect(fb, x + w - 1, y + r, 1, h - 2 * r, color);
}

// ---------------------------------------------------------------------------
// Title and big value.
// ---------------------------------------------------------------------------
pub fn draw_title(fb: &mut Framebuffer, fonts: &Fonts, title: &str) {
    let style = TextStyle::new(16, WHITE, false);
    let baseline_y = fonts.baseline_y(TITLE_Y, &style);
    fonts.draw(fb, title, TITLE_X, baseline_y, &style);
}

pub fn draw_big_value(fb: &mut Framebuffer, fonts: &Fonts, value: &str, color: u32) {
    let style = TextStyle::new(22, color, false);
    let baseline_y = fonts.baseline_y(BIG_VALUE_Y, &style);
    fonts.draw(fb, value, BIG_VALUE_X, baseline_y, &style);
}

pub fn draw_aux_text(fb: &mut Framebuffer, fonts: &Fonts, text: &str) {
    let style = TextStyle::new(11, GRAY, false).with_weight(FontWeight::Regular);
    let baseline_y = fonts.baseline_y(AUX_Y, &style);
    fonts.draw(fb, text, AUX_X, baseline_y, &style);
}

// ---------------------------------------------------------------------------
// Static background for detail pages.
// ---------------------------------------------------------------------------
pub fn draw_static_background(fb: &mut Framebuffer, fonts: &Fonts) {
    fill_rect(fb, 0, 0, W as i32, H as i32, BG);
    fill_rect(fb, 0, 0, W as i32, TOP_PANEL_H, PANEL);
    draw_line_h(fb, 0, W as i32, TOP_PANEL_H, ACCENT);
    BackButton::new().draw(fb, fonts);
}

// ---------------------------------------------------------------------------
// Info rows: two-column key-value pairs.
// ---------------------------------------------------------------------------
pub struct InfoRows {
    fonts: Fonts,
    left_labels: Vec<Label>,
    left_values: Vec<Label>,
    right_labels: Vec<Label>,
    right_values: Vec<Label>,
}

impl InfoRows {
    pub fn new(fonts: Fonts, rows: usize) -> Self {
        let label_style = TextStyle::new(11, GRAY, false).with_weight(FontWeight::Regular);
        let value_style = TextStyle::new(11, WHITE, false).with_weight(FontWeight::Regular);
        let mut left_labels = Vec::with_capacity(rows);
        let mut left_values = Vec::with_capacity(rows);
        let mut right_labels = Vec::with_capacity(rows);
        let mut right_values = Vec::with_capacity(rows);
        for i in 0..rows {
            let y = INFO_START_Y + i as i32 * INFO_LINE_H;
            left_labels.push(Label::new(INFO_LEFT_LABEL_X, y, label_style, Align::Left, BG, &fonts));
            left_values.push(Label::new(INFO_LEFT_VALUE_X, y, value_style, Align::Left, BG, &fonts));
            right_labels.push(Label::new(INFO_RIGHT_LABEL_X, y, label_style, Align::Left, BG, &fonts));
            right_values.push(Label::new(INFO_RIGHT_VALUE_X, y, value_style, Align::Left, BG, &fonts));
        }
        Self {
            fonts,
            left_labels,
            left_values,
            right_labels,
            right_values,
        }
    }

    pub fn set(&mut self, fb: &mut Framebuffer, row: usize, left: (&str, &str), right: (&str, &str)) {
        if row >= self.left_labels.len() {
            return;
        }
        self.left_labels[row].set(fb, &self.fonts, left.0);
        self.left_values[row].set(fb, &self.fonts, left.1);
        self.right_labels[row].set(fb, &self.fonts, right.0);
        self.right_values[row].set(fb, &self.fonts, right.1);
    }

    pub fn force_draw(&mut self, fb: &mut Framebuffer, row: usize, left: (&str, &str), right: (&str, &str)) {
        if row >= self.left_labels.len() {
            return;
        }
        self.left_labels[row].force_draw(fb, &self.fonts, left.0);
        self.left_values[row].force_draw(fb, &self.fonts, left.1);
        self.right_labels[row].force_draw(fb, &self.fonts, right.0);
        self.right_values[row].force_draw(fb, &self.fonts, right.1);
    }
}

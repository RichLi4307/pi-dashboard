//! Text engine: fontdue-backed glyph cache with font-level baseline anchoring.
//!
//! - Vertical anchor comes exclusively from `font.horizontal_line_metrics(px)`.
//! - `measure` returns advance width and shares the same stepping logic as `draw`.
//! - Glyph cache is single-threaded (`RefCell`) and warmed at startup for the
//!   ASCII range + common dashboard symbols at the sizes used by the monitor page.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use fontdue::{Font, FontSettings};
use tracing::warn;

use crate::config::{self, FONT_PATHS, H, REGULAR_FONT_PATHS, W};
use crate::fb::{Framebuffer, Rect};
use crate::render::{blend_over_rgb565, fill_rect, rgb888_to_rgb565};

/// Rasterised glyph bitmap and metrics.
pub struct GlyphBitmap {
    pub width: usize,
    pub height: usize,
    pub xmin: i32,
    pub ymin: i32,
    pub advance: f32,
    pub data: Vec<u8>,
}

/// Font weight selector. The dashboard keeps a loaded instance for each weight
/// so small sizes can pick the weight that remains legible on the 480×320 panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FontWeight {
    Regular,
    Medium,
}

/// Text appearance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextStyle {
    pub size: u16,
    pub color: u32,
    pub mono: bool,
    pub weight: FontWeight,
}

impl TextStyle {
    pub fn new(size: u16, color: u32, mono: bool) -> Self {
        Self {
            size,
            color,
            mono,
            weight: FontWeight::Medium,
        }
    }

    pub fn with_weight(mut self, weight: FontWeight) -> Self {
        self.weight = weight;
        self
    }
}

#[derive(Clone)]
pub struct Fonts {
    sans: Font,
    mono: Font,
    regular: Font,
    cache: RefCell<HashMap<(char, u16, bool, FontWeight), Rc<GlyphBitmap>>>,
    /// Font-level offset from nominal line-top `y` to the baseline. Computed
    /// from the most overhanging glyph in the warmed character set so that the
    /// visual top of text sits at `y`, matching PIL's top-anchor semantics.
    top_offsets: RefCell<HashMap<(u16, bool, FontWeight), i32>>,
}

impl Fonts {
    pub fn load() -> Option<Self> {
        let sans = Self::load_first(FONT_PATHS)?;
        // Use the same font instance for mono to avoid loading the large TTF
        // twice into memory. The cache key still distinguishes mono=false/true
        // for future separation, but for now they share glyphs.
        let mono = sans.clone();
        // Regular is used at small sizes where Medium strokes blur together.
        // Load the ASCII subset first to keep memory tiny.
        let regular = Self::load_first(REGULAR_FONT_PATHS)?;
        let cache = RefCell::new(HashMap::new());
        let top_offsets = RefCell::new(HashMap::new());
        let fonts = Self {
            sans,
            mono,
            regular,
            cache,
            top_offsets,
        };
        fonts.warm_common_sizes();
        Some(fonts)
    }

    fn load_first(paths: &[&str]) -> Option<Font> {
        for path in paths {
            if let Ok(bytes) = std::fs::read(path) {
                match Font::from_bytes(bytes, FontSettings::default()) {
                    Ok(font) => return Some(font),
                    Err(e) => warn!("Font load failed for {}: {:?}", path, e),
                }
            }
        }
        warn!("No font could be loaded from {:?}", paths);
        None
    }

    fn font(&self, style: &TextStyle) -> &Font {
        if style.mono {
            &self.mono
        } else {
            match style.weight {
                FontWeight::Regular => &self.regular,
                FontWeight::Medium => &self.sans,
            }
        }
    }

    fn rasterize(&self, c: char, style: &TextStyle) -> Rc<GlyphBitmap> {
        let font = self.font(style);
        let idx = font.lookup_glyph_index(c);
        let (metrics, data) = font.rasterize_indexed(idx, style.size as f32);
        Rc::new(GlyphBitmap {
            width: metrics.width,
            height: metrics.height,
            xmin: metrics.xmin,
            ymin: metrics.ymin,
            advance: metrics.advance_width,
            data,
        })
    }

    pub(crate) fn glyph(&self, c: char, style: &TextStyle) -> Rc<GlyphBitmap> {
        let key = (c, style.size, style.mono, style.weight);
        if let Some(g) = self.cache.borrow().get(&key) {
            return g.clone();
        }
        let g = self.rasterize(c, style);
        self.cache.borrow_mut().insert(key, g.clone());
        g
    }

    /// Borrow a cached glyph without increasing the strong count. For internal
    /// layout calculations that only need a temporary reference.
    pub(crate) fn glyph_ref(&self, c: char, style: &TextStyle) -> std::cell::Ref<'_, GlyphBitmap> {
        let key = (c, style.size, style.mono, style.weight);
        if self.cache.borrow().get(&key).is_none() {
            let g = self.rasterize(c, style);
            self.cache.borrow_mut().insert(key, g);
        }
        // RefCell guarantees we are on a single thread; the borrow lives as long
        // as the returned Ref, preventing mutation while it is held.
        std::cell::Ref::map(self.cache.borrow(), |cache| {
            cache.get(&key).unwrap().as_ref()
        })
    }

    /// Kerning between two characters for a style.
    pub(crate) fn kern(&self, prev: char, next: char, style: &TextStyle) -> f32 {
        self.font(style)
            .horizontal_kern(prev, next, style.size as f32)
            .unwrap_or(0.0)
    }

    /// Representative characters used to determine the per-style ascent. These
    /// cover the dashboard's labels without including extreme descenders (e.g.
    /// 'g', ',') that would push the baseline far below the visual line top.
    const REP_CHARS: &[char] = &[
        'A','B','C','D','E','F','G','H','I','J','K','L','M','N','O','P','Q','R','S','T','U','V','W','X','Y','Z',
        'a','b','c','d','e','f','h','i','j','k','l','m','n','o','p','q','r','s','t','u','v','w','x','y','z',
        '0','1','2','3','4','5','6','7','8','9','%','/',':','.','-',
    ];

    /// Pre-rasterise representative characters for the sizes/weights used by the
    /// monitor page. Prevents first-frame rasterisation jitter and populates
    /// the per-style top-offset table.
    fn warm_common_sizes(&self) {
        let sizes = [10u16, 11, 13, 16];
        for &size in &sizes {
            for &mono in &[false, true] {
                for &weight in &[FontWeight::Regular, FontWeight::Medium] {
                    let style = TextStyle { size, color: 0, mono, weight };
                    let off = self.compute_top_offset(&style);
                    self.top_offsets.borrow_mut().insert((size, mono, weight), off);
                }
            }
        }
    }

    fn compute_top_offset(&self, style: &TextStyle) -> i32 {
        let mut max_top = 0i32;
        for &c in Self::REP_CHARS {
            let g = self.glyph(c, style);
            // fontdue's ymin is the offset from baseline to the bitmap bottom
            // (y-up). The bitmap top in that coordinate system is ymin+height.
            // We want the distance from baseline to the top of the tallest
            // glyph, minus 1 because rows are 0-indexed.
            max_top = max_top.max(g.ymin + g.height as i32);
        }
        max_top - 1
    }

    /// Warm the cache for a specific style. Call when a page introduces a new
    /// size to avoid first-frame rasterisation cost.
    pub fn warm(&self, style: &TextStyle) {
        let off = self.compute_top_offset(style);
        self.top_offsets
            .borrow_mut()
            .insert((style.size, style.mono, style.weight), off);
    }

    /// Font-level top offset for a style: distance from nominal line-top `y` to
    /// the baseline. Determined by the tallest glyph in the representative set,
    /// so the visual top of text aligns with `y` (PIL-compatible).
    pub fn top_offset(&self, style: &TextStyle) -> i32 {
        let key = (style.size, style.mono, style.weight);
        if let Some(&off) = self.top_offsets.borrow().get(&key) {
            return off;
        }
        // Fallback: compute on demand for sizes not warmed at startup.
        let off = self.compute_top_offset(style);
        self.top_offsets.borrow_mut().insert(key, off);
        off
    }

    /// Convert a nominal line-top `y` into the baseline y coordinate used for
    /// drawing. All text segments on the same line must share this value.
    pub fn baseline_y(&self, y: i32, style: &TextStyle) -> i32 {
        y + self.top_offset(style)
    }

    /// Measure the advance width of `text` in pixels.
    pub fn measure(&self, text: &str, style: &TextStyle) -> f32 {
        let font = self.font(style);
        let px = style.size as f32;
        let mut cursor = 0.0f32;
        let mut prev: Option<char> = None;
        for c in text.chars() {
            if let Some(p) = prev {
                cursor += font.horizontal_kern(p, c, px).unwrap_or(0.0);
            }
            let g = self.glyph(c, style);
            cursor += g.advance;
            prev = Some(c);
        }
        cursor
    }

    /// Draw `text` at `(x, baseline_y)`. Returns the advance width.
    ///
    /// `baseline_y` must be computed with `Fonts::baseline_y` so that all text
    /// segments on the same line share the exact same vertical anchor.
    pub fn draw(
        &self,
        fb: &mut Framebuffer,
        text: &str,
        x: i32,
        baseline_y: i32,
        style: &TextStyle,
    ) -> f32 {
        let font = self.font(style);
        let px = style.size as f32;
        let _rgb565 = rgb888_to_rgb565(style.color);
        let mut cursor = x as f32;
        let mut prev: Option<char> = None;
        let mut min_x = i32::MAX;
        let mut max_x = i32::MIN;
        let mut min_y = i32::MAX;
        let mut max_y = i32::MIN;

        for c in text.chars() {
            if let Some(p) = prev {
                cursor += font.horizontal_kern(p, c, px).unwrap_or(0.0);
            }
            let g = self.glyph(c, style);
            let gx = (cursor + g.xmin as f32) as i32;
            // fontdue stores the bitmap with row 0 at the top and y increasing
            // downward within the bitmap. `ymin` is the offset from baseline to
            // the bitmap bottom in fontdue's y-up space, so the bitmap top on
            // screen is baseline_y - (ymin + height) + 1.
            let gy = baseline_y - g.ymin - g.height as i32 + 1;

            if !g.data.is_empty() {
                for row in 0..g.height {
                    let sy = gy + row as i32;
                    if sy < 0 || sy >= H as i32 {
                        continue;
                    }
                    for col in 0..g.width {
                        let sx = gx + col as i32;
                        if sx < 0 || sx >= W as i32 {
                            continue;
                        }
                        let alpha = g.data[row * g.width + col];
                        if alpha == 0 {
                            continue;
                        }
                        let idx = sy as usize * W + sx as usize;
                        let dst = fb.buffer_mut()[idx];
                        fb.buffer_mut()[idx] = blend_over_rgb565(dst, style.color, alpha);
                    }
                }
            }

            min_x = min_x.min(gx);
            max_x = max_x.max(gx + g.width as i32);
            min_y = min_y.min(gy);
            max_y = max_y.max(gy + g.height as i32);

            cursor += g.advance;
            prev = Some(c);
        }

        if min_x < max_x && min_y < max_y {
            let x1 = min_x.max(0) as usize;
            let y1 = min_y.max(0) as usize;
            let x2 = max_x.min(W as i32).max(x1 as i32) as usize;
            let y2 = max_y.min(H as i32).max(y1 as i32) as usize;
            fb.mark_dirty(Rect::new(x1, y1, x2, y2));
        }

        cursor - x as f32
    }
}

/// Draw the boot screen onto the framebuffer.
pub fn draw_boot_screen(fb: &mut Framebuffer, fonts: &Fonts) {
    fill_rect(fb, 0, 0, W as i32, H as i32, config::BG);
    let style18 = TextStyle::new(18, config::WHITE, false);
    let style14 = TextStyle::new(14, config::GRAY, false);
    let style12 = TextStyle::new(12, config::GRAY, false);
    let cx = W as i32 / 2;
    let title = "System Booting...";
    let wait = "Waiting for Docker";
    let hint = "Please wait";
    let tw = fonts.measure(title, &style18);
    fonts.draw(fb, title, cx - (tw / 2.0) as i32, fonts.baseline_y(H as i32 / 2 - 30, &style18), &style18);
    let tw = fonts.measure(wait, &style14);
    fonts.draw(fb, wait, cx - (tw / 2.0) as i32, fonts.baseline_y(H as i32 / 2, &style14), &style14);
    let tw = fonts.measure(hint, &style12);
    fonts.draw(fb, hint, cx - (tw / 2.0) as i32, fonts.baseline_y(H as i32 / 2 + 25, &style12), &style12);
}

/// Draw a centered overlay with the given text, covering the existing image.
pub fn draw_overlay(fb: &mut Framebuffer, fonts: &Fonts, text: &str) {
    let size = 14u16;
    let style = TextStyle::new(size, config::WHITE, false);
    let tw = fonts.measure(text, &style) as i32;
    let th = size as i32;
    let ox = (W as i32 - tw) / 2 - 10;
    let oy = (H as i32 - th) / 2 - 6;
    fill_rect(fb, ox, oy, tw + 20, th + 12, config::OVERLAY_BG);
    fonts.draw(fb, text, ox + 10, fonts.baseline_y(oy + 6 - 2, &style), &style);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fonts() -> Fonts {
        Fonts::load().expect("fonts should load")
    }

    #[test]
    fn baseline_is_constant_for_line() {
        let fonts = fonts();
        let style = TextStyle::new(16, 0xffffff, false);
        let base = fonts.baseline_y(6, &style);
        // Same (font, size) must always produce the same baseline for the same top.
        assert_eq!(fonts.baseline_y(6, &style), base);
        // Different top shifts by the same delta.
        assert_eq!(fonts.baseline_y(10, &style) - base, 4);
    }

    #[test]
    fn measure_draw_step_same() {
        let fonts = fonts();
        let mut fb = Framebuffer::headless();
        let style = TextStyle::new(13, 0xffffff, false);
        let text = "CPU0 100%";
        let advance_measure = fonts.measure(text, &style);
        let advance_draw = fonts.draw(&mut fb, text, 8, fonts.baseline_y(51, &style), &style);
        assert!((advance_measure - advance_draw).abs() < 0.01);
    }

    #[test]
    fn zero_width_text_no_dirty() {
        let fonts = fonts();
        let mut fb = Framebuffer::headless();
        let style = TextStyle::new(13, 0xffffff, false);
        fonts.draw(&mut fb, "", 8, fonts.baseline_y(51, &style), &style);
        assert!(fb.dirty_rects().is_empty());
    }

    /// Font recommendation table for the 480×320 Waveshare 3.5" panel (~165 PPI).
    /// Prints top-offset, box height, and recommended weight per size.
    #[test]
    fn print_font_recommendations() {
        let fonts = fonts();
        let weights = [("ExtraLight", false), ("Light", false), ("Regular", false),
                       ("Medium", true), ("SemiBold", false), ("Bold", false)];
        println!("\n=== 480×320 panel font recommendations (MapleMono NF CN) ===");
        println!("{:<6} {:<12} {:<12} {:<12} {:<12} {:<16} {:<50}",
            "size", "top_offset", "box_h", "'m' adv", "'m' open", "recommended wt", "note");
        for size in [10u16, 11, 13, 16] {
            // Evaluate 'm' stroke openings for every weight first.
            let mut best = ("", 0i32, 255u8);
            for (wt, _current) in &weights {
                let path = format!(
                    "/home/richli/MapleMono-NF-CN-unhinted/MapleMono-NF-CN-{}.ttf", wt);
                if let Ok(bytes) = std::fs::read(&path) {
                    if let Ok(font) = Font::from_bytes(bytes, FontSettings::default()) {
                        let idx = font.lookup_glyph_index('m');
                        let (metrics, data) = font.rasterize_indexed(idx, size as f32);
                        let mut open_cols = 0i32;
                        for col in 0..metrics.width {
                            let mut transparent = true;
                            for row in 0..metrics.height {
                                let a = data[row * metrics.width + col];
                                if a > 30 { transparent = false; break; }
                            }
                            if transparent { open_cols += 1; }
                        }
                        // Lower intensity = thinner, less blur. Tie-break by weight order.
                        let avg = if data.is_empty() {
                            255u8
                        } else {
                            (data.iter().map(|&v| v as u32).sum::<u32>() / data.len() as u32) as u8
                        };
                        if open_cols >= best.1 && avg < best.2 {
                            best = (*wt, open_cols, avg);
                        }
                    }
                }
            }

            let (recommended, note): (&str, &str) = match size {
                10 => ("Regular", "minimum readable; Medium blurs 'm', Light too faint"),
                11 => ("Regular", "readable, still compact; Medium acceptable"),
                13 => ("Medium", "current default; good balance"),
                16 => ("Medium", "headers, plenty of pixels"),
                _ => (best.0, "auto"),
            };
            let weight = if recommended == "Regular" {
                FontWeight::Regular
            } else {
                FontWeight::Medium
            };
            let style = TextStyle { size, color: 0xffffff, mono: false, weight };
            let top_offset = fonts.top_offset(&style);
            // Use the representative set's tallest descender ('g') for box height.
            let g = fonts.glyph('g', &style);
            let box_h = top_offset + 1 + g.ymin.abs();
            let m_adv = fonts.measure("m", &style);

            println!("{:<6} {:<12} {:<12} {:<12.2} {:<12} {:<16} {:<50}",
                size, top_offset, box_h, m_adv, best.1, recommended, note);
        }
    }

}

//! 2D drawing primitives.
//!
//! All drawing operates on the `Framebuffer` shadow buffer and marks dirty
//! regions. RGB888 constants from `config.rs` are converted to RGB565 on write.
//! Text rendering lives in `text.rs`.

use crate::config::{H, W};
use crate::fb::{Framebuffer, Rect};

/// Convert 0xRRGGBB to RGB565 little-endian.
pub fn rgb888_to_rgb565(c: u32) -> u16 {
    let r = ((c >> 16) & 0xff) as u16;
    let g = ((c >> 8) & 0xff) as u16;
    let b = (c & 0xff) as u16;
    ((r & 0xf8) << 8) | ((g & 0xfc) << 3) | (b >> 3)
}

/// Convert RGB565 to 0xRRGGBB.
pub fn rgb565_to_rgb888(c: u16) -> u32 {
    let r5 = (c >> 11) & 0x1f;
    let g6 = (c >> 5) & 0x3f;
    let b5 = c & 0x1f;
    let r = (r5 << 3) | (r5 >> 2);
    let g = (g6 << 2) | (g6 >> 4);
    let b = (b5 << 3) | (b5 >> 2);
    ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}

/// Alpha blend `fg` (0xRRGGBB) over `bg` (0xRRGGBB), return 0xRRGGBB.
pub fn blend_rgb888(bg: u32, fg: u32, alpha: u8) -> u32 {
    if alpha == 0 {
        return bg;
    }
    if alpha == 255 {
        return fg;
    }
    let a = alpha as u32;
    let inv = 255 - a;
    let r = (((bg >> 16) & 0xff) * inv + ((fg >> 16) & 0xff) * a) / 255;
    let g = (((bg >> 8) & 0xff) * inv + ((fg >> 8) & 0xff) * a) / 255;
    let b = ((bg & 0xff) * inv + (fg & 0xff) * a) / 255;
    (r << 16) | (g << 8) | b
}

/// Alpha blend `fg` over the RGB565 pixel `dst`, returning RGB565.
pub fn blend_over_rgb565(dst: u16, fg: u32, alpha: u8) -> u16 {
    if alpha == 0 {
        return dst;
    }
    let bg = rgb565_to_rgb888(dst);
    rgb888_to_rgb565(blend_rgb888(bg, fg, alpha))
}

/// Draw a filled rectangle in RGB888 color. Only changed rows are marked dirty.
pub fn fill_rect(fb: &mut Framebuffer, x: i32, y: i32, w: i32, h: i32, color: u32) {
    if w <= 0 || h <= 0 {
        return;
    }
    let x1 = x.max(0) as usize;
    let y1 = y.max(0) as usize;
    let x2 = (x + w).min(W as i32) as usize;
    let y2 = (y + h).min(H as i32) as usize;
    if x1 >= x2 || y1 >= y2 {
        return;
    }
    let rgb565 = rgb888_to_rgb565(color);
    let mut changed = false;
    let mut dirty_x1 = x2;
    let mut dirty_x2 = x1;
    let mut dirty_y1 = y2;
    let mut dirty_y2 = y1;

    for row in y1..y2 {
        let start = row * W + x1;
        let slice = &mut fb.buffer_mut()[start..start + (x2 - x1)];
        // Fast path: if the whole row is already the target colour, skip.
        if slice.iter().all(|&p| p == rgb565) {
            continue;
        }
        changed = true;
        dirty_y1 = dirty_y1.min(row);
        dirty_y2 = dirty_y2.max(row + 1);
        for (col, pixel) in slice.iter_mut().enumerate() {
            let gx = x1 + col;
            if *pixel != rgb565 {
                *pixel = rgb565;
                dirty_x1 = dirty_x1.min(gx);
                dirty_x2 = dirty_x2.max(gx + 1);
            }
        }
    }

    if changed {
        fb.mark_dirty(Rect::new(dirty_x1, dirty_y1, dirty_x2, dirty_y2));
    }
}

/// Bresenham line. Only changed pixels are marked dirty.
pub fn draw_line(fb: &mut Framebuffer, x1: i32, y1: i32, x2: i32, y2: i32, color: u32) {
    if x1 == x2 && y1 == y2 {
        return;
    }
    let rgb565 = rgb888_to_rgb565(color);
    let dx = (x2 - x1).abs();
    let dy = (y2 - y1).abs();
    let sx = if x1 < x2 { 1 } else { -1 };
    let sy = if y1 < y2 { 1 } else { -1 };
    let mut err = dx - dy;
    let mut x = x1;
    let mut y = y1;

    let dirty_x1 = x1.min(x2).max(0) as usize;
    let dirty_y1 = y1.min(y2).max(0) as usize;
    let mut dirty_x2 = x1.max(x2).max(0) as usize + 1;
    let mut dirty_y2 = y1.max(y2).max(0) as usize + 1;
    let mut changed = false;

    loop {
        if x >= 0 && x < W as i32 && y >= 0 && y < H as i32 {
            let idx = y as usize * W + x as usize;
            if fb.buffer_mut()[idx] != rgb565 {
                fb.buffer_mut()[idx] = rgb565;
                changed = true;
            }
        }
        if x == x2 && y == y2 {
            break;
        }
        let e2 = 2 * err;
        if e2 > -dy {
            err -= dy;
            x += sx;
        }
        if e2 < dx {
            err += dx;
            y += sy;
        }
    }

    if changed {
        dirty_x2 = dirty_x2.min(W);
        dirty_y2 = dirty_y2.min(H);
        if dirty_x1 < dirty_x2 && dirty_y1 < dirty_y2 {
            fb.mark_dirty(Rect::new(dirty_x1, dirty_y1, dirty_x2, dirty_y2));
        }
    }
}

/// Draw a horizontal line.
pub fn draw_line_h(fb: &mut Framebuffer, x1: i32, x2: i32, y: i32, color: u32) {
    if y < 0 || y >= H as i32 {
        return;
    }
    let x1c = x1.max(0) as usize;
    let x2c = (x2.min(W as i32) as usize).max(x1c);
    let yc = y as usize;
    let rgb565 = rgb888_to_rgb565(color);
    let start = yc * W + x1c;
    let slice = &mut fb.buffer_mut()[start..start + (x2c - x1c)];
    if slice.iter().all(|&p| p == rgb565) {
        return;
    }
    slice.fill(rgb565);
    fb.mark_dirty(Rect::new(x1c, yc, x2c, yc + 1));
}

/// Draw a filled ellipse (circle) at `(cx,cy)` with radius `r`.
pub fn fill_ellipse(fb: &mut Framebuffer, cx: i32, cy: i32, rx: i32, ry: i32, color: u32) {
    if rx <= 0 || ry <= 0 {
        return;
    }
    let rgb565 = rgb888_to_rgb565(color);
    let x1 = (cx - rx).max(0) as usize;
    let y1 = (cy - ry).max(0) as usize;
    let x2 = (cx + rx + 1).min(W as i32) as usize;
    let y2 = (cy + ry + 1).min(H as i32) as usize;
    let rx2 = (rx * rx) as f32;
    let ry2 = (ry * ry) as f32;
    let mut changed = false;
    for y in y1..y2 {
        let dy = (y as i32 - cy) as f32;
        let dx_max = (rx2 * (1.0 - dy * dy / ry2)).max(0.0).sqrt();
        let xl = ((cx as f32 - dx_max).ceil() as i32).max(0) as usize;
        let xr = ((cx as f32 + dx_max).floor() as i32 + 1).min(W as i32) as usize;
        let start = y * W + xl;
        let slice = &mut fb.buffer_mut()[start..start + (xr - xl)];
        if !slice.iter().all(|&p| p == rgb565) {
            slice.fill(rgb565);
            changed = true;
        }
    }
    if changed {
        fb.mark_dirty(Rect::new(x1, y1, x2, y2));
    }
}

/// Draw a filled rounded rectangle with corner radius `r`.
/// `r` is clamped to `min(w, h) / 2`. Only changed rows are marked dirty.
pub fn fill_rounded_rect(fb: &mut Framebuffer, x: i32, y: i32, w: i32, h: i32, r: i32, color: u32) {
    if w <= 0 || h <= 0 {
        return;
    }
    let r = r.min(w / 2).min(h / 2).max(0);
    let rgb565 = rgb888_to_rgb565(color);
    let x1 = x.max(0) as usize;
    let y1 = y.max(0) as usize;
    let x2 = (x + w).min(W as i32).max(x1 as i32) as usize;
    let y2 = (y + h).min(H as i32).max(y1 as i32) as usize;
    let r2 = (r * r) as f32;
    let mut changed = false;

    for row in y1..y2 {
        let dy = if row < (y + r) as usize {
            (y + r) as i32 - row as i32
        } else if row >= (y + h - r) as usize {
            row as i32 - (y + h - 1 - r) as i32
        } else {
            0
        };
        let inset = if dy > 0 && dy <= r {
            let dx = (r2 - (dy as f32).powi(2)).max(0.0).sqrt();
            (r as f32 - dx).ceil() as i32
        } else {
            0
        };
        let xl = (x + inset).max(0) as usize;
        let xr = (x + w - inset).min(W as i32).max(xl as i32) as usize;
        if xl >= xr {
            continue;
        }
        let start = row * W + xl;
        let slice = &mut fb.buffer_mut()[start..start + (xr - xl)];
        if !slice.iter().all(|&p| p == rgb565) {
            slice.fill(rgb565);
            changed = true;
        }
    }
    if changed {
        fb.mark_dirty(Rect::new(x1, y1, x2, y2));
    }
}

/// Draw a filled isosceles triangle. `up` selects whether the apex points up.
/// The bounding box is centred at `(cx, cy)` with width `w` and height `h`.
pub fn fill_triangle(fb: &mut Framebuffer, cx: i32, cy: i32, w: i32, h: i32, up: bool, color: u32) {
    if w <= 0 || h <= 0 {
        return;
    }
    let rgb565 = rgb888_to_rgb565(color);
    let y_top = cy - h / 2;
    let y_bottom = y_top + h;
    let y1 = y_top.max(0) as usize;
    let y2 = y_bottom.min(H as i32).max(y_top) as usize;
    let half_w = w as f32 / 2.0;
    let half_h = h as f32 / 2.0;
    let mut changed = false;

    for row in y1..y2 {
        let dy = if up {
            row as i32 - y_top
        } else {
            y_bottom - 1 - row as i32
        } as f32;
        let ratio = (dy / half_h).clamp(0.0, 1.0);
        let half = half_w * ratio;
        let xl = ((cx as f32 - half).ceil() as i32).max(0) as usize;
        let xr = ((cx as f32 + half).floor() as i32 + 1).min(W as i32).max(xl as i32) as usize;
        if xl >= xr {
            continue;
        }
        let start = row * W + xl;
        let slice = &mut fb.buffer_mut()[start..start + (xr - xl)];
        if !slice.iter().all(|&p| p == rgb565) {
            slice.fill(rgb565);
            changed = true;
        }
    }
    if changed {
        let x1 = (cx - w / 2).max(0) as usize;
        let x2 = (cx + w / 2 + 1).min(W as i32).max(x1 as i32) as usize;
        fb.mark_dirty(Rect::new(x1, y1, x2, y2));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> Framebuffer {
        Framebuffer::headless()
    }

    #[test]
    fn bresenham_line_marks_dirty() {
        let mut fb = setup();
        draw_line(&mut fb, 10, 10, 30, 20, 0xffffff);
        let r = fb.dirty_rects();
        assert!(!r.is_empty());
        let union = r.iter().fold(r[0], |a, b| a.union(b));
        assert!(union.width() >= 20);
        assert!(union.height() >= 10);
    }

    #[test]
    fn bresenham_line_zero_length_no_dirty() {
        let mut fb = setup();
        draw_line(&mut fb, 10, 10, 10, 10, 0xffffff);
        assert!(fb.dirty_rects().is_empty());
    }

    #[test]
    fn rounded_rect_marks_dirty_bbox() {
        let mut fb = setup();
        fill_rounded_rect(&mut fb, 10, 10, 20, 20, 4, 0xffffff);
        let r = fb.dirty_rects();
        assert!(!r.is_empty());
        let union = r.iter().fold(r[0], |a, b| a.union(b));
        assert!(union.width() >= 20);
        assert!(union.height() >= 20);
    }

    #[test]
    fn rounded_rect_zero_size_no_dirty() {
        let mut fb = setup();
        fill_rounded_rect(&mut fb, 10, 10, 0, 20, 4, 0xffffff);
        assert!(fb.dirty_rects().is_empty());
    }

    #[test]
    fn triangle_marks_dirty_bbox() {
        let mut fb = setup();
        fill_triangle(&mut fb, 50, 50, 8, 6, true, 0xffffff);
        let r = fb.dirty_rects();
        assert!(!r.is_empty());
        let union = r.iter().fold(r[0], |a, b| a.union(b));
        assert!(union.width() >= 8);
        assert!(union.height() >= 6);
    }

    #[test]
    fn triangle_zero_size_no_dirty() {
        let mut fb = setup();
        fill_triangle(&mut fb, 50, 50, 0, 6, true, 0xffffff);
        assert!(fb.dirty_rects().is_empty());
    }

    #[test]
    fn triangle_direction_up_narrows_at_top() {
        let mut fb = setup();
        // Up triangle: apex at top, base at bottom.
        fill_triangle(&mut fb, 50, 50, 10, 9, true, 0xffffff);
        let buf = fb.buffer();
        let white = rgb888_to_rgb565(0xffffff);
        let count_row = |y: usize| {
            let start = y * W;
            buf[start..start + W].iter().filter(|&&p| p == white).count()
        };
        let top = count_row(46);
        let bottom = count_row(54);
        assert!(top > 0, "up triangle top row must have pixels");
        assert!(top < bottom, "up triangle must be narrower at top ({} vs {})", top, bottom);
    }

    #[test]
    fn triangle_direction_down_narrows_at_bottom() {
        let mut fb = setup();
        // Down triangle: apex at bottom, base at top.
        fill_triangle(&mut fb, 50, 50, 10, 9, false, 0xffffff);
        let buf = fb.buffer();
        let white = rgb888_to_rgb565(0xffffff);
        let count_row = |y: usize| {
            let start = y * W;
            buf[start..start + W].iter().filter(|&&p| p == white).count()
        };
        let top = count_row(46);
        let bottom = count_row(54);
        assert!(bottom > 0, "down triangle bottom row must have pixels");
        assert!(bottom < top, "down triangle must be narrower at bottom ({} vs {})", bottom, top);
    }
}

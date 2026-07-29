//! RGB565 framebuffer → PNG → base64 for IPC screenshots.

use base64::Engine;

use crate::fb::Framebuffer;

/// Convert an RGB565 buffer to RGB888 bytes and encode as PNG.
/// The buffer is assumed to be width×height in row-major order.
pub fn encode_png(rgb565: &[u16], width: usize, height: usize) -> Option<Vec<u8>> {
    let mut rgb = vec![0u8; width * height * 3];
    for (i, &c) in rgb565.iter().enumerate() {
        let r5 = (c >> 11) & 0x1f;
        let g6 = (c >> 5) & 0x3f;
        let b5 = c & 0x1f;
        let r = ((r5 << 3) | (r5 >> 2)) as u8;
        let g = ((g6 << 2) | (g6 >> 4)) as u8;
        let b = ((b5 << 3) | (b5 >> 2)) as u8;
        rgb[i * 3] = r;
        rgb[i * 3 + 1] = g;
        rgb[i * 3 + 2] = b;
    }

    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width as u32, height as u32);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(&rgb).ok()?;
    }
    Some(out)
}

/// Lock the framebuffer just long enough to clone its shadow buffer, then
/// encode the clone to a base64 PNG string.
pub fn screenshot_base64(fb: &Framebuffer) -> Option<String> {
    let buf = fb.buffer().to_vec();
    let png = encode_png(&buf, crate::config::W, crate::config::H)?;
    Some(base64::engine::general_purpose::STANDARD.encode(png))
}

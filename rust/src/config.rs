//! Layout, color, timing and path constants.
//!
//! Mirrors `pi_dashboard/config.py`. Lookup tables for usage/temperature
//! gradients are built once at program start.

use std::sync::LazyLock;

pub const FB: &str = "/dev/fb1";
pub const W: usize = 480;
pub const H: usize = 320;

// ---------------------------------------------------------------------------
// GitHub Dark theme palette, stored as 0xRRGGBB.
// ---------------------------------------------------------------------------
pub const BG: u32 = 0x0d1117;
pub const PANEL: u32 = 0x161b22;
pub const ACCENT: u32 = 0x30363d;
pub const GREEN: u32 = 0x3fb950;
pub const RED: u32 = 0xf85149;
pub const YELLOW: u32 = 0xd29922;
pub const WHITE: u32 = 0xe6edf3;
pub const GRAY: u32 = 0x7d8590;
pub const CYAN: u32 = 0x39c5cf;
pub const ORANGE: u32 = 0xf0883e;
pub const BLUE: u32 = 0x58a6ff;
pub const OVERLAY_BG: u32 = 0x333333;

pub const USAGE_GRADIENT: &[(f32, u32)] = &[
    (0.0, 0x3fb950),
    (0.45, 0x7ee787),
    (0.65, 0xd29922),
    (0.85, 0xf0883e),
    (1.0, 0xf85149),
];

pub const TEMP_GRADIENT: &[(f32, u32)] = &[
    (0.0, 0x58a6ff),
    (0.25, 0x39c5cf),
    (0.45, 0x3fb950),
    (0.7, 0xd29922),
    (1.0, 0xf85149),
];

pub const TEMP_LO: f32 = 25.0;
pub const TEMP_HI: f32 = 90.0;

pub const FONT_PATHS: &[&str] = &[
    "/usr/local/share/fonts/maple-mono-nf-cn-unhinted/MapleMono-NF-CN-Medium-ASCII.ttf",
    "/usr/local/share/fonts/maple-mono-nf-cn-unhinted/MapleMono-NF-CN-Medium.ttf",
    "/usr/local/share/fonts/maple-mono-nf-cn-unhinted/MapleMono-NF-CN-Regular.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
];

pub const REGULAR_FONT_PATHS: &[&str] = &[
    "/usr/local/share/fonts/maple-mono-nf-cn-unhinted/MapleMono-NF-CN-Regular-ASCII.ttf",
    "/usr/local/share/fonts/maple-mono-nf-cn-unhinted/MapleMono-NF-CN-Regular.ttf",
    "/usr/local/share/fonts/maple-mono-nf-cn-unhinted/MapleMono-NF-CN-Medium-ASCII.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
];

pub const MONO_FONT_PATHS: &[&str] = &[
    "/usr/local/share/fonts/maple-mono-nf-cn-unhinted/MapleMono-NF-CN-Medium-ASCII.ttf",
    "/usr/local/share/fonts/maple-mono-nf-cn-unhinted/MapleMono-NF-CN-Medium.ttf",
    "/usr/local/share/fonts/maple-mono-nf-cn-unhinted/MapleMono-NF-CN-Regular.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
    "/usr/share/fonts/truetype/ubuntu/UbuntuMono-R.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
];

pub const REFRESH_INTERVAL_MS: u64 = 67; // ~15 FPS, within 48 MHz SPI headroom
pub const SLOW_RENDER_INTERVAL: f32 = 1.0;
pub const SLOW_DATA_INTERVAL: f32 = 2.0;
pub const CPU_SMOOTH_WINDOW: usize = 5;  // shorter smoothing at higher FPS; jumps + avg coexist
pub const BOOT_TIMEOUT: u64 = 30;

pub const MODE_NAMES: &[&str] = &["monitor"];

pub const TOUCH_POLL_INTERVAL_MS: u64 = 50;

pub const CONTAINER_PAGE_SIZE: usize = 10;
pub const DOCKER_START_Y: i32 = 108;
pub const DOCKER_HEADER_Y: i32 = DOCKER_START_Y;
pub const DOCKER_LIST_Y: i32 = DOCKER_START_Y + 18;
pub const DOCKER_LINE_HEIGHT: i32 = 16;

pub const TOUCH_DEVICES: &[&str] = &[
    "/dev/input/event1",
    "/dev/input/event0",
    "/dev/input/event2",
    "/dev/input/event3",
];

pub const IP_FILTER_ENABLED: bool = true;

pub const SOCKET_PATH: &str = "/var/lib/pi-dashboard/pi_dashboard.sock";

// ---------------------------------------------------------------------------
// Lookup tables, built lazily on first use.
// ---------------------------------------------------------------------------
pub static USAGE_COLOR_LUT: LazyLock<[u32; 101]> = LazyLock::new(|| {
    let mut lut = [0u32; 101];
    for (i, entry) in lut.iter_mut().enumerate() {
        *entry = gradient_color(i as f32 / 100.0, USAGE_GRADIENT);
    }
    lut
});

pub static TEMP_COLOR_LUT: LazyLock<[u32; 128]> = LazyLock::new(|| {
    let mut lut = [0u32; 128];
    let range = TEMP_HI - TEMP_LO;
    for (t, entry) in lut.iter_mut().enumerate() {
        let ratio = if range <= 0.0 {
            0.0
        } else {
            ((t as f32 - TEMP_LO) / range).clamp(0.0, 1.0)
        };
        *entry = gradient_color(ratio, TEMP_GRADIENT);
    }
    lut
});

/// Linear interpolation between gradient stops. Stops are monotonically
/// increasing in `0.0..=1.0` and cover both endpoints.
pub fn gradient_color(ratio: f32, stops: &[(f32, u32)]) -> u32 {
    let ratio = ratio.clamp(0.0, 1.0);
    let (mut prev_pos, mut prev_hex) = stops.first().copied().unwrap_or((0.0, 0xffffff));
    for &(pos, hex) in stops.iter().skip(1) {
        if ratio <= pos {
            if pos <= prev_pos {
                return hex;
            }
            let t = (ratio - prev_pos) / (pos - prev_pos);
            let r0 = ((prev_hex >> 16) & 0xff) as f32;
            let g0 = ((prev_hex >> 8) & 0xff) as f32;
            let b0 = (prev_hex & 0xff) as f32;
            let r1 = ((hex >> 16) & 0xff) as f32;
            let g1 = ((hex >> 8) & 0xff) as f32;
            let b1 = (hex & 0xff) as f32;
            let r = (r0 + (r1 - r0) * t) as u32;
            let g = (g0 + (g1 - g0) * t) as u32;
            let b = (b0 + (b1 - b0) * t) as u32;
            return (r << 16) | (g << 8) | b;
        }
        prev_pos = pos;
        prev_hex = hex;
    }
    stops.last().map(|s| s.1).unwrap_or(0xffffff)
}

/// Parse a percentage string such as "45%" or "1234/4096MB (50%)".
pub fn parse_percent(text: &str) -> Option<i32> {
    // Take the last occurrence of a number followed by '%'.
    let mut last = None;
    for m in text.split_whitespace() {
        if let Some(stripped) = m.strip_suffix('%') {
            if let Ok(v) = stripped.parse::<f32>() {
                last = Some(v as i32);
            }
        }
    }
    last
}

/// Parse a temperature string such as "42C" or "42.5C".
pub fn parse_temp(text: &str) -> Option<i32> {
    text.split(|c: char| !c.is_ascii_digit() && c != '.')
        .next()
        .and_then(|s| s.parse::<f32>().ok())
        .map(|v| v as i32)
}

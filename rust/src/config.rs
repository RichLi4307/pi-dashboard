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

// ---------------------------------------------------------------------------
// Semantic colour aliases used by the monitor page.
// ---------------------------------------------------------------------------
pub const OK: u32 = GREEN;              // 0x3fb950
pub const CAUTION: u32 = YELLOW;        // 0xd29922
pub const ALARM: u32 = 0xff0000;        // user-specified alarm red
pub const COOL: u32 = BLUE;             // 0x58a6ff
pub const TREND_HOT: u32 = RED;         // 0xf85149, rising-temperature arrow
pub const ROW_STRIPE: u32 = 0x131a24;   // zebra stripe between BG and PANEL
pub const SCROLL_TRACK: u32 = 0x21262e; // container scroll track

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
pub const SLOW_DATA_INTERVAL: f32 = 5.0;
pub const CPU_SMOOTH_WINDOW: usize = 5;  // shorter smoothing at higher FPS; jumps + avg coexist
pub const BOOT_TIMEOUT: u64 = 30;

pub const MODE_NAMES: &[&str] = &["monitor"];

pub const TOUCH_POLL_INTERVAL_MS: u64 = 50;

pub const CONTAINER_PAGE_SIZE: usize = 8;
pub const DOCKER_START_Y: i32 = 156;
pub const DOCKER_HEADER_Y: i32 = DOCKER_START_Y;
pub const DOCKER_LIST_Y: i32 = 176;
pub const DOCKER_LINE_HEIGHT: i32 = 14;

pub const TOUCH_DEVICES: &[&str] = &[
    "/dev/input/event1",
    "/dev/input/event0",
    "/dev/input/event2",
    "/dev/input/event3",
];

pub const IP_FILTER_ENABLED: bool = true;

pub const SOCKET_PATH: &str = "/var/lib/pi-dashboard/pi_dashboard.sock";

// ---------------------------------------------------------------------------
// Hostname, resolved once at startup.
// Priority: PI_DASHBOARD_HOSTNAME env → /etc/hostname → "pi"
// ---------------------------------------------------------------------------
pub static HOSTNAME: LazyLock<String> = LazyLock::new(|| {
    if let Ok(v) = std::env::var("PI_DASHBOARD_HOSTNAME") {
        if !v.is_empty() {
            return v.trim().to_string();
        }
    }
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "pi".to_string())
});

/// Return the cached hostname.
pub fn hostname() -> &'static str {
    &HOSTNAME
}

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
    // Take the last occurrence of a number followed by '%', allowing
    // surrounding punctuation like parentheses or units.
    let mut last = None;
    for token in text.split(|c: char| !c.is_ascii_digit() && c != '.' && c != '%') {
        if let Some(stripped) = token.strip_suffix('%') {
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

/// Hard temperature band colour used for the temperature value.
pub fn temp_band_color(t: i32) -> u32 {
    if t < 50 {
        COOL
    } else if t < 65 {
        OK
    } else if t < 75 {
        CAUTION
    } else if t < 80 {
        ORANGE
    } else {
        ALARM
    }
}

/// Usage percentage colour for numeric text labels (CPU/MEM/DISK/container CPU).
pub fn usage_text_color(pct: i32) -> u32 {
    if pct < 80 {
        OK
    } else if pct < 90 {
        CAUTION
    } else {
        ALARM
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_percent_last() {
        assert_eq!(parse_percent("45%"), Some(45));
        assert_eq!(parse_percent("1234/4096MB (50%)"), Some(50));
    }

    #[test]
    fn parse_temp_int() {
        assert_eq!(parse_temp("42C"), Some(42));
        assert_eq!(parse_temp("42.5C"), Some(42));
    }

    #[test]
    fn temp_band_color_boundaries() {
        assert_eq!(temp_band_color(49), COOL);
        assert_eq!(temp_band_color(50), OK);
        assert_eq!(temp_band_color(64), OK);
        assert_eq!(temp_band_color(65), CAUTION);
        assert_eq!(temp_band_color(74), CAUTION);
        assert_eq!(temp_band_color(75), ORANGE);
        assert_eq!(temp_band_color(79), ORANGE);
        assert_eq!(temp_band_color(80), ALARM);
        assert_eq!(temp_band_color(85), ALARM);
    }

    #[test]
    fn usage_text_color_boundaries() {
        assert_eq!(usage_text_color(0), OK);
        assert_eq!(usage_text_color(79), OK);
        assert_eq!(usage_text_color(80), CAUTION);
        assert_eq!(usage_text_color(89), CAUTION);
        assert_eq!(usage_text_color(90), ALARM);
        assert_eq!(usage_text_color(100), ALARM);
    }

    #[test]
    fn hostname_fallback_chain() {
        let h = hostname();
        assert!(!h.is_empty());
        // Should not be the fallback literal unless /etc/hostname is missing/empty.
        if std::path::Path::new("/etc/hostname").exists() {
            assert!(
                h != "pi" || std::fs::read_to_string("/etc/hostname")
                    .map(|s| s.trim().is_empty())
                    .unwrap_or(true),
                "hostname should come from /etc/hostname when present"
            );
        }
    }
}

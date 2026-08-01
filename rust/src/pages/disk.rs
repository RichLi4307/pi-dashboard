//! Disk detail page.

use anyhow::Result;
use std::fs;

use crate::chart::{LineChart, RangeMode, Series};
use crate::config::{CYAN, ORANGE};
use crate::fb::Framebuffer;
use crate::metrics::{fmt_rate, MetricsSnapshot};
use crate::render::fill_rect;
use crate::pages::detail_common::{
    draw_aux_text, draw_big_value, draw_static_background, draw_title, BackButton, InfoRows,
    CHART_H, CHART_W, CHART_X, CHART_Y,
};
use crate::pages::{Page, PageAction};
use crate::text::Fonts;
use crate::touch::TouchEvent;

pub struct DiskPage {
    fonts: Fonts,
    bg_done: bool,
    back: BackButton,
    chart: LineChart,
    info: InfoRows,
    last_pct: String,
    last_size: String,
}

impl DiskPage {
    pub fn new(fonts: Fonts) -> Self {
        Self {
            fonts: fonts.clone(),
            bg_done: false,
            back: BackButton::new(),
            chart: LineChart::new(CHART_X, CHART_Y, CHART_W, CHART_H, RangeMode::Auto),
            info: InfoRows::new(fonts, 4),
            last_pct: String::new(),
            last_size: String::new(),
        }
    }

    fn disk_usage() -> (String, String, String) {
        let mut total_b = 0u64;
        let mut free_b = 0u64;
        unsafe {
            let mut buf: libc::statvfs = std::mem::zeroed();
            if libc::statvfs(b"/\0".as_ptr() as *const _, &mut buf) == 0 {
                total_b = buf.f_blocks * buf.f_frsize;
                free_b = buf.f_bavail * buf.f_frsize;
            }
        }
        let used_b = total_b.saturating_sub(free_b);
        let pct = if total_b > 0 {
            100.0 * used_b as f32 / total_b as f32
        } else {
            0.0
        };
        (
            format!("{:.0}%", pct),
            fmt_bytes(used_b),
            fmt_bytes(total_b),
        )
    }

    fn disk_info() -> (String, String) {
        let text = fs::read_to_string("/proc/mounts").unwrap_or_default();
        let mut fs_type = "ext4".to_string();
        let mut device = "mmcblk0".to_string();
        for line in text.lines() {
            let cols: Vec<&str> = line.split_whitespace().collect();
            if cols.len() >= 3 && cols[1] == "/" {
                device = cols[0].trim_start_matches("/dev/").to_string();
                fs_type = cols[2].to_string();
                break;
            }
        }
        (device, fs_type)
    }
}

fn fmt_bytes(b: u64) -> String {
    if b >= 1024 * 1024 * 1024 {
        format!("{:.1}G", b as f32 / (1024.0 * 1024.0 * 1024.0))
    } else if b >= 1024 * 1024 {
        format!("{:.1}M", b as f32 / (1024.0 * 1024.0))
    } else if b >= 1024 {
        format!("{:.0}K", b as f32 / 1024.0)
    } else {
        format!("{}B", b)
    }
}

impl Page for DiskPage {
    fn id(&self) -> &'static str {
        "disk"
    }

    fn render(&mut self, fb: &mut Framebuffer, snapshot: &MetricsSnapshot) -> Result<()> {
        if !self.bg_done {
            draw_static_background(fb, &self.fonts);
            draw_title(fb, &self.fonts, "DISK");
            self.chart.force_draw(fb);
            let (device, fs_type) = Self::disk_info();
            self.info.force_draw(fb, 0, ("Mount", "/"), ("FS", &fs_type));
            self.info.force_draw(fb, 1, ("Device", &device), ("", ""));
            self.bg_done = true;
            fb.mark_full_dirty();
        }

        let (pct_str, used, total) = Self::disk_usage();
        if pct_str != self.last_pct {
            // Approximate big value color: reuse config usage color.
            let pct = crate::config::parse_percent(&pct_str).unwrap_or(0);
            let color = crate::config::usage_text_color(pct);
            fill_rect(fb, 0, 36, 240, 28, crate::config::BG);
            draw_big_value(fb, &self.fonts, &pct_str, color);
            self.last_pct = pct_str;
            fb.mark_dirty(crate::fb::Rect::new(0, 36, 240, 64));
        }

        let size_str = format!("{} / {}", used, total);
        if size_str != self.last_size {
            fill_rect(fb, 240, 36, 240, 28, crate::config::BG);
            draw_aux_text(fb, &self.fonts, &size_str);
            self.last_size = size_str;
            fb.mark_dirty(crate::fb::Rect::new(240, 36, 480, 64));
        }

        let series = vec![
            Series {
                data: snapshot.history.disk_read.clone(),
                color: CYAN,
            },
            Series {
                data: snapshot.history.disk_write.clone(),
                color: ORANGE,
            },
        ];
        self.chart.set(fb, &series);

        self.info.set(
            fb,
            2,
            ("Read", &fmt_rate(snapshot.io.disk_read)),
            ("Write", &fmt_rate(snapshot.io.disk_write)),
        );

        Ok(())
    }

    fn on_touch(&mut self, ev: TouchEvent) -> PageAction {
        if self.back.hit(&ev) {
            return PageAction::Switch("monitor");
        }
        PageAction::None
    }

    fn on_enter(&mut self, fb: &mut Framebuffer) {
        self.bg_done = false;
        self.last_pct.clear();
        self.last_size.clear();
        fb.mark_full_dirty();
    }
}

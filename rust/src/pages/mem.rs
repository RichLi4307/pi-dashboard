//! Memory detail page.

use anyhow::Result;
use std::collections::HashMap;

use crate::chart::{LineChart, RangeMode, Series};
use crate::config::{BG, OK};
use crate::fb::Framebuffer;
use crate::metrics::MetricsSnapshot;
use crate::render::fill_rect;
use crate::pages::detail_common::{
    draw_aux_text, draw_big_value, draw_static_background, draw_title, BackButton, InfoRows,
    CHART_H, CHART_W, CHART_X, CHART_Y,
};
use crate::pages::{Page, PageAction};
use crate::text::Fonts;
use crate::touch::TouchEvent;

pub struct MemPage {
    fonts: Fonts,
    bg_done: bool,
    back: BackButton,
    chart: LineChart,
    info: InfoRows,
    last_pct: String,
    last_used: String,
}

impl MemPage {
    pub fn new(fonts: Fonts) -> Self {
        Self {
            fonts: fonts.clone(),
            bg_done: false,
            back: BackButton::new(),
            chart: LineChart::new(CHART_X, CHART_Y, CHART_W, CHART_H, RangeMode::Fixed { min: 0.0, max: 100.0 }),
            info: InfoRows::new(fonts, 4),
            last_pct: String::new(),
            last_used: String::new(),
        }
    }

    fn read_meminfo() -> HashMap<String, u64> {
        let text = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
        let mut map = HashMap::new();
        for line in text.lines() {
            let Some((k, v)) = line.split_once(':') else { continue };
            let val = v
                .trim()
                .split_whitespace()
                .next()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);
            map.insert(k.trim().to_string(), val);
        }
        map
    }

    fn fmt_kb(kb: u64) -> String {
        if kb >= 1024 * 1024 {
            format!("{:.1}G", kb as f32 / (1024.0 * 1024.0))
        } else if kb >= 1024 {
            format!("{:.0}M", kb as f32 / 1024.0)
        } else {
            format!("{}K", kb)
        }
    }

    fn mem_pct(snapshot: &MetricsSnapshot) -> f32 {
        snapshot
            .history
            .mem_pct
            .last()
            .copied()
            .unwrap_or_else(|| crate::config::parse_percent(&snapshot.mem).unwrap_or(0) as f32)
    }
}

impl Page for MemPage {
    fn id(&self) -> &'static str {
        "mem"
    }

    fn render(&mut self, fb: &mut Framebuffer, snapshot: &MetricsSnapshot) -> Result<()> {
        if !self.bg_done {
            draw_static_background(fb, &self.fonts);
            draw_title(fb, &self.fonts, "MEMORY");
            self.chart.force_draw(fb);
            self.bg_done = true;
            fb.mark_full_dirty();
        }

        let pct = Self::mem_pct(snapshot);
        let pct_str = format!("{:.0}%", pct);
        if pct_str != self.last_pct {
            fill_rect(fb, 0, 36, 240, 28, BG);
            draw_big_value(fb, &self.fonts, &pct_str, OK);
            self.last_pct = pct_str;
            fb.mark_dirty(crate::fb::Rect::new(0, 36, 240, 64));
        }

        let mem = Self::read_meminfo();
        let total = mem.get("MemTotal").copied().unwrap_or(0);
        let available = mem.get("MemAvailable").copied().unwrap_or(0);
        let used = total.saturating_sub(available);
        let used_str = format!("{} / {}", Self::fmt_kb(used), Self::fmt_kb(total));
        if used_str != self.last_used {
            fill_rect(fb, 240, 36, 240, 28, BG);
            draw_aux_text(fb, &self.fonts, &used_str);
            self.last_used = used_str;
            fb.mark_dirty(crate::fb::Rect::new(240, 36, 480, 64));
        }

        let series = vec![Series {
            data: snapshot.history.mem_pct.clone(),
            color: OK,
        }];
        self.chart.set(fb, &series);

        let buffers = mem.get("Buffers").copied().unwrap_or(0);
        let cached = mem.get("Cached").copied().unwrap_or(0);
        let swap_total = mem.get("SwapTotal").copied().unwrap_or(0);
        let swap_free = mem.get("SwapFree").copied().unwrap_or(0);
        let swap_used = swap_total.saturating_sub(swap_free);
        self.info.set(
            fb,
            0,
            ("Total", &Self::fmt_kb(total)),
            ("Used", &Self::fmt_kb(used)),
        );
        self.info.set(
            fb,
            1,
            ("Available", &Self::fmt_kb(available)),
            ("Buffers", &Self::fmt_kb(buffers)),
        );
        self.info.set(
            fb,
            2,
            ("Cached", &Self::fmt_kb(cached)),
            ("Swap", &Self::fmt_kb(swap_used)),
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
        self.last_used.clear();
        fb.mark_full_dirty();
    }
}

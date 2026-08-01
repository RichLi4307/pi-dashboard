//! CPU detail page.

use anyhow::Result;
use std::fs;

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

pub struct CpuPage {
    fonts: Fonts,
    bg_done: bool,
    back: BackButton,
    chart: LineChart,
    info: InfoRows,
    last_total: String,
    last_load: String,
    last_cores: String,
}

impl CpuPage {
    pub fn new(fonts: Fonts) -> Self {
        Self {
            fonts: fonts.clone(),
            bg_done: false,
            back: BackButton::new(),
            chart: LineChart::new(CHART_X, CHART_Y, CHART_W, CHART_H, RangeMode::Fixed { min: 0.0, max: 100.0 }),
            info: InfoRows::new(fonts, 4),
            last_total: String::new(),
            last_load: String::new(),
            last_cores: String::new(),
        }
    }

    fn total_cpu(snapshot: &MetricsSnapshot) -> f32 {
        snapshot
            .history
            .cpu_total
            .last()
            .copied()
            .unwrap_or_else(|| {
                snapshot
                    .cpu
                    .iter()
                    .map(|(_, v)| *v)
                    .sum::<f32>()
                    / snapshot.cpu.len().max(1) as f32
            })
    }

    fn loadavg() -> String {
        fs::read_to_string("/proc/loadavg")
            .ok()
            .and_then(|s| s.split_whitespace().take(3).map(|x| x.to_string()).collect::<Vec<_>>().join(" ").into())
            .unwrap_or_else(|| "n/a".to_string())
    }

    fn cpuinfo() -> (String, String, String, String) {
        let text = fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
        let mut hardware = "BCM2711".to_string();
        let mut model = "n/a".to_string();
        for line in text.lines() {
            if let Some(v) = line.strip_prefix("Hardware\t: ") {
                hardware = v.trim().to_string();
            }
            if let Some(v) = line.strip_prefix("model name\t: ") {
                model = v.trim().to_string();
            }
        }
        let governor = fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor")
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "n/a".to_string());
        let freq_khz = fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq")
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok())
            .unwrap_or(0);
        let freq = if freq_khz >= 1_000_000 {
            format!("{:.1}G", freq_khz as f32 / 1_000_000.0)
        } else if freq_khz > 0 {
            format!("{:.0}M", freq_khz as f32 / 1_000.0)
        } else {
            "n/a".to_string()
        };
        (hardware, model, governor, freq)
    }
}

impl Page for CpuPage {
    fn id(&self) -> &'static str {
        "cpu"
    }

    fn render(&mut self, fb: &mut Framebuffer, snapshot: &MetricsSnapshot) -> Result<()> {
        if !self.bg_done {
            draw_static_background(fb, &self.fonts);
            draw_title(fb, &self.fonts, "CPU");
            self.chart.force_draw(fb);
            let (hw, _model, gov, freq) = Self::cpuinfo();
            self.info.force_draw(fb, 0, ("Model", &hw), ("Cores", "4"));
            self.info.force_draw(fb, 1, ("Governor", &gov), ("Freq", &freq));
            self.bg_done = true;
            fb.mark_full_dirty();
        }

        let total = Self::total_cpu(snapshot);
        let total_str = format!("{:.0}%", total);
        if total_str != self.last_total {
            fill_rect(fb, 0, 36, 240, 28, BG);
            draw_big_value(fb, &self.fonts, &total_str, OK);
            self.last_total = total_str;
            fb.mark_dirty(crate::fb::Rect::new(0, 36, 240, 64));
        }

        let load = Self::loadavg();
        let load_str = format!("load {}", load);
        if load_str != self.last_load {
            fill_rect(fb, 240, 36, 240, 28, BG);
            draw_aux_text(fb, &self.fonts, &load_str);
            self.last_load = load_str;
            fb.mark_dirty(crate::fb::Rect::new(240, 36, 480, 64));
        }

        let series = vec![Series {
            data: snapshot.history.cpu_total.clone(),
            color: OK,
        }];
        self.chart.set(fb, &series);

        // Per-core current values (reuse snapshot.cpu which is smoothed per-core).
        let mut parts = Vec::new();
        for i in 0..4 {
            let pct = snapshot
                .cpu
                .iter()
                .find(|(name, _)| name == &format!("cpu{}", i))
                .map(|(_, v)| *v)
                .unwrap_or(0.0);
            parts.push(format!("C{} {:.0}", i, pct));
        }
        let cores_str = parts.join("  ");
        if cores_str != self.last_cores {
            self.info.set(fb, 2, ("Cores", &cores_str), ("", ""));
            self.last_cores = cores_str;
        }

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
        self.last_total.clear();
        self.last_load.clear();
        self.last_cores.clear();
        fb.mark_full_dirty();
    }
}

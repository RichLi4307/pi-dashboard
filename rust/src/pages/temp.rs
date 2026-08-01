//! Temperature detail page.

use anyhow::Result;

use crate::chart::{LineChart, RangeMode, Series};
use crate::config::{ALARM, BG, COOL, OK, TREND_HOT};
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

const HISTORY_MAX_MIN_SAMPLES: usize = 4;

pub struct TempPage {
    fonts: Fonts,
    bg_done: bool,
    back: BackButton,
    chart: LineChart,
    info: InfoRows,
    last_temp: String,
    last_aux: String,
    max_temp: f32,
    min_temp: f32,
    max_lifetime: f32,
}

impl TempPage {
    pub fn new(fonts: Fonts) -> Self {
        let mut chart = LineChart::new(CHART_X, CHART_Y, CHART_W, CHART_H, RangeMode::Fixed { min: 20.0, max: 90.0 });
        chart.set_threshold(Some((80.0, ALARM)));
        Self {
            fonts: fonts.clone(),
            bg_done: false,
            back: BackButton::new(),
            chart,
            info: InfoRows::new(fonts, 4),
            last_temp: String::new(),
            last_aux: String::new(),
            max_temp: f32::MIN,
            min_temp: f32::MAX,
            max_lifetime: f32::MIN,
        }
    }

    fn current_temp(snapshot: &MetricsSnapshot) -> f32 {
        snapshot
            .history
            .temp
            .last()
            .copied()
            .unwrap_or(crate::config::parse_temp(&snapshot.temp).unwrap_or(0) as f32)
    }

    fn trend(snapshot: &MetricsSnapshot) -> (&'static str, f32, u32) {
        let h = &snapshot.history.temp;
        if h.len() < 2 {
            return ("steady", 0.0, crate::config::GRAY);
        }
        let cur = *h.last().unwrap();
        let idx = h.len().saturating_sub(31);
        let old = h[idx.min(h.len() - 1)];
        let delta = cur - old;
        if delta >= 1.0 {
            ("rising", delta, TREND_HOT)
        } else if delta <= -1.0 {
            ("falling", delta.abs(), COOL)
        } else {
            ("steady", 0.0, crate::config::GRAY)
        }
    }

    fn throttled() -> String {
        match std::process::Command::new("vcgencmd")
            .args(["get_throttled"])
            .output()
        {
            Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
                .trim()
                .split('=')
                .nth(1)
                .unwrap_or("n/a")
                .to_string(),
            _ => "n/a".to_string(),
        }
    }
}

impl Page for TempPage {
    fn id(&self) -> &'static str {
        "temp"
    }

    fn render(&mut self, fb: &mut Framebuffer, snapshot: &MetricsSnapshot) -> Result<()> {
        if !self.bg_done {
            draw_static_background(fb, &self.fonts);
            draw_title(fb, &self.fonts, "TEMPERATURE");
            self.chart.force_draw(fb);
            self.info.force_draw(fb, 0, ("Sensor", "thermal_zone0"), ("Throttled", &Self::throttled()));
            self.bg_done = true;
            fb.mark_full_dirty();
        }

        let temp = Self::current_temp(snapshot);
        self.max_temp = self.max_temp.max(temp);
        self.min_temp = self.min_temp.min(temp);
        self.max_lifetime = self.max_lifetime.max(temp);

        let temp_color = crate::config::temp_band_color(temp as i32);
        let temp_str = format!("{:.0}C", temp);
        if temp_str != self.last_temp {
            // Erase old big value by redrawing background strip.
            fill_rect(fb, 0, 36, 240, 28, BG);
            draw_big_value(fb, &self.fonts, &temp_str, temp_color);
            self.last_temp = temp_str;
            fb.mark_dirty(crate::fb::Rect::new(0, 36, 240, 64));
        }

        let (trend, delta, _trend_color) = Self::trend(snapshot);
        let delta_str = if trend == "steady" {
            "steady".to_string()
        } else {
            format!("{} {:.0}C", trend, delta)
        };
        let aux = format!(
            "max {:.0}C / min {:.0}C   {}",
            self.max_temp,
            self.min_temp,
            delta_str
        );
        if aux != self.last_aux {
            fill_rect(fb, 240, 36, 240, 28, BG);
            draw_aux_text(fb, &self.fonts, &aux);
            self.last_aux = aux;
            fb.mark_dirty(crate::fb::Rect::new(240, 36, 480, 64));
        }

        let series = vec![Series {
            data: snapshot.history.temp.clone(),
            color: if temp >= 80.0 { ALARM } else { OK },
        }];
        self.chart.set(fb, &series);

        // Trend + lifetime max in info rows (rows 1..3 update dynamically).
        let trend_text = format!("{}", delta_str);
        self.info.set(fb, 1, ("Trend", &trend_text), ("Max today", &format!("{:.0}C", self.max_lifetime)));

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
        self.last_temp.clear();
        self.last_aux.clear();
        self.max_temp = f32::MIN;
        self.min_temp = f32::MAX;
        fb.mark_full_dirty();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fb::Framebuffer;
    use crate::screenshot::encode_png;
    use crate::text::Fonts;

    #[test]
    fn headless_temp_golden_render() {
        let fonts = Fonts::load().expect("fonts should load");
        let mut page = TempPage::new(fonts);
        let mut fb = Framebuffer::headless();
        let snapshot = MetricsSnapshot {
            temp: "63C".into(),
            history: crate::metrics::HistorySnapshot {
                temp: vec![45.0, 46.0, 47.0, 48.0, 49.0, 50.0, 51.0, 52.0, 53.0, 54.0, 55.0, 56.0, 57.0, 58.0, 59.0, 60.0, 61.0, 62.0, 63.0],
                ..Default::default()
            },
            ..Default::default()
        };
        page.render(&mut fb, &snapshot).unwrap();
        page.render(&mut fb, &snapshot).unwrap();
        let png = encode_png(fb.buffer(), crate::config::W, crate::config::H).expect("png encode");
        assert!(!png.is_empty());
        let path = "/tmp/pi_dashboard_golden_temp.png";
        std::fs::write(path, png).expect("write png");
    }
}

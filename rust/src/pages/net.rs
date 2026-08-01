//! Network detail page.

use anyhow::Result;

use crate::chart::{LineChart, RangeMode, Series};
use crate::config::{BG, CYAN, OK};
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

pub struct NetPage {
    fonts: Fonts,
    bg_done: bool,
    back: BackButton,
    chart: LineChart,
    info: InfoRows,
    last_down: String,
    last_up: String,
}

impl NetPage {
    pub fn new(fonts: Fonts) -> Self {
        Self {
            fonts: fonts.clone(),
            bg_done: false,
            back: BackButton::new(),
            chart: LineChart::new(CHART_X, CHART_Y, CHART_W, CHART_H, RangeMode::Auto),
            info: InfoRows::new(fonts, 4),
            last_down: String::new(),
            last_up: String::new(),
        }
    }

}

impl Page for NetPage {
    fn id(&self) -> &'static str {
        "net"
    }

    fn render(&mut self, fb: &mut Framebuffer, snapshot: &MetricsSnapshot) -> Result<()> {
        if !self.bg_done {
            draw_static_background(fb, &self.fonts);
            draw_title(fb, &self.fonts, "NETWORK");
            self.chart.force_draw(fb);
            self.bg_done = true;
            fb.mark_full_dirty();
        }

        let down = snapshot.io.net_down;
        let up = snapshot.io.net_up;
        let down_str = fmt_rate(down);
        if down_str != self.last_down {
            fill_rect(fb, 0, 36, 240, 28, BG);
            draw_big_value(fb, &self.fonts, &down_str, CYAN);
            self.last_down = down_str;
            fb.mark_dirty(crate::fb::Rect::new(0, 36, 240, 64));
        }

        let up_str = format!("up {}/s", fmt_rate(up));
        if up_str != self.last_up {
            fill_rect(fb, 240, 36, 240, 28, BG);
            draw_aux_text(fb, &self.fonts, &up_str);
            self.last_up = up_str;
            fb.mark_dirty(crate::fb::Rect::new(240, 36, 480, 64));
        }

        let series = vec![
            Series {
                data: snapshot.history.net_down.clone(),
                color: CYAN,
            },
            Series {
                data: snapshot.history.net_up.clone(),
                color: OK,
            },
        ];
        self.chart.set(fb, &series);

        // Info rows: interface → IP. Heuristic mapping from snapshot.ips.
        let ts_state = if snapshot.tailscale == "ON" { "ON" } else { "OFF" };
        let mut rows = Vec::new();
        for ip in &snapshot.ips {
            if ip.starts_with("100.") {
                rows.push(("tailscale0", ip.as_str()));
            } else if ip.starts_with("192.168.137.") {
                rows.push(("eth0", ip.as_str()));
            } else if ip.starts_with("192.168.1.") {
                rows.push(("wlan0", ip.as_str()));
            } else {
                rows.push(("interface", ip.as_str()));
            }
        }
        for (i, (iface, ip)) in rows.iter().take(3).enumerate() {
            let right = if i == 0 {
                ("TS", ts_state)
            } else {
                ("", "")
            };
            self.info.set(fb, i, (iface, ip), right);
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
        self.last_down.clear();
        self.last_up.clear();
        fb.mark_full_dirty();
    }
}

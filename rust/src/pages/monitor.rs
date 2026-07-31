//! Monitor page: replicates `monitor_mode.py` layout and behaviour.
//!
//! All mutable text fields are `Label` instances; only changed values redraw.
//! CPU bars are `Bar` instances. Dirty regions are honest: no blanket marks.

use std::time::Instant;

use anyhow::Result;
use time::OffsetDateTime;

use crate::config::{
    self, ACCENT, BG, CONTAINER_PAGE_SIZE, CYAN, DOCKER_HEADER_Y, DOCKER_LINE_HEIGHT,
    DOCKER_LIST_Y, DOCKER_START_Y, GRAY, GREEN, PANEL, RED, SLOW_RENDER_INTERVAL,
    TEMP_COLOR_LUT, USAGE_COLOR_LUT, W, WHITE, YELLOW,
};
use crate::config::{parse_percent, parse_temp};
use crate::fb::Framebuffer;
use crate::label::{Align, Bar, Label};
use crate::metrics::{ContainerInfo, MetricsSnapshot};
use crate::pages::{Page, PageAction};
use crate::render::{draw_line_h, fill_rect};
use crate::text::{FontWeight, Fonts, TextStyle};
use crate::touch::TouchEvent;

/// A row of container labels: name, status, state.
struct ContainerRow {
    name: Label,
    status: Label,
    state: Label,
}

pub struct MonitorPage {
    fonts: Fonts,
    container_scroll_offset: usize,
    bg_done: bool,
    last_slow_render: f32,
    start: Instant,

    // Top panel
    host_label: Label,
    time_label: Label,
    ts_label: Label,

    // IP row
    ip_label: Label,

    // CPU bars
    cpu_labels: [Label; 4],
    cpu_bars: [Bar; 4],
    cpu_pct_labels: [Label; 4],

    // Metric values
    temp_label: Label,
    mem_label: Label,
    disk_label: Label,

    // Container list
    container_header_name: Label,
    container_header_status: Label,
    container_header_state: Label,
    container_rows: Vec<ContainerRow>,
    page_label: Label,

    // Bottom bar
    footer_label: Label,
    fps_label: Label,

    // Cached metric label x positions
    metric_value_x: [i32; 3],
}

impl MonitorPage {
    pub fn new(fonts: Fonts) -> Self {
        let host_style = TextStyle::new(16, CYAN, false);
        let title_style = TextStyle::new(16, WHITE, false);
        let small_style = TextStyle::new(11, GRAY, false);
        let text_style = TextStyle::new(13, WHITE, false);
        // Docker list uses Regular at 11 px: size 10 makes strokes like 'm'
        // collapse into a blob on the 480×320 panel regardless of weight.
        let tiny_style = TextStyle::new(11, WHITE, false).with_weight(FontWeight::Regular);

        let host_label = Label::new(8, 6, host_style, Align::Left, PANEL, &fonts);
        // Time x will be recomputed on first render.
        let time_label = Label::new(0, 6, title_style, Align::Left, PANEL, &fonts);
        let ts_label = Label::new(295, 8, small_style, Align::Left, PANEL, &fonts);

        let ip_label = Label::new(8, 34, TextStyle::new(13, CYAN, false), Align::Left, BG, &fonts);

        let core_positions = [(8, 51), (248, 51), (8, 68), (248, 68)];
        let cpu_labels = core_positions.map(|(x, y)| {
            Label::new(x, y, TextStyle::new(13, GRAY, false), Align::Left, BG, &fonts)
        });
        let cpu_bars = core_positions.map(|(x, y)| {
            Bar::new(x + 44, y + 3, 130, 9, ACCENT)
        });
        let cpu_pct_labels = core_positions.map(|(x, y)| {
            Label::new(x + 44 + 130 + 6, y, TextStyle::new(13, WHITE, false), Align::Left, BG, &fonts)
        });

        // Static metric labels. Their values never change; use a dummy label and
        // draw them once in the static background instead.
        let temp_label = Label::new(0, 85, text_style, Align::Left, BG, &fonts);
        let mem_label = Label::new(0, 85, text_style, Align::Left, BG, &fonts);
        let disk_label = Label::new(0, 85, text_style, Align::Left, BG, &fonts);

        let header_style = TextStyle::new(11, GRAY, false);
        let container_header_name = Label::new(20, DOCKER_HEADER_Y, header_style, Align::Left, BG, &fonts);
        let container_header_status = Label::new(175, DOCKER_HEADER_Y, header_style, Align::Left, BG, &fonts);
        let container_header_state = Label::new(360, DOCKER_HEADER_Y, header_style, Align::Left, BG, &fonts);

        let mut container_rows = Vec::with_capacity(CONTAINER_PAGE_SIZE);
        for i in 0..CONTAINER_PAGE_SIZE {
            let y = DOCKER_LIST_Y + i as i32 * DOCKER_LINE_HEIGHT;
            container_rows.push(ContainerRow {
                name: Label::new(20, y, tiny_style, Align::Left, BG, &fonts),
                status: Label::new(175, y, TextStyle::new(11, GRAY, false).with_weight(FontWeight::Regular), Align::Left, BG, &fonts),
                state: Label::new(360, y, tiny_style, Align::Left, BG, &fonts),
            });
        }

        let page_label = Label::new(420, DOCKER_LIST_Y, TextStyle::new(11, YELLOW, false), Align::Left, BG, &fonts);
        let footer_label = Label::new(8, config::H as i32 - 18, TextStyle::new(11, GRAY, false).with_weight(FontWeight::Regular), Align::Left, PANEL, &fonts);
        let fps_label = Label::new(340, config::H as i32 - 18, TextStyle::new(11, GRAY, false).with_weight(FontWeight::Regular), Align::Left, PANEL, &fonts);

        Self {
            fonts,
            container_scroll_offset: 0,
            bg_done: false,
            last_slow_render: -1000.0,
            start: Instant::now(),
            host_label,
            time_label,
            ts_label,
            ip_label,
            cpu_labels,
            cpu_bars,
            cpu_pct_labels,
            temp_label,
            mem_label,
            disk_label,
            container_header_name,
            container_header_status,
            container_header_state,
            container_rows,
            page_label,
            footer_label,
            fps_label,
            metric_value_x: [0; 3],
        }
    }

    fn now_secs(&self) -> f32 {
        self.start.elapsed().as_secs_f32()
    }

    fn draw_static_background(&mut self, fb: &mut Framebuffer) {
        // Clear screen.
        fill_rect(fb, 0, 0, W as i32, config::H as i32, BG);

        // Top panel.
        fill_rect(fb, 0, 0, W as i32, 30, PANEL);

        // CPU bars: static labels and tracks.
        for (idx, (_x, _y)) in [(8, 51), (248, 51), (8, 68), (248, 68)].iter().enumerate() {
            self.cpu_labels[idx].force_draw(fb, &self.fonts, &format!("CPU{idx}"));
            self.cpu_bars[idx].force_draw(fb, 0.0, config::GREEN);
        }
        // Static metric labels.
        for (label, x, text) in [
            ("TEMP", 8, "TEMP "),
            ("MEM", 126, "MEM "),
            ("DISK", 360, "DISK "),
        ] {
            let style = TextStyle::new(13, GRAY, false);
            let w = self.fonts.measure(text, &style) as i32;
            self.fonts.draw(fb, text, x, self.fonts.baseline_y(85, &style), &style);
            let idx = match label {
                "TEMP" => 0,
                "MEM" => 1,
                _ => 2,
            };
            self.metric_value_x[idx] = x + w;
        }
        // Horizontal separator.
        draw_line_h(fb, 0, W as i32, 100, ACCENT);

        // Container header.
        self.container_header_name.force_draw(fb, &self.fonts, "CONTAINER");
        self.container_header_status.force_draw(fb, &self.fonts, "STATUS");
        self.container_header_state.force_draw(fb, &self.fonts, "STATE");
        // Bottom status bar.
        fill_rect(fb, 0, config::H as i32 - 20, W as i32, 20, PANEL);
        self.footer_label.force_draw(fb, &self.fonts, "Powered by RichLi4307");
        self.fps_label.force_draw(fb, &self.fonts, "15 FPS");
    }

    fn draw_slow_content(&mut self, fb: &mut Framebuffer, snapshot: &MetricsSnapshot) {
        // Host name.
        self.host_label.set(fb, &self.fonts, "FocusRasPi4B");
        // Time: recompute x based on host width.
        let host_w = self.fonts.measure("FocusRasPi4B", &TextStyle::new(16, CYAN, false)) as i32;
        self.time_label.set_x(8 + host_w + 10);
        let now_str = {
            let dt = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
            format!("{:02}:{:02}:{:02}", dt.hour(), dt.minute(), dt.second())
        };
        self.time_label.set(fb, &self.fonts, &now_str);

        // Tailscale status.
        let ts_color = if snapshot.tailscale == "ON" { GREEN } else { RED };
        self.ts_label.set_style_color(ts_color);
        self.ts_label.set(fb, &self.fonts, &format!("TS:{}", snapshot.tailscale));

        // IP row.
        let ip_str = snapshot.ips.iter().take(3).cloned().collect::<Vec<_>>().join("             ");
        self.ip_label.set(fb, &self.fonts, &format!("IP {ip_str}"));

        // Metric values.
        self.temp_label.set_x(self.metric_value_x[0]);
        self.temp_label
            .set_style_color(Self::temp_color(&snapshot.temp));
        self.temp_label.set(fb, &self.fonts, &snapshot.temp);

        self.mem_label.set_x(self.metric_value_x[1]);
        self.mem_label
            .set_style_color(Self::usage_color(&snapshot.mem));
        self.mem_label.set(fb, &self.fonts, &snapshot.mem);

        self.disk_label.set_x(self.metric_value_x[2]);
        self.disk_label
            .set_style_color(Self::usage_color(&snapshot.disk));
        self.disk_label.set(fb, &self.fonts, &snapshot.disk);

        // Container list.
        let total = snapshot.containers.len();
        let max_offset = total.saturating_sub(CONTAINER_PAGE_SIZE);
        if self.container_scroll_offset > max_offset {
            self.container_scroll_offset = max_offset;
        }
        let offset = self.container_scroll_offset;
        let visible: Vec<&ContainerInfo> = snapshot
            .containers
            .iter()
            .skip(offset)
            .take(CONTAINER_PAGE_SIZE)
            .collect();

        for (i, row) in self.container_rows.iter_mut().enumerate() {
            if let Some((name, status, state)) = visible.get(i).map(|c| (&c.0, &c.1, &c.2)) {
                let color = match state.as_str() {
                    "running" => GREEN,
                    "exited" => RED,
                    _ => YELLOW,
                };
                row.name.set(fb, &self.fonts, name);
                row.status.set(fb, &self.fonts, status);
                row.state.set_style_color(color);
                row.state.set(fb, &self.fonts, state);
            } else {
                row.name.clear(fb);
                row.status.clear(fb);
                row.state.clear(fb);
            }
        }

        let total_pages = ((total + CONTAINER_PAGE_SIZE - 1) / CONTAINER_PAGE_SIZE).max(1);
        let current_page = (offset / CONTAINER_PAGE_SIZE) + 1;
        if total_pages > 1 {
            self.page_label
                .set(fb, &self.fonts, &format!("{current_page}/{total_pages}"));
        } else {
            self.page_label.clear(fb);
        }
    }

    fn draw_cpu_bars(&mut self, fb: &mut Framebuffer, cpu: &[(String, f32)]) {
        for idx in 0..4 {
            let pct = cpu
                .iter()
                .find(|(name, _)| name == &format!("cpu{idx}"))
                .map(|(_, v)| *v)
                .unwrap_or(0.0);
            let color = USAGE_COLOR_LUT[pct as usize];
            self.cpu_bars[idx].set(fb, pct, color);
            let pct_text = format!("{:.0}%", pct);
            self.cpu_pct_labels[idx].set_style_color(color);
            self.cpu_pct_labels[idx].set(fb, &self.fonts, &pct_text);
        }
    }

    fn usage_color(text: &str) -> u32 {
        parse_percent(text)
            .and_then(|p| USAGE_COLOR_LUT.get(p as usize).copied())
            .unwrap_or(GRAY)
    }

    fn temp_color(text: &str) -> u32 {
        parse_temp(text)
            .and_then(|t| TEMP_COLOR_LUT.get(t as usize).copied())
            .unwrap_or(GRAY)
    }
}

impl Page for MonitorPage {
    fn id(&self) -> &'static str {
        "monitor"
    }

    fn render(&mut self, fb: &mut Framebuffer, snapshot: &MetricsSnapshot) -> Result<()> {
        if !self.bg_done {
            self.draw_static_background(fb);
            self.bg_done = true;
            fb.mark_full_dirty();
        }

        let now = self.now_secs();
        if now - self.last_slow_render >= SLOW_RENDER_INTERVAL {
            self.draw_slow_content(fb, snapshot);
            self.last_slow_render = now;
        }

        self.draw_cpu_bars(fb, &snapshot.cpu);
        Ok(())
    }

    fn on_touch(&mut self, ev: TouchEvent) -> PageAction {
        if !ev.pressed {
            return PageAction::None;
        }
        let y = ev.y;

        if y < DOCKER_START_Y || y > DOCKER_START_Y + (CONTAINER_PAGE_SIZE as i32) * DOCKER_LINE_HEIGHT + 20 {
            return PageAction::None;
        }

        if y < DOCKER_LIST_Y {
            return PageAction::None;
        }

        self.container_scroll_offset += 1;
        PageAction::None
    }

    fn on_enter(&mut self, fb: &mut Framebuffer) {
        self.bg_done = false;
        self.last_slow_render = -1000.0;
        self.container_scroll_offset = 0;
        fb.mark_full_dirty();
    }

    fn scroll_containers(&mut self, total: usize) -> Option<(usize, usize)> {
        let max_offset = total.saturating_sub(CONTAINER_PAGE_SIZE);
        if max_offset == 0 {
            return Some((0, total));
        }
        self.container_scroll_offset += 1;
        if self.container_scroll_offset > max_offset {
            self.container_scroll_offset = 0;
        }
        Some((self.container_scroll_offset, total))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pages::Page;
    use crate::text::Fonts;
    use crate::touch::TouchEvent;

    fn make_page() -> MonitorPage {
        let fonts = Fonts::load().expect("fonts should load");
        MonitorPage::new(fonts)
    }

    #[test]
    fn touch_release_ignored() {
        let mut page = make_page();
        let ev = TouchEvent { x: 100, y: 150, pressed: false, timestamp: 0.0 };
        assert_eq!(page.on_touch(ev), PageAction::None);
    }

    #[test]
    fn touch_container_list_increments_offset() {
        let mut page = make_page();
        let y = DOCKER_LIST_Y + 4;
        let ev = TouchEvent { x: 10, y, pressed: true, timestamp: 0.0 };
        assert_eq!(page.on_touch(ev), PageAction::None);
        assert_eq!(page.container_scroll_offset, 1);
    }

    #[test]
    fn scroll_containers_wraps() {
        let mut page = make_page();
        let total = 25;
        let max_offset = total - CONTAINER_PAGE_SIZE;
        for expected in 1..=max_offset {
            let (offset, _) = page.scroll_containers(total).unwrap();
            assert_eq!(offset, expected);
        }
        let (offset, tot) = page.scroll_containers(total).unwrap();
        assert_eq!(offset, 0);
        assert_eq!(tot, total);
    }

    #[test]
    fn headless_golden_render() {
        use crate::screenshot::encode_png;

        let mut page = make_page();
        let mut fb = Framebuffer::headless();
        let snapshot = MetricsSnapshot {
            ips: vec!["192.168.1.250".into(), "100.118.236.1".into()],
            tailscale: "ON".into(),
            containers: vec![
                ("astrbot".into(), "Up 15 hours".into(), "running".into()),
                ("napcat".into(), "Up 15 hours".into(), "running".into()),
            ],
            disk: "11%".into(),
            temp: "63C".into(),
            mem: "3200/7801MB (41%)".into(),
            cpu: vec![
                ("cpu0".into(), 58.0),
                ("cpu1".into(), 100.0),
                ("cpu2".into(), 12.0),
                ("cpu3".into(), 0.0),
            ],
        };
        page.render(&mut fb, &snapshot).unwrap();
        page.render(&mut fb, &snapshot).unwrap();
        let png = encode_png(fb.buffer(), crate::config::W, crate::config::H).expect("png encode");
        assert!(!png.is_empty());
        let path = "/tmp/pi_dashboard_golden_rust.png";
        std::fs::write(path, png).expect("write png");
        // For automated diff, a Python reference generator compares against this file.
    }
}

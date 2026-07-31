//! Monitor page: visual language v2 layout.
//!
//! Static background is drawn once; all mutable fields are widgets that erase
//! and redraw only on change. No blanket mark_dirty.

use std::collections::VecDeque;
use std::time::Instant;

use anyhow::Result;
use time::OffsetDateTime;

use crate::config::{
    self, ACCENT, ALARM, BG, CAUTION, CONTAINER_PAGE_SIZE, COOL, CYAN,
    DOCKER_HEADER_Y, DOCKER_LINE_HEIGHT, DOCKER_LIST_Y, GRAY, OK, PANEL,
    ROW_STRIPE, SCROLL_TRACK, SLOW_RENDER_INTERVAL, TREND_HOT, USAGE_COLOR_LUT,
    W, WHITE,
};
use crate::config::{parse_percent, parse_temp, temp_band_color, usage_text_color};
use crate::fb::{Framebuffer, Rect};
use crate::label::{Align, Label};
use crate::metrics::{abbreviate_status, ContainerInfo, MetricsSnapshot};
use crate::pages::{Page, PageAction};
use crate::render::{
    draw_line_h, fill_ellipse, fill_rect, fill_rounded_rect, fill_triangle,
};
use crate::text::{FontWeight, Fonts, TextStyle};
use crate::touch::TouchEvent;

const TEMP_TREND_WINDOW_SECS: f32 = 60.0;
const TEMP_TREND_COMPARE_SECS: f32 = 30.0;
const TEMP_TREND_DEADBAND: f32 = 1.0;

// ---------------------------------------------------------------------------
// Layout constants (480×320, 8 px grid).
// ---------------------------------------------------------------------------
const TOP_PANEL_H: i32 = 32;
const HERO_CARD_X: [i32; 3] = [12, 167, 322];
const HERO_CARD_W: i32 = 146;
const HERO_CARD_H: i32 = 40;
const HERO_CARD_Y: i32 = 40;

const CPU_ROWS: [i32; 2] = [100, 126];
const CPU_COLS: [i32; 2] = [12, 244];
const CPU_CELL_RIGHT: [i32; 2] = [236, 468];
const CPU_BAR_X_OFF: i32 = 38;
const CPU_BAR_Y_OFF: i32 = 1;
const CPU_BAR_W: i32 = 151;
const CPU_BAR_H: i32 = 10;
const CPU_BAR_R: i32 = 5;

const DOCKER_RIGHT: i32 = 468;
const DOCKER_TRACK_X: i32 = 460;
const DOCKER_TRACK_W: i32 = 4;
const DOCKER_CONTENT_RIGHT: i32 = 456;
const DOCKER_ZEBRA_RIGHT: i32 = 456;
const NAME_X: i32 = 26;
const PAGE_RIGHT: i32 = 264;
const UPTIME_RIGHT: i32 = 336;
const STATE_RIGHT: i32 = 416;
const CPU_RIGHT: i32 = 456;
const UNDERLINE_Y: i32 = 172;
const DOT_X: i32 = 16;

const TS_CHIP_RIGHT: i32 = 383;
const TS_CHIP_W: i32 = 46;
const TS_CHIP_H: i32 = 18;
const TS_CHIP_Y: i32 = 7;
const TS_CHIP_R: i32 = 9;
const TIME_RIGHT: i32 = 468;

const TREND_W: i32 = 8;
const TREND_H: i32 = 6;

// ---------------------------------------------------------------------------
// Temperature trend state.
// ---------------------------------------------------------------------------
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TrendState {
    None,
    Rising,
    Falling,
    Steady,
}

// ---------------------------------------------------------------------------
// Tailscale status chip: rounded pill with state dot + "TS".
// ---------------------------------------------------------------------------
struct TsChip {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    r: i32,
    dot_x: i32,
    dot_y: i32,
    dot_r: i32,
    text_x: i32,
    text_baseline_y: i32,
    text_style: TextStyle,
    last_on: Option<bool>,
    last_text_bbox: Option<Rect>,
}

impl TsChip {
    fn new(x: i32, y: i32, w: i32, h: i32, r: i32, fonts: &Fonts) -> Self {
        let dot_r = 3;
        let dot_x = x + 8 + dot_r;
        let dot_y = y + h / 2;
        let text_x = dot_x + dot_r + 5;
        let text_style = TextStyle::new(11, GRAY, false);
        let text_baseline_y = fonts.baseline_y(y + (h - 11) / 2, &text_style);
        Self {
            x,
            y,
            w,
            h,
            r,
            dot_x,
            dot_y,
            dot_r,
            text_x,
            text_baseline_y,
            text_style,
            last_on: None,
            last_text_bbox: None,
        }
    }

    fn draw(&mut self, fb: &mut Framebuffer, fonts: &Fonts, on: bool) {
        if self.last_on == Some(on) {
            return;
        }
        // Erase previous text (the pill background itself is redrawn below).
        if let Some(last) = self.last_text_bbox {
            fill_rect(fb, last.x1 as i32, last.y1 as i32, last.width() as i32, last.height() as i32, PANEL);
        }
        // Redraw the pill outline so old dots/text are fully covered.
        fill_rounded_rect(fb, self.x, self.y, self.w, self.h, self.r, PANEL);
        draw_rounded_rect_outline(fb, self.x, self.y, self.w, self.h, self.r, ACCENT);

        let color = if on { OK } else { ALARM };
        fill_ellipse(fb, self.dot_x, self.dot_y, self.dot_r, self.dot_r, color);

        let mut style = self.text_style;
        style.color = color;
        let text = "TS";
        fonts.draw(fb, text, self.text_x, self.text_baseline_y, &style);

        // Compute text bbox for next erase.
        let tw = fonts.measure(text, &style) as i32;
        let g = fonts.glyph_ref('T', &style);
        let gx = self.text_x + g.xmin;
        let gy = self.text_baseline_y - g.ymin - g.height as i32 + 1;
        self.last_text_bbox = Some(Rect::new(
            gx.max(0) as usize,
            gy.max(0) as usize,
            (gx + tw).max(0) as usize,
            (gy + 11).max(0) as usize,
        ));

        self.last_on = Some(on);
    }
}

fn draw_rounded_rect_outline(fb: &mut Framebuffer, x: i32, y: i32, w: i32, h: i32, r: i32, color: u32) {
    // Four edges, leaving the corner arcs untouched. Good enough for a 1px outline.
    draw_line_h(fb, x + r, x + w - r, y, color);
    draw_line_h(fb, x + r, x + w - r, y + h - 1, color);
    fill_rect(fb, x, y + r, 1, h - 2 * r, color);
    fill_rect(fb, x + w - 1, y + r, 1, h - 2 * r, color);
}

// ---------------------------------------------------------------------------
// Hero metric card: rounded panel with label and value.
// ---------------------------------------------------------------------------
struct HeroCard {
    label: Label,
    value: Label,
}

impl HeroCard {
    fn new(x: i32, y: i32, _label_text: &str, fonts: &Fonts) -> Self {
        let label_style = TextStyle::new(11, GRAY, false);
        let value_style = TextStyle::new(16, WHITE, false);
        Self {
            label: Label::new(x + 10, y + 8, label_style, Align::Left, PANEL, fonts),
            value: Label::new(x + 10, y + 23, value_style, Align::Left, PANEL, fonts),
        }
    }

    fn draw_card_background(&mut self, fb: &mut Framebuffer, x: i32, y: i32) {
        fill_rounded_rect(fb, x, y, HERO_CARD_W, HERO_CARD_H, 4, PANEL);
    }

    fn draw_label(&mut self, fb: &mut Framebuffer, fonts: &Fonts, text: &str) {
        self.label.force_draw(fb, fonts, text);
    }
}

// ---------------------------------------------------------------------------
// Rounded percentage bar (pill shape).
// ---------------------------------------------------------------------------
#[derive(Debug)]
struct RoundedBar {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    r: i32,
    track_color: u32,
    last_pct: i32,
    last_fill_color: Option<u32>,
}

impl RoundedBar {
    fn new(x: i32, y: i32, w: i32, h: i32, r: i32, track_color: u32) -> Self {
        Self {
            x,
            y,
            w,
            h,
            r,
            track_color,
            last_pct: -1,
            last_fill_color: None,
        }
    }

    fn fill_width(&self, pct: i32) -> i32 {
        ((self.w as f32 * pct as f32) / 100.0) as i32
    }

    fn set(&mut self, fb: &mut Framebuffer, pct: f32, fill_color: u32) {
        let pct_i = pct.round() as i32;
        let color_changed = self.last_fill_color != Some(fill_color);
        if pct_i == self.last_pct && !color_changed {
            return;
        }
        // Full redraw: track + fill.
        fill_rounded_rect(fb, self.x, self.y, self.w, self.h, self.r, self.track_color);
        let fw = self.fill_width(pct_i);
        if fw > 0 {
            fill_rounded_rect(fb, self.x, self.y, fw, self.h, self.r, fill_color);
        }
        self.last_pct = pct_i;
        self.last_fill_color = Some(fill_color);
    }

    fn force_draw(&mut self, fb: &mut Framebuffer, pct: f32, fill_color: u32) {
        self.last_pct = -1;
        self.last_fill_color = None;
        self.set(fb, pct, fill_color);
    }
}

// ---------------------------------------------------------------------------
// CPU pill: label + rounded bar + right-aligned percentage.
// ---------------------------------------------------------------------------
struct CpuPill {
    label: Label,
    bar: RoundedBar,
    pct: Label,
}

impl std::fmt::Debug for CpuPill {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CpuPill").finish()
    }
}

// ---------------------------------------------------------------------------
// Docker table row.
// ---------------------------------------------------------------------------
struct ContainerRow {
    y: i32,
    bg: u32,
    dot_color: Option<u32>,
    name: Label,
    uptime: Label,
    state: Label,
    cpu: Label,
}

// ---------------------------------------------------------------------------
// Monitor page.
// ---------------------------------------------------------------------------
pub struct MonitorPage {
    fonts: Fonts,
    bg_done: bool,
    last_slow_render: f32,
    start: Instant,
    container_scroll_offset: usize,

    // Top bar
    host_label: Label,
    time_label: Label,
    ts_chip: TsChip,

    // Hero cards
    temp_card: HeroCard,
    mem_card: HeroCard,
    disk_card: HeroCard,

    // CPU pills
    cpu_pills: [CpuPill; 4],

    // Docker table
    header_name: Label,
    header_uptime: Label,
    header_state: Label,
    header_cpu: Label,
    page_label: Label,
    container_rows: Vec<ContainerRow>,

    scroll_last_offset: usize,
    scroll_last_total: usize,

    // Bottom bar
    footer_label: Label,
    fps_label: Label,

    // Temperature trend
    temp_trend_history: VecDeque<(f32, f32)>,
    temp_trend_last_state: TrendState,
    temp_trend_last_alarm: bool,
    temp_trend_last_bbox: Option<Rect>,
}

impl MonitorPage {
    pub fn new(fonts: Fonts) -> Self {
        let host_style = TextStyle::new(16, CYAN, false);
        let time_style = TextStyle::new(16, WHITE, false);

        let host_label = Label::new(12, 8, host_style, Align::Left, PANEL, &fonts);
        let time_label = Label::new(TIME_RIGHT, 8, time_style, Align::Right, PANEL, &fonts);
        let ts_chip = TsChip::new(
            TS_CHIP_RIGHT - TS_CHIP_W,
            TS_CHIP_Y,
            TS_CHIP_W,
            TS_CHIP_H,
            TS_CHIP_R,
            &fonts,
        );

        let temp_card = HeroCard::new(HERO_CARD_X[0], HERO_CARD_Y, "TEMP", &fonts);
        let mem_card = HeroCard::new(HERO_CARD_X[1], HERO_CARD_Y, "MEM", &fonts);
        let disk_card = HeroCard::new(HERO_CARD_X[2], HERO_CARD_Y, "DISK", &fonts);

        let cpu_label_style = TextStyle::new(11, GRAY, false).with_weight(FontWeight::Regular);
        let cpu_pct_style = TextStyle::new(11, WHITE, false).with_weight(FontWeight::Regular);
        let mut cpu_pills = Vec::with_capacity(4);
        for (_row_idx, row) in CPU_ROWS.iter().enumerate() {
            for (col_idx, col) in CPU_COLS.iter().enumerate() {
                let right = CPU_CELL_RIGHT[col_idx];
                cpu_pills.push(CpuPill {
                    label: Label::new(*col, *row, cpu_label_style, Align::Left, BG, &fonts),
                    bar: RoundedBar::new(
                        col + CPU_BAR_X_OFF,
                        row + CPU_BAR_Y_OFF,
                        CPU_BAR_W,
                        CPU_BAR_H,
                        CPU_BAR_R,
                        ACCENT,
                    ),
                    pct: Label::new(right, *row, cpu_pct_style, Align::Right, BG, &fonts),
                });
            }
        }

        let header_style = TextStyle::new(11, GRAY, false).with_weight(FontWeight::Regular);
        let header_name = Label::new(NAME_X, DOCKER_HEADER_Y, header_style, Align::Left, BG, &fonts);
        let header_uptime = Label::new(UPTIME_RIGHT, DOCKER_HEADER_Y, header_style, Align::Right, BG, &fonts);
        let header_state = Label::new(STATE_RIGHT, DOCKER_HEADER_Y, header_style, Align::Right, BG, &fonts);
        let header_cpu = Label::new(CPU_RIGHT, DOCKER_HEADER_Y, header_style, Align::Right, BG, &fonts);

        let mut container_rows = Vec::with_capacity(CONTAINER_PAGE_SIZE);
        for i in 0..CONTAINER_PAGE_SIZE {
            let y = DOCKER_LIST_Y + i as i32 * DOCKER_LINE_HEIGHT;
            let bg = if i % 2 == 1 { ROW_STRIPE } else { BG };
            let row_style = TextStyle::new(11, WHITE, false).with_weight(FontWeight::Regular);
            let gray_style = TextStyle::new(11, GRAY, false).with_weight(FontWeight::Regular);
            container_rows.push(ContainerRow {
                y,
                bg,
                dot_color: None,
                name: Label::new(NAME_X, y, row_style, Align::Left, bg, &fonts),
                uptime: Label::new(UPTIME_RIGHT, y, gray_style, Align::Right, bg, &fonts),
                state: Label::new(STATE_RIGHT, y, row_style, Align::Right, bg, &fonts),
                cpu: Label::new(CPU_RIGHT, y, row_style, Align::Right, bg, &fonts),
            });
        }

        let page_label = Label::new(PAGE_RIGHT, DOCKER_HEADER_Y, header_style, Align::Right, BG, &fonts);
        let footer_style = TextStyle::new(11, GRAY, false).with_weight(FontWeight::Regular);
        let footer_label = Label::new(12, config::H as i32 - 18, footer_style, Align::Left, PANEL, &fonts);
        let fps_label = Label::new(TIME_RIGHT, config::H as i32 - 18, footer_style, Align::Right, PANEL, &fonts);

        Self {
            fonts,
            bg_done: false,
            last_slow_render: -1000.0,
            start: Instant::now(),
            container_scroll_offset: 0,
            host_label,
            time_label,
            ts_chip,
            temp_card,
            mem_card,
            disk_card,
            cpu_pills: cpu_pills.try_into().unwrap(),
            header_name,
            header_uptime,
            header_state,
            header_cpu,
            page_label,
            container_rows,
            scroll_last_offset: usize::MAX,
            scroll_last_total: usize::MAX,
            footer_label,
            fps_label,
            temp_trend_history: VecDeque::new(),
            temp_trend_last_state: TrendState::None,
            temp_trend_last_alarm: false,
            temp_trend_last_bbox: None,
        }
    }

    fn now_secs(&self) -> f32 {
        self.start.elapsed().as_secs_f32()
    }

    // -----------------------------------------------------------------------
    // Static background.
    // -----------------------------------------------------------------------
    fn draw_static_background(&mut self, fb: &mut Framebuffer) {
        fill_rect(fb, 0, 0, W as i32, config::H as i32, BG);

        // Top panel.
        fill_rect(fb, 0, 0, W as i32, TOP_PANEL_H, PANEL);

        // Hero cards.
        for x in HERO_CARD_X {
            self.temp_card.draw_card_background(fb, x, HERO_CARD_Y);
        }
        self.temp_card.draw_label(fb, &self.fonts, "TEMP");
        self.mem_card.draw_label(fb, &self.fonts, "MEM");
        self.disk_card.draw_label(fb, &self.fonts, "DISK");

        // CPU labels and empty bars.
        for (idx, pill) in self.cpu_pills.iter_mut().enumerate() {
            pill.label.force_draw(fb, &self.fonts, &format!("CPU{idx}"));
            pill.bar.force_draw(fb, 0.0, OK);
        }

        // Docker header.
        self.header_name.force_draw(fb, &self.fonts, "NAME");
        self.header_uptime.force_draw(fb, &self.fonts, "UPTIME");
        self.header_state.force_draw(fb, &self.fonts, "STATE");
        self.header_cpu.force_draw(fb, &self.fonts, "CPU");
        draw_line_h(fb, 12, DOCKER_RIGHT, UNDERLINE_Y, ACCENT);

        // Zebra stripes.
        for i in 0..CONTAINER_PAGE_SIZE as i32 {
            let y = DOCKER_LIST_Y + i * DOCKER_LINE_HEIGHT;
            if i % 2 == 1 {
                fill_rect(fb, 12, y, DOCKER_ZEBRA_RIGHT - 12, DOCKER_LINE_HEIGHT, ROW_STRIPE);
            }
        }

        // Bottom bar.
        fill_rect(fb, 0, config::H as i32 - 20, W as i32, 20, PANEL);
        self.footer_label.force_draw(fb, &self.fonts, "Powered by RichLi4307");
        self.fps_label.force_draw(fb, &self.fonts, "15 FPS");
    }

    // -----------------------------------------------------------------------
    // Slow content (1 Hz).
    // -----------------------------------------------------------------------
    fn draw_slow_content(&mut self, fb: &mut Framebuffer, snapshot: &MetricsSnapshot) {
        self.host_label.set(fb, &self.fonts, "FocusRasPi4B");

        let now_str = {
            let dt = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
            format!("{:02}:{:02}:{:02}", dt.hour(), dt.minute(), dt.second())
        };
        self.time_label.set(fb, &self.fonts, &now_str);

        let ts_on = snapshot.tailscale == "ON";
        self.ts_chip.draw(fb, &self.fonts, ts_on);

        // Hero values.
        let temp_val = parse_temp(&snapshot.temp).unwrap_or(50);
        let temp_color = temp_band_color(temp_val);
        self.temp_card.value.set_style_color(temp_color);
        self.temp_card.value.set(fb, &self.fonts, &snapshot.temp);

        let mem_pct = parse_percent(&snapshot.mem).unwrap_or(0);
        self.mem_card.value.set_style_color(usage_text_color(mem_pct));
        self.mem_card.value.set(fb, &self.fonts, &format!("{mem_pct}%"));

        let disk_pct = parse_percent(&snapshot.disk).unwrap_or(0);
        self.disk_card.value.set_style_color(usage_text_color(disk_pct));
        self.disk_card.value.set(fb, &self.fonts, &format!("{disk_pct}%"));

        // Temperature trend arrow.
        let now = self.now_secs();
        self.update_temp_trend(now, temp_val as f32);
        let (trend_state, trend_alarm) = self.evaluate_temp_trend(now);
        let value_style = TextStyle::new(16, temp_color, false);
        let value_w = self.fonts.measure(&snapshot.temp, &value_style) as i32;
        let trend_x = HERO_CARD_X[0] + 10 + value_w + 4;
        let trend_y = self.temp_card.value.baseline_y();
        let fonts = self.fonts.clone();
        self.draw_temp_trend(
            fb,
            &fonts,
            trend_x,
            trend_y,
            trend_state,
            trend_alarm,
        );
    }

    // -----------------------------------------------------------------------
    // CPU pills (fast path).
    // -----------------------------------------------------------------------
    fn draw_cpu_pills(&mut self, fb: &mut Framebuffer, cpu: &[(String, f32)]) {
        for idx in 0..4 {
            let pct = cpu
                .iter()
                .find(|(name, _)| name == &format!("cpu{idx}"))
                .map(|(_, v)| *v)
                .unwrap_or(0.0);
            let fill_color = USAGE_COLOR_LUT[pct as usize];
            self.cpu_pills[idx].bar.set(fb, pct, fill_color);
            let pct_text = format!("{:.0}%", pct);
            let pct_i = pct.round() as i32;
            self.cpu_pills[idx].pct.set_style_color(usage_text_color(pct_i));
            self.cpu_pills[idx].pct.set(fb, &self.fonts, &pct_text);
        }
    }

    // -----------------------------------------------------------------------
    // Docker table (slow path, called from draw_slow_content ideally; here kept
    // separate for clarity).
    // -----------------------------------------------------------------------
    fn draw_docker_table(&mut self, fb: &mut Framebuffer, snapshot: &MetricsSnapshot) {
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
            if let Some(c) = visible.get(i) {
                let status_unhealthy = c.status.contains("(unhealthy)");
                let state_color = Self::container_state_color(&c.state, status_unhealthy);
                let name_style = TextStyle::new(11, WHITE, false).with_weight(FontWeight::Regular);

                let display_name = Self::truncate_to_width(&c.name, (PAGE_RIGHT - NAME_X - 4) as f32, &self.fonts, &name_style);
                row.name.set(fb, &self.fonts, &display_name);

                let abbr = abbreviate_status(&c.status);
                row.uptime.set(fb, &self.fonts, &abbr);

                row.state.set_style_color(state_color);
                row.state.set(fb, &self.fonts, &c.state);

                let cpu_text = match c.cpu {
                    Some(pct) => {
                        row.cpu.set_style_color(usage_text_color(pct.round() as i32));
                        if pct >= 100.0 {
                            "100%".to_string()
                        } else {
                            format!("{:.1}%", pct)
                        }
                    }
                    None => {
                        row.cpu.set_style_color(GRAY);
                        "-".to_string()
                    }
                };
                row.cpu.set(fb, &self.fonts, &cpu_text);

                Self::draw_state_dot(fb, row, state_color);
            } else {
                row.name.clear(fb);
                row.uptime.clear(fb);
                row.state.clear(fb);
                row.cpu.clear(fb);
                Self::clear_state_dot(fb, row);
            }
        }

        self.draw_scroll_track(fb, offset, total);

        let total_pages = ((total + CONTAINER_PAGE_SIZE - 1) / CONTAINER_PAGE_SIZE).max(1);
        let current_page = (offset / CONTAINER_PAGE_SIZE) + 1;
        if total_pages > 1 {
            self.page_label.set(fb, &self.fonts, &format!("{current_page}/{total_pages}"));
        } else {
            self.page_label.clear(fb);
        }
    }

    // -----------------------------------------------------------------------
    // Helpers.
    // -----------------------------------------------------------------------
    fn truncate_to_width(text: &str, max_width: f32, fonts: &Fonts, style: &TextStyle) -> String {
        if fonts.measure(text, style) <= max_width {
            return text.to_string();
        }
        let mut s = text.to_string();
        while !s.is_empty() {
            let candidate = format!("{}..", s);
            if fonts.measure(&candidate, style) <= max_width {
                return candidate;
            }
            s.pop();
            while !s.is_empty() && !s.is_char_boundary(s.len()) {
                s.pop();
            }
        }
        "..".to_string()
    }

    fn container_state_color(state: &str, status_unhealthy: bool) -> u32 {
        if status_unhealthy || state == "dead" {
            ALARM
        } else if state == "running" {
            OK
        } else if state == "exited" {
            GRAY
        } else if matches!(state, "created" | "paused" | "restarting") {
            CAUTION
        } else {
            GRAY
        }
    }

    fn draw_state_dot(fb: &mut Framebuffer, row: &mut ContainerRow, color: u32) {
        if row.dot_color == Some(color) {
            return;
        }
        if row.dot_color.is_some() {
            fill_ellipse(fb, DOT_X, row.y + 7, 3, 3, row.bg);
        }
        fill_ellipse(fb, DOT_X, row.y + 7, 3, 3, color);
        row.dot_color = Some(color);
    }

    fn clear_state_dot(fb: &mut Framebuffer, row: &mut ContainerRow) {
        if row.dot_color.is_some() {
            fill_ellipse(fb, DOT_X, row.y + 7, 3, 3, row.bg);
            row.dot_color = None;
        }
    }

    fn draw_scroll_track(&mut self, fb: &mut Framebuffer, offset: usize, total: usize) {
        let total_pages = ((total + CONTAINER_PAGE_SIZE - 1) / CONTAINER_PAGE_SIZE).max(1);
        let track_top = DOCKER_LIST_Y;
        let track_h = DOCKER_LINE_HEIGHT * CONTAINER_PAGE_SIZE as i32;
        if total_pages <= 1 {
            if self.scroll_last_total != usize::MAX {
                fill_rect(fb, DOCKER_TRACK_X, track_top, DOCKER_TRACK_W, track_h, BG);
                self.scroll_last_offset = usize::MAX;
                self.scroll_last_total = usize::MAX;
            }
            return;
        }
        if self.scroll_last_offset == offset && self.scroll_last_total == total {
            return;
        }

        fill_rect(fb, DOCKER_TRACK_X, track_top, DOCKER_TRACK_W, track_h, BG);
        fill_rect(fb, DOCKER_TRACK_X, track_top, DOCKER_TRACK_W, track_h, SCROLL_TRACK);
        let max_offset = total.saturating_sub(CONTAINER_PAGE_SIZE);
        let thumb_h = (track_h as usize * CONTAINER_PAGE_SIZE / total).max(8);
        let thumb_y = if max_offset == 0 {
            track_top
        } else {
            track_top + (track_h - thumb_h as i32) * offset as i32 / max_offset as i32
        };
        fill_rect(fb, DOCKER_TRACK_X, thumb_y, DOCKER_TRACK_W, thumb_h as i32, GRAY);

        self.scroll_last_offset = offset;
        self.scroll_last_total = total;
    }

    // -----------------------------------------------------------------------
    // Temperature trend.
    // -----------------------------------------------------------------------
    fn update_temp_trend(&mut self, now: f32, temp: f32) {
        self.temp_trend_history.push_back((now, temp));
        while let Some(&(t, _)) = self.temp_trend_history.front() {
            if now - t > TEMP_TREND_WINDOW_SECS {
                self.temp_trend_history.pop_front();
            } else {
                break;
            }
        }
    }

    fn evaluate_temp_trend(&self, now: f32) -> (TrendState, bool) {
        if self.temp_trend_history.len() < 2 {
            return (TrendState::None, false);
        }
        let current = self.temp_trend_history.back().unwrap().1;
        let alarm = current >= 80.0;
        let target = now - TEMP_TREND_COMPARE_SECS;
        let mut old = None;
        for &(t, v) in self.temp_trend_history.iter().rev() {
            if t <= target {
                old = Some(v);
                break;
            }
        }
        let old = old.unwrap_or(self.temp_trend_history.front().unwrap().1);
        let delta = current - old;
        let state = if delta >= TEMP_TREND_DEADBAND {
            TrendState::Rising
        } else if delta <= -TEMP_TREND_DEADBAND {
            TrendState::Falling
        } else {
            TrendState::Steady
        };
        (state, alarm)
    }

    fn draw_temp_trend(
        &mut self,
        fb: &mut Framebuffer,
        fonts: &Fonts,
        x: i32,
        baseline_y: i32,
        state: TrendState,
        alarm: bool,
    ) {
        if state == TrendState::None {
            if let Some(last) = self.temp_trend_last_bbox {
                fill_rect(fb, last.x1 as i32, last.y1 as i32, last.width() as i32, last.height() as i32, PANEL);
                self.temp_trend_last_bbox = None;
            }
            self.temp_trend_last_state = TrendState::None;
            self.temp_trend_last_alarm = false;
            return;
        }

        let arrow_color = match state {
            TrendState::Rising => TREND_HOT,
            TrendState::Falling => COOL,
            TrendState::Steady => GRAY,
            TrendState::None => PANEL,
        };

        let mut new_bbox = Rect::new(
            x.max(0) as usize,
            (baseline_y - TREND_H / 2).max(0) as usize,
            (x + TREND_W).max(0) as usize,
            (baseline_y + TREND_H / 2 + 1).max(0) as usize,
        );
        let alarm_bbox = if alarm {
            let style = TextStyle::new(16, ALARM, false);
            let g = fonts.glyph_ref('!', &style);
            let ax = x + TREND_W + 4;
            let gx = ax + g.xmin;
            let gy = baseline_y - g.ymin - g.height as i32 + 1;
            Some(Rect::new(
                gx.max(0) as usize,
                gy.max(0) as usize,
                (gx + g.width as i32).max(0) as usize,
                (gy + g.height as i32).max(0) as usize,
            ))
        } else {
            None
        };
        if let Some(ab) = alarm_bbox {
            new_bbox = new_bbox.union(&ab);
        }

        if state == self.temp_trend_last_state
            && alarm == self.temp_trend_last_alarm
            && self.temp_trend_last_bbox == Some(new_bbox)
        {
            return;
        }

        if let Some(last) = self.temp_trend_last_bbox {
            fill_rect(fb, last.x1 as i32, last.y1 as i32, last.width() as i32, last.height() as i32, PANEL);
        }

        match state {
            TrendState::Rising => fill_triangle(fb, x + TREND_W / 2, baseline_y, TREND_W, TREND_H, true, arrow_color),
            TrendState::Falling => fill_triangle(fb, x + TREND_W / 2, baseline_y, TREND_W, TREND_H, false, arrow_color),
            TrendState::Steady => fill_rect(fb, x, baseline_y - 1, TREND_W, 2, arrow_color),
            TrendState::None => {}
        }
        if alarm {
            fonts.draw(fb, "!", x + TREND_W + 4, baseline_y, &TextStyle::new(16, ALARM, false));
        }

        self.temp_trend_last_state = state;
        self.temp_trend_last_alarm = alarm;
        self.temp_trend_last_bbox = Some(new_bbox);
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
            self.draw_docker_table(fb, snapshot);
            self.last_slow_render = now;
        }

        self.draw_cpu_pills(fb, &snapshot.cpu);
        Ok(())
    }

    fn on_touch(&mut self, ev: TouchEvent) -> PageAction {
        if !ev.pressed {
            return PageAction::None;
        }
        let y = ev.y;
        if y < DOCKER_HEADER_Y || y >= DOCKER_LIST_Y + CONTAINER_PAGE_SIZE as i32 * DOCKER_LINE_HEIGHT {
            return PageAction::None;
        }
        self.container_scroll_offset += 1;
        PageAction::None
    }

    fn on_enter(&mut self, fb: &mut Framebuffer) {
        self.bg_done = false;
        self.last_slow_render = -1000.0;
        self.container_scroll_offset = 0;
        self.temp_trend_history.clear();
        self.temp_trend_last_state = TrendState::None;
        self.temp_trend_last_alarm = false;
        self.temp_trend_last_bbox = None;
        self.scroll_last_offset = usize::MAX;
        self.scroll_last_total = usize::MAX;
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
        let ev = TouchEvent { x: 100, y: 200, pressed: false, timestamp: 0.0 };
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
                ContainerInfo { name: "astrbot".into(), id: "a1".into(), status: "Up 15 hours".into(), state: "running".into(), cpu: Some(0.8) },
                ContainerInfo { name: "napcat".into(), id: "a2".into(), status: "Up 15 hours".into(), state: "running".into(), cpu: Some(0.1) },
                ContainerInfo { name: "homeassistant".into(), id: "a3".into(), status: "Up 3 hours".into(), state: "running".into(), cpu: Some(2.1) },
                ContainerInfo { name: "pi-dashboard-mcp".into(), id: "a4".into(), status: "Up 3 hours".into(), state: "running".into(), cpu: None },
                ContainerInfo { name: "github-proxy".into(), id: "a5".into(), status: "Up 30 hours".into(), state: "running".into(), cpu: Some(0.0) },
                ContainerInfo { name: "hass-mcp".into(), id: "a6".into(), status: "Up 30 hours".into(), state: "running".into(), cpu: Some(0.0) },
                ContainerInfo { name: "mcp-python-sandbox".into(), id: "a7".into(), status: "Up 18 hours".into(), state: "running".into(), cpu: Some(0.5) },
                ContainerInfo { name: "exited-demo".into(), id: "a8".into(), status: "Exited (0) 3 hours ago".into(), state: "exited".into(), cpu: Some(0.0) },
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
    }
}

//! Monitor page: visual language v4 layout.
//!
//! Static background is drawn once; all mutable fields are widgets that erase
//! and redraw only on change. No blanket mark_dirty.

use std::collections::VecDeque;
use std::time::Instant;

use anyhow::Result;
use time::OffsetDateTime;

use crate::config::{
    self, hostname, ACCENT, ALARM, BG, CAUTION, CONTAINER_PAGE_SIZE, COOL, CYAN,
    DOCKER_HEADER_Y, DOCKER_LINE_HEIGHT, DOCKER_LIST_Y, GRAY, OK, PANEL,
    ROW_STRIPE, SCROLL_TRACK, SLOW_RENDER_INTERVAL, TREND_HOT, USAGE_COLOR_LUT,
    W, WHITE,
};
use crate::config::{parse_percent, parse_temp, temp_band_color, usage_text_color};
use crate::fb::{Framebuffer, Rect};
use crate::label::{Align, Label};
use crate::metrics::{abbreviate_status, fmt_rate, ContainerInfo, MetricsSnapshot};
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
const HERO_CARD_X: [i32; 4] = [12, 128, 244, 360];
const HERO_CARD_W: i32 = 108;
const HERO_CARD_H: i32 = 40;
const HERO_CARD_Y: i32 = 40;

const CPU_ROWS: [i32; 2] = [100, 126];
const CPU_COLS: [i32; 2] = [12, 256];
const CPU_CELL_RIGHT: [i32; 2] = [224, 468];
const CPU_BAR_X_OFF: i32 = 34;
const CPU_BAR_Y_OFF: i32 = 1;
const CPU_BAR_W: i32 = 143;
const CPU_BAR_H: i32 = 10;
const CPU_BAR_R: i32 = 5;
const CPU_AREA: (i32, i32, i32, i32) = (12, 88, 468, 148); // x1,y1,x2,y2

const DOCKER_RIGHT: i32 = 468;
const DOCKER_TRACK_X: i32 = 460;
const DOCKER_TRACK_W: i32 = 4;
const DOCKER_CONTENT_RIGHT: i32 = 450;
const DOCKER_ZEBRA_RIGHT: i32 = 456;
const DOT_X: i32 = 18;
const NAME_X: i32 = 30;
const NAME_TRUNCATE_WIDTH: f32 = 208.0;
const PAGE_RIGHT: i32 = 242;
const UPTIME_CENTER: i32 = 274;
const STATE_CENTER: i32 = 358;
const CPU_CENTER: i32 = 430;
const UNDERLINE_Y: i32 = 172;

const HOST_X: i32 = 12;
const HOST_Y: i32 = 8;
const TS_CHIP_X: i32 = 134;
const TS_CHIP_W: i32 = 46;
const TS_CHIP_H: i32 = 18;
const TS_CHIP_Y: i32 = 7;
const TS_CHIP_R: i32 = 9;
const MENU_CHIP_X: i32 = 188;
const MENU_CHIP_W: i32 = 42;
const MENU_CHIP_H: i32 = 18;
const MENU_CHIP_Y: i32 = 7;
const MENU_CHIP_R: i32 = 9;
const TIME_RIGHT: i32 = 388;
const RST_CHIP_X: i32 = 398;
const RST_CHIP_W: i32 = 30;
const RST_CHIP_H: i32 = 24;
const RST_CHIP_Y: i32 = 4;
const RST_CHIP_R: i32 = 6;
const PWR_CHIP_X: i32 = 434;
const PWR_CHIP_W: i32 = 30;
const PWR_CHIP_H: i32 = 24;
const PWR_CHIP_Y: i32 = 4;
const PWR_CHIP_R: i32 = 6;

const TREND_W: i32 = 10;
const TREND_H: i32 = 9;
const NET_UP_TREND_W: i32 = 7;
const NET_UP_TREND_H: i32 = 5;
const NET_DOWN_TREND_W: i32 = 10;
const NET_DOWN_TREND_H: i32 = 9;

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
// Power dialog action.
// ---------------------------------------------------------------------------
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PowerAction {
    Reboot,
    PowerOff,
}

impl PowerAction {
    fn title(&self) -> &'static str {
        match self {
            PowerAction::Reboot => "Reboot?",
            PowerAction::PowerOff => "Power off?",
        }
    }

    fn command(&self) -> (&'static str, &'static [&'static str]) {
        match self {
            PowerAction::Reboot => ("sudo", &["systemctl", "reboot"]),
            PowerAction::PowerOff => ("sudo", &["systemctl", "poweroff"]),
        }
    }
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
    fn new(x: i32, y: i32, fonts: &Fonts) -> Self {
        let label_style = TextStyle::new(11, GRAY, false);
        let value_style = TextStyle::new(16, WHITE, false);
        Self {
            label: Label::new(x + 8, y + 3, label_style, Align::Left, PANEL, fonts),
            value: Label::new(x + 8, y + 19, value_style, Align::Left, PANEL, fonts),
        }
    }

    fn draw_card_background(fb: &mut Framebuffer, x: i32, y: i32) {
        fill_rounded_rect(fb, x, y, HERO_CARD_W, HERO_CARD_H, 4, PANEL);
    }
}

// ---------------------------------------------------------------------------
// NET hero card: label line with up-rate, value line with down-rate.
// ---------------------------------------------------------------------------
struct NetCard {
    x: i32,
    y: i32,
    net_text_width: i32,
    label_text: Label,
    up_rate: Label,
    down_rate: Label,
    last_up: String,
    last_down: String,
    last_up_bbox: Option<Rect>,
    last_down_bbox: Option<Rect>,
}

impl NetCard {
    fn new(x: i32, y: i32, fonts: &Fonts) -> Self {
        let label_style = TextStyle::new(11, GRAY, false).with_weight(FontWeight::Regular);
        let value_style = TextStyle::new(16, WHITE, false);
        let net_text_width = fonts.measure("NET", &label_style) as i32;
        Self {
            x,
            y,
            net_text_width,
            label_text: Label::new(x + 8, y + 3, label_style, Align::Left, PANEL, fonts),
            up_rate: Label::new(0, y + 3, label_style, Align::Left, PANEL, fonts),
            down_rate: Label::new(0, y + 19, value_style, Align::Left, PANEL, fonts),
            last_up: String::new(),
            last_down: String::new(),
            last_up_bbox: None,
            last_down_bbox: None,
        }
    }

    fn draw_static(&mut self, fb: &mut Framebuffer, fonts: &Fonts) {
        HeroCard::draw_card_background(fb, self.x, self.y);
        self.label_text.force_draw(fb, fonts, "NET");
    }

    fn set(&mut self, fb: &mut Framebuffer, fonts: &Fonts, up: f32, down: f32) {
        let up_str = fmt_rate(up);
        let down_str = fmt_rate(down);

        let label_off = self.x + 8 + self.net_text_width + NET_UP_TREND_W + 3;
        self.up_rate.set_x(label_off);

        let down_off = self.x + 8 + NET_DOWN_TREND_W + 3;
        self.down_rate.set_x(down_off);

        if up_str != self.last_up {
            self.erase_up(fb);
            self.draw_up_arrow(fb);
            self.up_rate.force_draw(fb, fonts, &up_str);
            self.last_up = up_str;
            self.last_up_bbox = Some(self.up_bbox());
        }

        if down_str != self.last_down {
            self.erase_down(fb);
            self.draw_down_arrow(fb);
            self.down_rate.force_draw(fb, fonts, &down_str);
            self.last_down = down_str;
            self.last_down_bbox = Some(self.down_bbox());
        }
    }

    fn erase_up(&mut self, fb: &mut Framebuffer) {
        let x = self.up_arrow_x();
        let bbox = Rect::new(
            x as usize,
            (self.y + 3) as usize,
            (x + NET_UP_TREND_W + 60) as usize,
            (self.y + 3 + NET_UP_TREND_H + 4) as usize,
        );
        fill_rect(fb, bbox.x1 as i32, bbox.y1 as i32, bbox.width() as i32, bbox.height() as i32, PANEL);
        if let Some(last) = self.last_up_bbox {
            fill_rect(fb, last.x1 as i32, last.y1 as i32, last.width() as i32, last.height() as i32, PANEL);
        }
    }

    fn erase_down(&mut self, fb: &mut Framebuffer) {
        let x = self.down_arrow_x();
        let bbox = Rect::new(
            x as usize,
            (self.y + 19) as usize,
            (x + NET_DOWN_TREND_W + 80) as usize,
            (self.y + 19 + NET_DOWN_TREND_H + 4) as usize,
        );
        fill_rect(fb, bbox.x1 as i32, bbox.y1 as i32, bbox.width() as i32, bbox.height() as i32, PANEL);
        if let Some(last) = self.last_down_bbox {
            fill_rect(fb, last.x1 as i32, last.y1 as i32, last.width() as i32, last.height() as i32, PANEL);
        }
    }

    fn up_arrow_x(&self) -> i32 {
        self.x + 8 + self.net_text_width + 2 + NET_UP_TREND_W / 2
    }

    fn down_arrow_x(&self) -> i32 {
        self.x + 8 + NET_DOWN_TREND_W / 2
    }

    fn draw_up_arrow(&self, fb: &mut Framebuffer) {
        let cx = self.up_arrow_x();
        let cy = self.y + 3 + NET_UP_TREND_H / 2;
        fill_triangle(fb, cx, cy, NET_UP_TREND_W, NET_UP_TREND_H, true, GRAY);
    }

    fn draw_down_arrow(&self, fb: &mut Framebuffer) {
        let cx = self.down_arrow_x();
        let cy = self.y + 19 + NET_DOWN_TREND_H / 2;
        fill_triangle(fb, cx, cy, NET_DOWN_TREND_W, NET_DOWN_TREND_H, false, CYAN);
    }

    fn up_bbox(&self) -> Rect {
        let ax = self.up_arrow_x() - NET_UP_TREND_W / 2;
        Rect::new(
            ax.max(0) as usize,
            (self.y + 3).max(0) as usize,
            (ax + NET_UP_TREND_W + 60).max(0) as usize,
            (self.y + 3 + NET_UP_TREND_H + 4).max(0) as usize,
        )
    }

    fn down_bbox(&self) -> Rect {
        let ax = self.down_arrow_x() - NET_DOWN_TREND_W / 2;
        Rect::new(
            ax.max(0) as usize,
            (self.y + 19).max(0) as usize,
            (ax + NET_DOWN_TREND_W + 80).max(0) as usize,
            (self.y + 19 + NET_DOWN_TREND_H + 4).max(0) as usize,
        )
    }
}

// ---------------------------------------------------------------------------
// Top-bar chip button (no outline-change animation; outline is static).
// ---------------------------------------------------------------------------
struct TopButton {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    r: i32,
    text: &'static str,
    color: u32,
    text_baseline_y: i32,
    text_x: i32,
}

impl TopButton {
    fn new(x: i32, y: i32, w: i32, h: i32, r: i32, text: &'static str, color: u32, fonts: &Fonts) -> Self {
        let style = TextStyle::new(11, color, false).with_weight(FontWeight::Regular);
        let text_w = fonts.measure(text, &style) as i32;
        let text_x = x + (w - text_w) / 2;
        let text_baseline_y = fonts.baseline_y(y + (h - 11) / 2, &style);
        Self {
            x,
            y,
            w,
            h,
            r,
            text,
            color,
            text_baseline_y,
            text_x,
        }
    }

    fn draw_static(&self, fb: &mut Framebuffer, fonts: &Fonts) {
        fill_rounded_rect(fb, self.x, self.y, self.w, self.h, self.r, PANEL);
        draw_rounded_rect_outline(fb, self.x, self.y, self.w, self.h, self.r, ACCENT);
        let style = TextStyle::new(11, self.color, false).with_weight(FontWeight::Regular);
        fonts.draw(fb, self.text, self.text_x, self.text_baseline_y, &style);
    }

    fn hit(&self, ev: &TouchEvent) -> bool {
        ev.pressed
            && ev.x >= self.x
            && ev.x < self.x + self.w
            && ev.y >= self.y
            && ev.y < self.y + self.h
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
        // Full redraw: track + inset fill.
        fill_rounded_rect(fb, self.x, self.y, self.w, self.h, self.r, self.track_color);
        let fw = self.fill_width(pct_i);
        if fw > 2 {
            fill_rounded_rect(fb, self.x + 1, self.y + 1, fw - 2, self.h - 2, self.r - 1, fill_color);
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
    menu_button: TopButton,
    rst_button: TopButton,
    pwr_button: TopButton,

    // Hero cards
    temp_card: HeroCard,
    mem_card: HeroCard,
    disk_card: HeroCard,
    net_card: NetCard,

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
    load_label: Label,
    fps_label: Label,

    // Temperature trend
    temp_trend_history: VecDeque<(f32, f32)>,
    temp_trend_last_state: TrendState,
    temp_trend_last_alarm: bool,
    temp_trend_last_bbox: Option<Rect>,

    // Power dialog
    power_state: PowerState,
    power_result_rx: Option<std::sync::mpsc::Receiver<Result<(), String>>>,
    pending_full_refresh: bool,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum PowerState {
    Idle,
    Confirming { action: PowerAction, deadline: f32 },
    Executing { action: PowerAction, started: f32 },
    Failed { deadline: f32 },
}

impl MonitorPage {
    pub fn new(fonts: Fonts) -> Self {
        let host_style = TextStyle::new(16, CYAN, false);
        let time_style = TextStyle::new(16, WHITE, false);

        let host_label = Label::new(12, 8, host_style, Align::Left, PANEL, &fonts);
        let time_label = Label::new(TIME_RIGHT, 8, time_style, Align::Right, PANEL, &fonts);
        let ts_chip = TsChip::new(
            TS_CHIP_X,
            TS_CHIP_Y,
            TS_CHIP_W,
            TS_CHIP_H,
            TS_CHIP_R,
            &fonts,
        );
        let menu_button = TopButton::new(
            MENU_CHIP_X,
            MENU_CHIP_Y,
            MENU_CHIP_W,
            MENU_CHIP_H,
            MENU_CHIP_R,
            "MENU",
            GRAY,
            &fonts,
        );
        let rst_button = TopButton::new(
            RST_CHIP_X,
            RST_CHIP_Y,
            RST_CHIP_W,
            RST_CHIP_H,
            RST_CHIP_R,
            "RST",
            WHITE,
            &fonts,
        );
        let pwr_button = TopButton::new(
            PWR_CHIP_X,
            PWR_CHIP_Y,
            PWR_CHIP_W,
            PWR_CHIP_H,
            PWR_CHIP_R,
            "PWR",
            ALARM,
            &fonts,
        );

        let temp_card = HeroCard::new(HERO_CARD_X[0], HERO_CARD_Y, &fonts);
        let mem_card = HeroCard::new(HERO_CARD_X[1], HERO_CARD_Y, &fonts);
        let disk_card = HeroCard::new(HERO_CARD_X[2], HERO_CARD_Y, &fonts);
        let net_card = NetCard::new(HERO_CARD_X[3], HERO_CARD_Y, &fonts);

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
        let header_uptime = Label::new(UPTIME_CENTER, DOCKER_HEADER_Y, header_style, Align::Center, BG, &fonts);
        let header_state = Label::new(STATE_CENTER, DOCKER_HEADER_Y, header_style, Align::Center, BG, &fonts);
        let header_cpu = Label::new(CPU_CENTER, DOCKER_HEADER_Y, header_style, Align::Center, BG, &fonts);

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
                uptime: Label::new(UPTIME_CENTER, y, gray_style, Align::Center, bg, &fonts),
                state: Label::new(STATE_CENTER, y, row_style, Align::Center, bg, &fonts),
                cpu: Label::new(CPU_CENTER, y, row_style, Align::Center, bg, &fonts),
            });
        }

        let page_label = Label::new(PAGE_RIGHT, DOCKER_HEADER_Y, header_style, Align::Right, BG, &fonts);
        let footer_style = TextStyle::new(11, GRAY, false).with_weight(FontWeight::Regular);
        let load_style = TextStyle::new(11, GRAY, false).with_weight(FontWeight::Regular);
        let footer_label = Label::new(12, config::H as i32 - 17, footer_style, Align::Left, PANEL, &fonts);
        let load_label = Label::new(170, config::H as i32 - 17, load_style, Align::Left, PANEL, &fonts);
        let fps_label = Label::new(TIME_RIGHT, config::H as i32 - 17, footer_style, Align::Right, PANEL, &fonts);

        Self {
            fonts,
            bg_done: false,
            last_slow_render: -1000.0,
            start: Instant::now(),
            container_scroll_offset: 0,
            host_label,
            time_label,
            ts_chip,
            menu_button,
            rst_button,
            pwr_button,
            temp_card,
            mem_card,
            disk_card,
            net_card,
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
            load_label,
            fps_label,
            temp_trend_history: VecDeque::new(),
            temp_trend_last_state: TrendState::None,
            temp_trend_last_alarm: false,
            temp_trend_last_bbox: None,
            power_state: PowerState::Idle,
            power_result_rx: None,
            pending_full_refresh: false,
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

        // Top-bar chip buttons.
        self.menu_button.draw_static(fb, &self.fonts);
        self.rst_button.draw_static(fb, &self.fonts);
        self.pwr_button.draw_static(fb, &self.fonts);

        // Hero cards.
        for x in HERO_CARD_X {
            HeroCard::draw_card_background(fb, x, HERO_CARD_Y);
        }
        self.temp_card.label.force_draw(fb, &self.fonts, "TEMP");
        self.mem_card.label.force_draw(fb, &self.fonts, "MEM");
        self.disk_card.label.force_draw(fb, &self.fonts, "DISK");
        self.net_card.draw_static(fb, &self.fonts);

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
        self.load_label.force_draw(fb, &self.fonts, "");
        self.fps_label.force_draw(fb, &self.fonts, "15 FPS");
    }

    // -----------------------------------------------------------------------
    // Slow content (1 Hz).
    // -----------------------------------------------------------------------
    fn draw_slow_content(&mut self, fb: &mut Framebuffer, snapshot: &MetricsSnapshot) {
        self.host_label.set(fb, &self.fonts, hostname());

        let now_str = {
            let dt = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
            format!("{:02}:{:02}:{:02}", dt.hour(), dt.minute(), dt.second())
        };
        self.time_label.set(fb, &self.fonts, &now_str);

        let ts_on = snapshot.tailscale == "ON";
        self.ts_chip.draw(fb, &self.fonts, ts_on);

        // Bottom bar load (center).
        let load = Self::loadavg();
        let load_text = format!("load {}", load);
        self.load_label.set(fb, &self.fonts, &load_text);

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

        self.net_card.set(fb, &self.fonts, snapshot.io.net_up, snapshot.io.net_down);

        // Temperature trend arrow.
        let now = self.now_secs();
        self.update_temp_trend(now, temp_val as f32);
        let (trend_state, trend_alarm) = self.evaluate_temp_trend(now);
        self.draw_temp_value(fb, &snapshot.temp, trend_state, trend_alarm);
    }

    fn loadavg() -> String {
        std::fs::read_to_string("/proc/loadavg")
            .ok()
            .and_then(|s| s.split_whitespace().next().map(|x| x.to_string()))
            .unwrap_or_else(|| "n/a".to_string())
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
    // Docker table (slow path).
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

                let display_name = Self::truncate_to_width(&c.name, NAME_TRUNCATE_WIDTH, &self.fonts, &name_style);
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

    fn draw_temp_value(
        &mut self,
        fb: &mut Framebuffer,
        temp_text: &str,
        state: TrendState,
        alarm: bool,
    ) {
        let value_label = &mut self.temp_card.value;
        let temp_val = parse_temp(temp_text).unwrap_or(50);
        value_label.set_style_color(temp_band_color(temp_val));
        value_label.set(fb, &self.fonts, temp_text);

        if state == TrendState::None {
            if let Some(last) = self.temp_trend_last_bbox {
                fill_rect(fb, last.x1 as i32, last.y1 as i32, last.width() as i32, last.height() as i32, PANEL);
                self.temp_trend_last_bbox = None;
            }
            self.temp_trend_last_state = TrendState::None;
            self.temp_trend_last_alarm = false;
            return;
        }

        let value_bbox = value_label.bbox().unwrap_or(Rect::new(0, 0, 0, 0));
        let trend_x = value_bbox.x2 as i32 + 4 + TREND_W / 2;
        let trend_cy = value_bbox.y1 as i32 + 9;
        let baseline_y = value_label.baseline_y();

        let arrow_bbox = Rect::new(
            (trend_x - TREND_W / 2).max(0) as usize,
            (trend_cy - TREND_H / 2).max(0) as usize,
            (trend_x + TREND_W / 2 + 1).max(0) as usize,
            (trend_cy + TREND_H / 2 + 1).max(0) as usize,
        );
        let alarm_bbox = if alarm {
            let style = TextStyle::new(16, ALARM, false);
            let g = self.fonts.glyph_ref('!', &style);
            let ax = trend_x + TREND_W / 2 + 1 + 4;
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

        let mut new_bbox = arrow_bbox;
        new_bbox = new_bbox.union(&value_bbox);
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

        // Force redraw the value so value + arrow + alarm are rendered atomically
        // in the same frame within the same dirty region.
        value_label.force_draw(fb, &self.fonts, temp_text);

        let arrow_color = match state {
            TrendState::Rising => TREND_HOT,
            TrendState::Falling => COOL,
            TrendState::Steady => GRAY,
            TrendState::None => PANEL,
        };

        match state {
            TrendState::Rising => fill_triangle(fb, trend_x, trend_cy, TREND_W, TREND_H, true, arrow_color),
            TrendState::Falling => fill_triangle(fb, trend_x, trend_cy, TREND_W, TREND_H, false, arrow_color),
            TrendState::Steady => fill_rect(fb, trend_x - TREND_W / 2, trend_cy - 1, TREND_W, 3, arrow_color),
            TrendState::None => {}
        }
        if alarm {
            let ax = trend_x + TREND_W / 2 + 1 + 4;
            self.fonts.draw(fb, "!", ax, baseline_y, &TextStyle::new(16, ALARM, false));
        }

        self.temp_trend_last_state = state;
        self.temp_trend_last_alarm = alarm;
        self.temp_trend_last_bbox = Some(new_bbox);
    }

    // -----------------------------------------------------------------------
    // Power dialog (Phase C).
    // -----------------------------------------------------------------------
    fn enter_power_confirm(&mut self, action: PowerAction) {
        let deadline = self.now_secs() + 10.0;
        self.power_state = PowerState::Confirming { action, deadline };
    }

    fn cancel_power(&mut self) {
        self.power_state = PowerState::Idle;
        self.power_result_rx = None;
        self.bg_done = false;
        self.pending_full_refresh = true;
    }

    fn execute_power(&mut self, action: PowerAction) {
        let started = self.now_secs();
        self.power_state = PowerState::Executing { action, started };

        let (cmd, args) = action.command();
        let (tx, rx) = std::sync::mpsc::channel();
        self.power_result_rx = Some(rx);

        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let result = match tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    tokio::process::Command::new(cmd).args(args).output(),
                )
                .await
                {
                    Ok(Ok(out)) if out.status.success() => Ok(()),
                    Ok(Ok(out)) => Err(String::from_utf8_lossy(&out.stderr).to_string()),
                    Ok(Err(e)) => Err(e.to_string()),
                    Err(_) => Err("timeout".to_string()),
                };
                let _ = tx.send(result);
            });
        } else {
            // No runtime in unit tests: immediately report failure so the UI
            // returns to idle after the 2s failed window.
            let _ = tx.send(Err("no runtime".to_string()));
        }
    }

    fn draw_power_dialog(&mut self, fb: &mut Framebuffer) {
        let title = match self.power_state {
            PowerState::Executing { .. } => "Executing...",
            PowerState::Failed { .. } => "Failed",
            _ => {
                // Confirming: title depends on action.
                if let PowerState::Confirming { action, .. } = self.power_state {
                    action.title()
                } else {
                    ""
                }
            }
        };

        // Dialog background.
        fill_rounded_rect(fb, 90, 105, 300, 110, 6, PANEL);
        draw_rounded_rect_outline(fb, 90, 105, 300, 110, 6, ALARM);

        let title_style = TextStyle::new(16, WHITE, false);
        let title_w = self.fonts.measure(title, &title_style) as i32;
        let title_x = 90 + (300 - title_w) / 2;
        let title_y = self.fonts.baseline_y(120, &title_style);
        self.fonts.draw(fb, title, title_x, title_y, &title_style);

        if let PowerState::Confirming { .. } = self.power_state {
            // CANCEL button.
            fill_rounded_rect(fb, 110, 160, 100, 32, 4, PANEL);
            draw_rounded_rect_outline(fb, 110, 160, 100, 32, 4, GRAY);
            let cancel_style = TextStyle::new(11, WHITE, false).with_weight(FontWeight::Regular);
            let cancel_w = self.fonts.measure("CANCEL", &cancel_style) as i32;
            let cancel_x = 110 + (100 - cancel_w) / 2;
            let cancel_y = self.fonts.baseline_y(160 + (32 - 11) / 2, &cancel_style);
            self.fonts.draw(fb, "CANCEL", cancel_x, cancel_y, &cancel_style);

            // CONFIRM button.
            fill_rounded_rect(fb, 270, 160, 100, 32, 4, PANEL);
            draw_rounded_rect_outline(fb, 270, 160, 100, 32, 4, ALARM);
            let confirm_style = TextStyle::new(11, ALARM, false).with_weight(FontWeight::Regular);
            let confirm_w = self.fonts.measure("CONFIRM", &confirm_style) as i32;
            let confirm_x = 270 + (100 - confirm_w) / 2;
            let confirm_y = self.fonts.baseline_y(160 + (32 - 11) / 2, &confirm_style);
            self.fonts.draw(fb, "CONFIRM", confirm_x, confirm_y, &confirm_style);
        }

        fb.mark_full_dirty();
    }

    fn update_power_state(&mut self) {
        let now = self.now_secs();

        // Check async command result.
        if let PowerState::Executing { started, .. } = self.power_state {
            if let Some(ref rx) = self.power_result_rx {
                match rx.try_recv() {
                    Ok(Ok(())) => {
                        // Command succeeded (real system would reboot/poweroff).
                        // Keep showing Executing...; the host will go down.
                    }
                    Ok(Err(_)) => {
                        self.power_state = PowerState::Failed {
                            deadline: now + 2.0,
                        };
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        // Task dropped: in production this means the machine is
                        // shutting down; in tests treat as success (stay executing).
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        // Still running.
                    }
                }
            }
            // Safety net: if executing lasts > 10s, show failed.
            if now >= started + 10.0 {
                self.power_state = PowerState::Failed {
                    deadline: now + 2.0,
                };
            }
        }

        // Confirm / failed timeouts.
        match self.power_state {
            PowerState::Confirming { deadline, .. } if now >= deadline => self.cancel_power(),
            PowerState::Failed { deadline } if now >= deadline => self.cancel_power(),
            _ => {}
        }
    }

    fn hit_cancel_button(ev: &TouchEvent) -> bool {
        ev.pressed && ev.x >= 110 && ev.x < 210 && ev.y >= 160 && ev.y < 192
    }

    fn hit_confirm_button(ev: &TouchEvent) -> bool {
        ev.pressed && ev.x >= 270 && ev.x < 370 && ev.y >= 160 && ev.y < 192
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

        self.update_power_state();

        let now = self.now_secs();
        if now - self.last_slow_render >= SLOW_RENDER_INTERVAL && matches!(self.power_state, PowerState::Idle) {
            self.draw_slow_content(fb, snapshot);
            self.draw_docker_table(fb, snapshot);
            self.last_slow_render = now;
        }

        self.draw_cpu_pills(fb, &snapshot.cpu);

        if self.pending_full_refresh {
            fb.mark_full_dirty();
            self.pending_full_refresh = false;
        }

        // Power dialog is drawn on top, every frame, because the background
        // behind it is static and the dialog itself has no partial-update needs.
        if !matches!(self.power_state, PowerState::Idle) {
            self.draw_power_dialog(fb);
        }

        Ok(())
    }

    fn on_touch(&mut self, ev: TouchEvent) -> PageAction {
        if !ev.pressed {
            return PageAction::None;
        }

        // Power dialog takes precedence.
        match self.power_state {
            PowerState::Confirming { action, .. } => {
                if Self::hit_confirm_button(&ev) {
                    self.execute_power(action);
                    return PageAction::None;
                }
                if Self::hit_cancel_button(&ev) {
                    self.cancel_power();
                    return PageAction::None;
                }
                // Outside dialog = cancel.
                self.cancel_power();
                return PageAction::None;
            }
            PowerState::Executing { .. } | PowerState::Failed { .. } => {
                // Ignore touches while executing/failed.
                return PageAction::None;
            }
            PowerState::Idle => {}
        }

        // Top bar buttons.
        if self.menu_button.hit(&ev) {
            return PageAction::None;
        }
        if self.rst_button.hit(&ev) {
            self.enter_power_confirm(PowerAction::Reboot);
            return PageAction::None;
        }
        if self.pwr_button.hit(&ev) {
            self.enter_power_confirm(PowerAction::PowerOff);
            return PageAction::None;
        }

        // Hero cards.
        let (x, y) = (ev.x, ev.y);
        if y >= HERO_CARD_Y && y < HERO_CARD_Y + HERO_CARD_H {
            for (idx, card_x) in HERO_CARD_X.iter().enumerate() {
                if x >= *card_x && x < *card_x + HERO_CARD_W {
                    return match idx {
                        0 => PageAction::Switch("temp"),
                        1 => PageAction::Switch("mem"),
                        2 => PageAction::Switch("disk"),
                        3 => PageAction::Switch("net"),
                        _ => PageAction::None,
                    };
                }
            }
        }

        // CPU area.
        let (cpu_x1, cpu_y1, cpu_x2, cpu_y2) = CPU_AREA;
        if x >= cpu_x1 && x < cpu_x2 && y >= cpu_y1 && y < cpu_y2 {
            return PageAction::Switch("cpu");
        }

        // Docker list scroll.
        if y >= DOCKER_LIST_Y && y < DOCKER_LIST_Y + CONTAINER_PAGE_SIZE as i32 * DOCKER_LINE_HEIGHT {
            self.container_scroll_offset += 1;
        }

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
        self.load_label.clear(fb);
        self.power_state = PowerState::Idle;
        self.power_result_rx = None;
        self.pending_full_refresh = false;
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
    fn touch_hero_temp_switches_page() {
        let mut page = make_page();
        let ev = TouchEvent {
            x: HERO_CARD_X[0] + 10,
            y: HERO_CARD_Y + 10,
            pressed: true,
            timestamp: 0.0,
        };
        assert_eq!(page.on_touch(ev), PageAction::Switch("temp"));
    }

    #[test]
    fn touch_cpu_area_switches_cpu_page() {
        let mut page = make_page();
        let ev = TouchEvent { x: 100, y: 100, pressed: true, timestamp: 0.0 };
        assert_eq!(page.on_touch(ev), PageAction::Switch("cpu"));
    }

    #[test]
    fn touch_menu_noop() {
        let mut page = make_page();
        let ev = TouchEvent {
            x: MENU_CHIP_X + 5,
            y: MENU_CHIP_Y + 5,
            pressed: true,
            timestamp: 0.0,
        };
        assert_eq!(page.on_touch(ev), PageAction::None);
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
            io: Default::default(),
            history: Default::default(),
        };
        page.render(&mut fb, &snapshot).unwrap();
        page.render(&mut fb, &snapshot).unwrap();
        let png = encode_png(fb.buffer(), crate::config::W, crate::config::H).expect("png encode");
        assert!(!png.is_empty());
        let path = "/tmp/pi_dashboard_golden_rust.png";
        std::fs::write(path, png).expect("write png");
    }

    #[test]
    fn touch_pwr_opens_confirm_dialog() {
        let mut page = make_page();
        let ev = TouchEvent {
            x: PWR_CHIP_X + 5,
            y: PWR_CHIP_Y + 5,
            pressed: true,
            timestamp: 0.0,
        };
        assert_eq!(page.on_touch(ev), PageAction::None);
        assert!(
            matches!(page.power_state, PowerState::Confirming { action: PowerAction::PowerOff, .. }),
            "expected power-off confirm dialog"
        );
    }

    #[test]
    fn touch_rst_opens_confirm_dialog() {
        let mut page = make_page();
        let ev = TouchEvent {
            x: RST_CHIP_X + 5,
            y: RST_CHIP_Y + 5,
            pressed: true,
            timestamp: 0.0,
        };
        assert_eq!(page.on_touch(ev), PageAction::None);
        assert!(
            matches!(page.power_state, PowerState::Confirming { action: PowerAction::Reboot, .. }),
            "expected reboot confirm dialog"
        );
    }

    #[test]
    fn power_dialog_cancel_returns_to_idle() {
        let mut page = make_page();
        page.enter_power_confirm(PowerAction::Reboot);
        let ev = TouchEvent {
            x: 155, // inside CANCEL button
            y: 175,
            pressed: true,
            timestamp: 0.0,
        };
        assert_eq!(page.on_touch(ev), PageAction::None);
        assert!(matches!(page.power_state, PowerState::Idle));
        assert!(page.pending_full_refresh);
    }

    #[test]
    fn power_dialog_outside_tap_cancels() {
        let mut page = make_page();
        page.enter_power_confirm(PowerAction::PowerOff);
        let ev = TouchEvent {
            x: 50,
            y: 50,
            pressed: true,
            timestamp: 0.0,
        };
        assert_eq!(page.on_touch(ev), PageAction::None);
        assert!(matches!(page.power_state, PowerState::Idle));
    }

    #[test]
    fn power_confirm_executes_and_returns_via_failed() {
        let mut page = make_page();
        let mut fb = Framebuffer::headless();
        page.enter_power_confirm(PowerAction::Reboot);

        let confirm = TouchEvent {
            x: 320, // inside CONFIRM button
            y: 175,
            pressed: true,
            timestamp: 0.0,
        };
        assert_eq!(page.on_touch(confirm), PageAction::None);
        assert!(matches!(page.power_state, PowerState::Executing { .. }));

        // In unit tests there is no tokio runtime, so the command reports
        // "no runtime" immediately and the UI moves to Failed for 2s.
        page.render(&mut fb, &MetricsSnapshot::default()).unwrap();
        assert!(matches!(page.power_state, PowerState::Failed { .. }));

        // Advance past the 2s failed window by sleeping.
        std::thread::sleep(std::time::Duration::from_millis(2100));
        page.render(&mut fb, &MetricsSnapshot::default()).unwrap();
        assert!(matches!(page.power_state, PowerState::Idle));
    }

    #[test]
    fn cpu_bar_fill_inset_preserves_outline() {
        use crate::render::rgb888_to_rgb565;

        let mut fb = Framebuffer::headless();
        let mut bar = RoundedBar::new(10, 10, 20, 10, 5, 0x00ff00);
        // fw = 19 (95% of 20); exercise the inset fill path.
        bar.set(&mut fb, 95.0, 0xff0000);
        let buf = fb.buffer();
        let fill = rgb888_to_rgb565(0xff0000);
        let track = rgb888_to_rgb565(0x00ff00);

        // a) Fill pixels must stay within x+1 .. x+w-1.
        for row in 10..20 {
            let start = row * crate::config::W;
            for col in 0..crate::config::W {
                if buf[start + col] == fill {
                    assert!(
                        col >= 11 && col <= 28,
                        "fill pixel overflows track outline at ({}, {})",
                        col,
                        row
                    );
                }
            }
        }

        // b) Track left edge corner outline (inset 5/2/1 rows) is not covered.
        // For r=5 at row 10/11/12 the leftmost track pixels are at x=15/12/11.
        for (row, col) in [(10, 15), (11, 12), (12, 11)] {
            let idx = row * crate::config::W + col;
            assert_eq!(
                buf[idx], track,
                "track left edge overwritten at row {} col {}",
                row, col
            );
        }

        // c) Left/right fill insets are symmetric per row (relative to the fill's own bounds).
        let fill_left = 11;
        let fill_right = 11 + 17 - 1; // x + w - 1 for the inset fill rect
        for row in 10..20 {
            let start = row * crate::config::W;
            let left = (0..crate::config::W).position(|c| buf[start + c] == fill);
            let right = (0..crate::config::W).rposition(|c| buf[start + c] == fill);
            if let (Some(l), Some(r)) = (left, right) {
                let left_inset = l - fill_left;
                let right_inset = fill_right - r;
                assert_eq!(
                    left_inset, right_inset,
                    "asymmetric fill at row {}: left_inset={}, right_inset={}",
                    row, left_inset, right_inset
                );
            }
        }
    }
}

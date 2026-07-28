from __future__ import annotations

import logging
import re
import time
from datetime import datetime
from typing import Any, List, Tuple

from PIL import Image, ImageDraw

from .config import (
    COLORS,
    CONTAINER_PAGE_SIZE,
    DOCKER_HEADER_Y,
    DOCKER_LINE_HEIGHT,
    DOCKER_LIST_Y,
    DOCKER_START_Y,
    REFRESH_INTERVAL,
    SLOW_DATA_INTERVAL,
    SLOW_RENDER_INTERVAL,
    SWITCH_HOTZONE_SIZE,
    W,
    H,
)
from .fonts import get_font
from .metrics import (
    CpuSampler,
    get_ip_list,
    read_cpu_temp,
    read_disk_usage,
    read_docker_containers,
    read_mem_info,
    read_tailscale_status,
)
from .panel import Mode
from .render import TEMP_COLOR_LUT, USAGE_COLOR_LUT
from .touch import TouchEvent

logger = logging.getLogger("pi_dashboard.monitor")


def _usage_color(text: str) -> str:
    """从 '1234/4096MB (50%)' 或 '45%' 中取百分比查渐变色表；解析失败用灰色。"""
    m = re.search(r"(\d+(?:\.\d+)?)\s*%", text)
    if not m:
        return COLORS["gray"]
    pct = max(0, min(100, int(float(m.group(1)))))
    return USAGE_COLOR_LUT[pct]


def _temp_color(text: str) -> str:
    """从 '42C' 中取温度查渐变色表；解析失败用灰色。"""
    m = re.match(r"(\d+(?:\.\d+)?)", text)
    if not m:
        return COLORS["gray"]
    t = max(0, min(127, int(float(m.group(1)))))
    return TEMP_COLOR_LUT[t]


class MonitorMode(Mode):
    name = "monitor"

    def __init__(self) -> None:
        self._cpu_sampler = CpuSampler()
        self.container_scroll_offset = 0
        self._last_fast_collect: float = 0.0
        self._last_slow_collect: float = 0.0

        self.now: str = ""
        self.ip_list: List[str] = []
        self.temp: str = ""
        self.cpu: dict[str, float] = {}
        self.mem: str = ""
        self.disk: str = ""
        self.containers: List[Tuple[str, str, str]] = []
        self.tailscale: str = ""

        # 静态背景缓存：布局不变，只画一次
        self._bg_cache: Image.Image | None = None
        self._bg_meta: dict[str, Any] = {}
        # 慢变内容缓存：除 CPU 占用条外，其余动态内容每 2 秒才重画一次
        self._slow_cache: Image.Image | None = None
        self._last_slow_render: float = 0.0

    def on_activate(self) -> None:
        logger.info("Monitor mode activated")
        self._bg_cache = None
        self._bg_meta = {}
        self._slow_cache = None
        self._last_slow_render = 0.0
        self.collect_fast_data()
        self.collect_slow_data()

    def on_deactivate(self) -> None:
        logger.info("Monitor mode deactivated")

    def collect_fast_data(self) -> None:
        # 快速路径：/proc、/sys 纯文件读取，每帧可安全执行
        self.now = datetime.now().strftime("%H:%M:%S")
        self.temp = read_cpu_temp()
        self.cpu = self._cpu_sampler.read()
        self.mem = read_mem_info()
        self.disk = read_disk_usage()

    def collect_slow_data(self) -> None:
        # 慢速路径：涉及子进程 / 网络，降低频率避免拖垮帧率
        self.ip_list = get_ip_list()
        self.containers = read_docker_containers()
        self.tailscale = read_tailscale_status()

    def on_tick(self) -> None:
        now = time.monotonic()
        if now - self._last_fast_collect >= REFRESH_INTERVAL:
            self.collect_fast_data()
            self._last_fast_collect = now
        if now - self._last_slow_collect >= SLOW_DATA_INTERVAL:
            self.collect_slow_data()
            self._last_slow_collect = now
            # 慢数据变了，下一次 render 要重建慢变内容缓存
            self._slow_cache = None

    def handle_touch(self, event: TouchEvent) -> bool:
        if not event.pressed:
            return False
        x, y = event.x, event.y
        if y < DOCKER_START_Y or y > DOCKER_START_Y + CONTAINER_PAGE_SIZE * DOCKER_LINE_HEIGHT + 20:
            return False

        total = len(self.containers)
        max_offset = max(0, total - CONTAINER_PAGE_SIZE)
        if max_offset <= 0:
            return True

        if y < DOCKER_LIST_Y:
            return True

        self.container_scroll_offset += 1
        if self.container_scroll_offset > max_offset:
            self.container_scroll_offset = 0

        return True

    def _build_background(self) -> Image.Image:
        """绘制不随数据变化的静态背景，缓存后每帧复用。"""
        img = Image.new("RGB", (W, H), COLORS["bg"])
        draw = ImageDraw.Draw(img)

        f_text = get_font(13)
        f_small = get_font(11)
        f_tiny = get_font(10)

        # 顶部标题栏背景
        draw.rectangle([0, 0, W, 30], fill=COLORS["panel"])

        # 右上角模式切换按钮（完全静态）
        btn_x = W - SWITCH_HOTZONE_SIZE
        draw.rectangle([btn_x, 4, W, 26], fill="#21262d", outline=COLORS["accent"])
        btn_text = ">_ CONSOLE"
        btn_tw = draw.textbbox((0, 0), btn_text, font=f_small)[2]
        draw.text(
            (btn_x + (SWITCH_HOTZONE_SIZE - btn_tw) // 2, 8),
            btn_text,
            font=f_small,
            fill=COLORS["yellow"],
        )

        # CPU 占用条：标签与背景条（填充和百分比是动态的）
        core_positions = [(8, 51), (248, 51), (8, 68), (248, 68)]
        self._bg_meta["core_bars"] = []
        for idx, (x, y) in enumerate(core_positions):
            label = f"CPU{idx}"
            draw.text((x, y), label, font=f_text, fill=COLORS["gray"])
            bar_x = x + 44
            bar_w = 130
            draw.rectangle([bar_x, y + 3, bar_x + bar_w, y + 12], fill=COLORS["accent"])
            self._bg_meta["core_bars"].append((x, y, bar_x, bar_w))

        # 温度/内存/磁盘标签（数值动态），记录数值起始 x 坐标
        # 标签用灰色弱化，让渐变色的数值成为视觉焦点
        metrics = [("TEMP", 8, 85), ("MEM", 126, 85), ("DISK", 360, 85)]
        self._bg_meta["metric_x"] = {}
        for label, x, y in metrics:
            text = f"{label} "
            draw.text((x, y), text, font=f_text, fill=COLORS["gray"])
            self._bg_meta["metric_x"][label] = draw.textbbox((x, y), text, font=f_text)[2]

        draw.line([0, 100, W, 100], fill=COLORS["accent"], width=1)

        # 容器列表表头（name 列加宽，status 列居中）
        y = DOCKER_HEADER_Y
        draw.text((8, y), "CONTAINER", font=f_small, fill=COLORS["gray"])
        draw.text((175, y), "STATUS", font=f_small, fill=COLORS["gray"])
        draw.text((360, y), "STATE", font=f_small, fill=COLORS["gray"])

        # 底部状态栏
        draw.rectangle([0, H - 20, W, H], fill=COLORS["panel"])
        draw.text((8, H - 18), "Powered by RichLi4307", font=f_tiny, fill=COLORS["gray"])

        return img

    def _render_slow(self, draw: ImageDraw.ImageDraw) -> None:
        """绘制除 CPU 占用条外的慢变内容（0.5 Hz）。"""
        f_title = get_font(16)
        f_text = get_font(13)
        f_small = get_font(11)
        f_tiny = get_font(10)

        # 顶部动态信息：主机名青色、时间白色，拉开层级
        draw.text((8, 6), "FocusRasPi4B", font=f_title, fill=COLORS["cyan"])
        host_w = draw.textbbox((8, 6), "FocusRasPi4B", font=f_title)[2]
        draw.text((host_w + 10, 6), self.now, font=f_title, fill=COLORS["white"])
        ts_color = COLORS["green"] if self.tailscale == "ON" else COLORS["red"]
        draw.text((295, 8), f"TS:{self.tailscale}", font=f_small, fill=ts_color)

        ip_str = "             ".join(self.ip_list[:3])
        draw.text((8, 34), f"IP {ip_str}", font=f_text, fill=COLORS["cyan"])

        # 温度/内存/磁盘数值：按查表渐变着色，解析失败回落灰色
        metric_x = self._bg_meta["metric_x"]
        draw.text((metric_x["TEMP"], 85), self.temp, font=f_text, fill=_temp_color(self.temp))
        draw.text((metric_x["MEM"], 85), self.mem, font=f_text, fill=_usage_color(self.mem))
        draw.text((metric_x["DISK"], 85), self.disk, font=f_text, fill=_usage_color(self.disk))

        # 容器列表（行首加状态色圆点，name 列右移让位）
        total = len(self.containers)
        max_offset = max(0, total - CONTAINER_PAGE_SIZE)
        if self.container_scroll_offset > max_offset:
            self.container_scroll_offset = max_offset

        offset = self.container_scroll_offset
        visible = self.containers[offset: offset + CONTAINER_PAGE_SIZE]

        y = DOCKER_LIST_Y
        for name, status, state in visible:
            if state == "running":
                color = COLORS["green"]
            elif state == "exited":
                color = COLORS["red"]
            else:
                color = COLORS["yellow"]
            draw.ellipse([8, y + 3, 14, y + 9], fill=color)
            draw.text((20, y), name, font=f_tiny, fill=COLORS["white"])
            draw.text((175, y), status, font=f_tiny, fill=COLORS["gray"])
            draw.text((360, y), state, font=f_tiny, fill=color)
            y += DOCKER_LINE_HEIGHT

        total_pages = max(1, (total + CONTAINER_PAGE_SIZE - 1) // CONTAINER_PAGE_SIZE)
        current_page = (offset // CONTAINER_PAGE_SIZE) + 1
        if total_pages > 1:
            draw.text((420, DOCKER_LIST_Y), f"{current_page}/{total_pages}", font=f_tiny, fill=COLORS["yellow"])

        fps = int(round(1.0 / REFRESH_INTERVAL)) if REFRESH_INTERVAL > 0 else 1
        draw.text((340, H - 18), f"{fps} FPS", font=f_tiny, fill=COLORS["gray"])

    def _render_cpu_bars(self, draw: ImageDraw.ImageDraw) -> None:
        """绘制 CPU 占用条（5 Hz），颜色直接查预插值表，无逐帧计算。"""
        f_text = get_font(13)

        for idx, (x, y, bar_x, bar_w) in enumerate(self._bg_meta["core_bars"]):
            core = f"cpu{idx}"
            pct = self.cpu.get(core, 0.0)
            color = USAGE_COLOR_LUT[max(0, min(100, int(pct)))]
            fill_w = int(bar_w * pct / 100.0)
            if fill_w > 0:
                draw.rectangle([bar_x, y + 3, bar_x + fill_w, y + 12], fill=color)
            draw.text((bar_x + bar_w + 6, y), f"{pct:.0f}%", font=f_text, fill=color)

    def render(self) -> Image.Image:
        if self._bg_cache is None:
            self._bg_cache = self._build_background()

        now = time.monotonic()
        # 慢变内容每 2 秒重建一次；慢数据更新时也会把 _slow_cache 置空
        if self._slow_cache is None or (now - self._last_slow_render) >= SLOW_RENDER_INTERVAL:
            slow_img = self._bg_cache.copy()
            slow_draw = ImageDraw.Draw(slow_img)
            self._render_slow(slow_draw)
            self._slow_cache = slow_img
            self._last_slow_render = now

        # 每帧在慢变缓存基础上只重绘 CPU 条
        img = self._slow_cache.copy()
        draw = ImageDraw.Draw(img)
        self._render_cpu_bars(draw)
        return img

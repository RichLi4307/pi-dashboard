from __future__ import annotations

import logging
import os
import time

import numpy as np
from PIL import Image, ImageDraw

from .config import COLORS, FB, TEMP_GRADIENT, TEMP_RANGE, USAGE_GRADIENT, W, H
from .fonts import get_font

logger = logging.getLogger("pi_dashboard.render")


def gradient_color(ratio: float, stops: list[tuple[float, str]]) -> str:
    """按 0~1 比例在多段色标间线性插值，返回 #rrggbb。

    stops 为 [(位置, 颜色), ...]，位置单调递增且覆盖 0 和 1。
    只在模块加载时用于构建查找表，运行期渲染直接查表，零计算开销。
    """
    ratio = max(0.0, min(1.0, ratio))
    prev_pos, prev_hex = stops[0]
    for pos, hexs in stops[1:]:
        if ratio <= pos:
            if pos <= prev_pos:
                return hexs
            t = (ratio - prev_pos) / (pos - prev_pos)
            r0, g0, b0 = (int(prev_hex[i:i + 2], 16) for i in (1, 3, 5))
            r1, g1, b1 = (int(hexs[i:i + 2], 16) for i in (1, 3, 5))
            r = int(r0 + (r1 - r0) * t)
            g = int(g0 + (g1 - g0) * t)
            b = int(b0 + (b1 - b0) * t)
            return f"#{r:02x}{g:02x}{b:02x}"
        prev_pos, prev_hex = pos, hexs
    return stops[-1][1]


# 预插值查找表：加载时一次算好，运行期只做一次列表索引，不给 ARM 小核添负担
# 用量百分比 0~100% → 颜色（CPU 条 / 内存 / 磁盘共用）
USAGE_COLOR_LUT = [gradient_color(p / 100.0, USAGE_GRADIENT) for p in range(101)]
# 温度 0~127°C → 颜色（超出范围钳位到端点色）
_temp_lo, _temp_hi = TEMP_RANGE
TEMP_COLOR_LUT = [
    gradient_color((t - _temp_lo) / (_temp_hi - _temp_lo), TEMP_GRADIENT)
    for t in range(128)
]


def blit(img: Image.Image) -> None:
    # 用 uint8 读取避免一次性展开 uint16，再分通道原地计算 RGB565，
    # 比原始写法减少中间数组分配，实测每帧快约 1 ms。
    arr = np.array(img, dtype=np.uint8)
    r = arr[:, :, 0].astype(np.uint16)
    g = arr[:, :, 1].astype(np.uint16)
    b = arr[:, :, 2].astype(np.uint16)
    np.bitwise_and(r, 0xF8, out=r)
    np.left_shift(r, 8, out=r)
    np.bitwise_and(g, 0xFC, out=g)
    np.left_shift(g, 3, out=g)
    np.right_shift(b, 3, out=b)
    rgb565 = r
    np.bitwise_or(rgb565, g, out=rgb565)
    np.bitwise_or(rgb565, b, out=rgb565)
    with open(FB, "wb") as fh:
        fh.write(rgb565.astype("<u2").tobytes())


def clear_framebuffer() -> None:
    try:
        with open(FB, "wb") as fh:
            fh.write(b"\x00" * (W * H * 2))
    except OSError as exc:
        logger.warning("Failed to clear framebuffer: %s", exc)


def wait_fb(timeout: int = 30) -> bool:
    for _ in range(timeout * 2):
        if os.path.exists(FB):
            return True
        time.sleep(0.5)
    logger.error("Framebuffer %s not available after %ds", FB, timeout)
    return False


def render_boot_screen() -> Image.Image:
    img = Image.new("RGB", (W, H), COLORS["bg"])
    draw = ImageDraw.Draw(img)
    draw.text((W // 2 - 90, H // 2 - 30), "System Booting...", font=get_font(18), fill=COLORS["white"])
    draw.text((W // 2 - 80, H // 2), "Waiting for Docker", font=get_font(14), fill=COLORS["gray"])
    draw.text((W // 2 - 40, H // 2 + 25), "Please wait", font=get_font(12), fill=COLORS["gray"])
    return img


# 在画面中央绘制一个半透明浮层，显示切换提示文字并保持 duration 秒
# 用于 mode 切换后提示 "Switch to Monitor Mode" 或 "Switch to Console Mode"
def render_overlay(base_img: Image.Image, text: str, duration: float = 1.0) -> None:
    draw = ImageDraw.Draw(base_img)
    # 计算文字包围盒，确定浮层尺寸
    bbox = draw.textbbox((0, 0), text, font=get_font(14))
    tw = bbox[2] - bbox[0]
    th = bbox[3] - bbox[1]
    # 浮层居中于屏幕，上下左右各留 10px 内边距
    ox = (W - tw) // 2 - 10
    oy = (H - th) // 2 - 6
    # 绘制深灰色背景矩形
    draw.rectangle([ox, oy, ox + tw + 20, oy + th + 12], fill="#333333")
    # 绘制白色文字
    draw.text((ox + 10, oy + 6), text, font=get_font(14), fill=COLORS["white"])
    blit(base_img)
    time.sleep(duration)  # 保持浮层显示指定时间后返回

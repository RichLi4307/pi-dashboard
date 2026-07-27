from __future__ import annotations

import logging
import os
import struct
import threading
from collections import namedtuple
from queue import Queue

from .config import TOUCH_DEVICES

logger = logging.getLogger("pi_dashboard.touch")

# 触摸事件结构：x(屏幕坐标)、y(屏幕坐标)、pressed(是否按下)、timestamp(事件时间戳)
TouchEvent = namedtuple("TouchEvent", ["x", "y", "pressed", "timestamp"])

# Linux input_event 结构体 (64位 aarch64):  struct input_event { __le32 sec, __le32 usec, __le16 type, __le16 code, __le32 value }
EVENT_STRUCT = struct.Struct("llHHI")
EVENT_SIZE = EVENT_STRUCT.size

# Linux 输入子系统事件类型常量
EV_KEY = 1       # 按键事件（如触摸按下/抬起）
EV_ABS = 3       # 绝对坐标事件（如触摸 X/Y 坐标）
BTN_TOUCH = 330  # 触摸按键代码
ABS_X = 0        # X 轴绝对坐标
ABS_Y = 1        # Y 轴绝对坐标

# 校准矩阵文件路径（备选方案，当前使用系统级虚拟设备 touch-fix 替代）
CAL_MATRIX_PATH = "/etc/pointercal"


# 加载 /etc/pointercal 校准矩阵文件（7 个浮点数：a b c d e f s）
# 返回 None 表示无校准文件，代码将使用原始坐标直接输出
def _load_calibration() -> list[float] | None:
    try:
        with open(CAL_MATRIX_PATH, "r") as fh:
            parts = fh.read().strip().split()
            if len(parts) >= 7:
                return [float(p) for p in parts[:7]]
    except (OSError, ValueError, IndexError):
        pass
    return None


# 应用校准矩阵将原始 ADC 坐标映射到屏幕坐标
# 映射公式：sx = (a*raw_x + b*raw_y + c)/s,  sy = (d*raw_x + e*raw_y + f)/s
# 最终坐标被限制在 480×320 屏幕范围内
def _apply_calibration(raw_x: int, raw_y: int, cal: list[float] | None) -> tuple[int, int]:
    if cal is None or len(cal) < 7:
        # 无校准文件时直接传递原始坐标（虚拟设备 touch-fix 已预先映射）
        sx, sy = raw_x, raw_y
        return max(0, min(479, sx)), max(0, min(319, sy))
    a, b, c, d, e, f, s = cal[:7]
    sx = int((a * raw_x + b * raw_y + c) / s)
    sy = int((d * raw_x + e * raw_y + f) / s)
    return max(0, min(479, sx)), max(0, min(319, sy))


# 从配置的触摸设备列表中查找第一个可用的设备
# 正常配置应优先使用虚拟设备 /dev/input/event1（已校准），回退到原始设备 event0
def _find_touch_device() -> str | None:
    for dev in TOUCH_DEVICES:
        if os.path.exists(dev):
            return dev
    return None


# 【触摸读取线程入口】持续从输入设备读取原始事件，解析后送入事件队列
# 面板主循环（panel.run）从队列中消费触摸事件并分发给各模式
def touch_reader_thread(event_queue: Queue, stop_event: threading.Event) -> None:
    dev_path = _find_touch_device()
    if dev_path is None:
        logger.error("No touch device found among %s", TOUCH_DEVICES)
        return

    cal = _load_calibration()

    try:
        with open(dev_path, "rb") as fh:
            raw_x, raw_y = 0, 0
            pressed = False

            while not stop_event.is_set():
                # 从设备文件读取一个 input_event 结构体（16 字节）
                data = fh.read(EVENT_SIZE)
                if not data or len(data) < EVENT_SIZE:
                    continue

                # 解析二进制事件数据
                sec, usec, ev_type, code, value = EVENT_STRUCT.unpack(data)
                timestamp = sec + usec / 1_000_000

                # 处理触摸按下/抬起事件
                if ev_type == EV_KEY and code == BTN_TOUCH:
                    pressed = bool(value)
                    # 抬起时上报抬起事件，带最后记录的坐标
                    if not pressed:
                        event_queue.put(TouchEvent(raw_x, raw_y, False, timestamp))

                # 处理绝对坐标事件（X/Y 轴）
                elif ev_type == EV_ABS:
                    if code == ABS_X:
                        raw_x = value
                    elif code == ABS_Y:
                        raw_y = value
                    # 手指按下期间实时上报坐标事件（每次坐标变化都推送）
                    if pressed:
                        sx, sy = _apply_calibration(raw_x, raw_y, cal)
                        event_queue.put(TouchEvent(sx, sy, True, timestamp))

    except OSError as exc:
        logger.error("Touch device read error: %s", exc)

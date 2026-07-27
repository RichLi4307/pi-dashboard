from __future__ import annotations

import fcntl
import logging
import os
import pty
import select
import struct
import threading
import time
from queue import Queue, Empty

from PIL import Image, ImageDraw

from .render import blit
from .config import COLORS, SWITCH_HOTZONE_SIZE, W, H
from .fonts import get_font
from .panel import Mode
from .touch import TouchEvent

logger = logging.getLogger("pi_dashboard.console")

CONSOLE_BG = "#0a0a0a"
CONSOLE_GREEN = "#00ff66"
CONSOLE_WHITE = "#cccccc"
CONSOLE_GRAY = "#555555"
STATUS_BAR_H = 18
FOOTER_H = 20
FONT_SIZE = 12
LINE_H = 15
MAX_LINES = (H - STATUS_BAR_H - FOOTER_H) // LINE_H

# Linux input_event 结构体 (同 touch.py)
EVENT_STRUCT = struct.Struct("llHHI")
EVENT_SIZE = EVENT_STRUCT.size

EV_KEY = 1
EV_SYN = 0

# ioctl 常量 (linux/termios.h) - 设置 PTY 窗口大小
TIOCSWINSZ = 0x5414

# QWERTY 无 Shift 映射：linux/input-event-codes.h 中的 KEY_* 码 → ASCII
_KEY_NORMAL: dict[int, str] = {
    2: "1", 3: "2", 4: "3", 5: "4", 6: "5", 7: "6", 8: "7",
    9: "8", 10: "9", 11: "0",
    12: "-", 13: "=",
    16: "q", 17: "w", 18: "e", 19: "r", 20: "t", 21: "y",
    22: "u", 23: "i", 24: "o", 25: "p",
    26: "[", 27: "]",
    30: "a", 31: "s", 32: "d", 33: "f", 34: "g", 35: "h",
    36: "j", 37: "k", 38: "l",
    39: ";", 40: "'", 41: "`",
    43: "\\",
    44: "z", 45: "x", 46: "c", 47: "v", 48: "b", 49: "n", 50: "m",
    51: ",", 52: ".", 53: "/",
    57: " ",  # SPACE
}
# QWERTY Shift 映射
_KEY_SHIFT: dict[int, str] = {
    2: "!", 3: "@", 4: "#", 5: "$", 6: "%", 7: "^", 8: "&",
    9: "*", 10: "(", 11: ")",
    12: "_", 13: "+",
    16: "Q", 17: "W", 18: "E", 19: "R", 20: "T", 21: "Y",
    22: "U", 23: "I", 24: "O", 25: "P",
    26: "{", 27: "}",
    30: "A", 31: "S", 32: "D", 33: "F", 34: "G", 35: "H",
    36: "J", 37: "K", 38: "L",
    39: ":", 40: '"', 41: "~",
    43: "|",
    44: "Z", 45: "X", 46: "C", 47: "V", 48: "B", 49: "N", 50: "M",
    51: "<", 52: ">", 53: "?",
    57: " ",
}
# 功能键映射（无需 Shift）
_KEY_SPECIAL: dict[int, str] = {
    1: "\x1b",      # ESC
    15: "\t",       # TAB
    111: "\x1b[3~", # DELETE
}
# 方向键 / 编辑键序列（无需 Shift）
_KEY_SEQUENCES: dict[int, str] = {
    103: "\x1b[A",  # UP
    108: "\x1b[B",  # DOWN
    105: "\x1b[C",  # RIGHT
    106: "\x1b[D",  # LEFT
    102: "\x1b[H",  # HOME
    107: "\x1b[F",  # END
    104: "\x1b[5~", # PGUP
    109: "\x1b[6~", # PGDN
}

# 特殊按键码
KEY_ENTER = 28
KEY_BACKSPACE = 14
KEY_LEFTCTRL = 29
KEY_RIGHTSHIFT = 54
KEY_LEFTSHIFT = 42


class ConsoleMode(Mode):
    name = "console"

    # 每行最大字符数（等宽字体 12px，屏幕宽 480px，约 66 字符）
    COLS = 66

    def __init__(self) -> None:
        self._lines: list[str] = []
        self._buf = ""
        self._child_fd: int | None = None
        self._lock = threading.Lock()
        self._running = False
        self._reader_thread: threading.Thread | None = None

        self._kbd_queue: Queue = Queue()
        self._kbd_stop = threading.Event()
        self._kbd_thread: threading.Thread | None = None

    def _word_wrap(self, text: str) -> list[str]:
        """将长行按 COLS 自动换行，保留原始行末换行逻辑"""
        if len(text) <= self.COLS:
            return [text]
        wrapped = []
        for i in range(0, len(text), self.COLS):
            wrapped.append(text[i:i + self.COLS])
        return wrapped

    def _reader(self) -> None:
        while self._running and self._child_fd is not None:
            r, _, _ = select.select([self._child_fd], [], [], 0.1)
            if r:
                try:
                    data = os.read(self._child_fd, 4096)
                    if not data:
                        break
                    text = data.decode("utf-8", errors="replace")
                    with self._lock:
                        self._buf += text
                        while "\n" in self._buf:
                            idx = self._buf.index("\n")
                            line = self._buf[:idx]
                            self._buf = self._buf[idx + 1:]
                            self._lines.extend(self._word_wrap(line))
                            if len(self._lines) > MAX_LINES:
                                self._lines = self._lines[-MAX_LINES:]
                except OSError:
                    break

    @staticmethod
    def _find_keyboard_device() -> str | None:
        candidates = []
        for i in range(32):
            dev = f"/dev/input/event{i}"
            if not os.path.exists(dev):
                continue
            try:
                with open(f"/sys/class/input/event{i}/device/name", "r") as f:
                    name = f.read().strip().lower()

                with open(f"/sys/class/input/event{i}/device/capabilities/ev", "r") as f:
                    ev_str = f.read().strip()
                ev_parts = ev_str.split()
                ev_int = int(ev_parts[0], 16) if ev_parts else 0
                if not (ev_int & (1 << EV_KEY)):
                    continue

                with open(f"/sys/class/input/event{i}/device/capabilities/key", "r") as f:
                    key_str = f.read().strip()
                key_parts = key_str.split()
                if not key_parts:
                    continue
                key_int = int(key_parts[0], 16)

                has_letter = ((key_int >> 16) & 1) or ((key_int >> 30) & 1)
                if not has_letter:
                    continue

                # 优先选择名字明确包含 "keyboard" 的设备
                if "keyboard" in name:
                    logger.info("Found keyboard at %s (name: %s)", dev, name)
                    return dev
                # 排除明显的非键盘设备
                if any(x in name for x in ("mouse", "consumer", "system control", "touchscreen")):
                    continue

                candidates.append((dev, name))
            except (OSError, ValueError) as exc:
                logger.debug("Skip event%d: %s", i, exc)
                continue

        if candidates:
            dev, name = candidates[0]
            logger.info("Found keyboard at %s (name: %s)", dev, name)
            return dev
        return None

    def _kbd_reader_thread(self) -> None:
        mod_state: dict[int, bool] = {
            KEY_LEFTCTRL: False,
            KEY_LEFTSHIFT: False,
            KEY_RIGHTSHIFT: False,
        }

        while not self._kbd_stop.is_set():
            dev_path = self._find_keyboard_device()
            if dev_path is None:
                logger.info("No keyboard device found; retrying in 2s")
                time.sleep(2.0)
                continue

            logger.info("Keyboard device: %s", dev_path)

            try:
                with open(dev_path, "rb", buffering=0) as fh:
                    poll = select.poll()
                    poll.register(fh, select.POLLIN)
                    while not self._kbd_stop.is_set():
                        events = poll.poll(200)
                        if not events:
                            continue
                        for fd, _flag in events:
                            if fd != fh.fileno():
                                continue
                            try:
                                data = fh.read(EVENT_SIZE)
                            except OSError as exc:
                                logger.warning("Keyboard read error: %s", exc)
                                break
                            if not data or len(data) < EVENT_SIZE:
                                break

                            try:
                                _sec, _usec, ev_type, code, value = EVENT_STRUCT.unpack(data)
                            except struct.error:
                                continue

                            if ev_type == EV_SYN:
                                continue
                            if ev_type != EV_KEY:
                                continue

                            if code in mod_state:
                                mod_state[code] = bool(value)
                                continue

                            # 只处理按下事件 (value == 1)，忽略释放和重复
                            if value != 1:
                                continue

                            ctrl_held = mod_state[KEY_LEFTCTRL]
                            shift_held = mod_state[KEY_LEFTSHIFT] or mod_state[KEY_RIGHTSHIFT]

                            ch: str | None = None
                            if code == KEY_ENTER:
                                ch = "\r"
                            elif code == KEY_BACKSPACE:
                                ch = "\x7f"
                            elif ctrl_held:
                                if code == 46:  # KEY_C
                                    ch = "\x03"
                                elif code == 47:  # KEY_V
                                    pass  # paste not supported
                                elif code == 45:  # KEY_X
                                    ch = "\x18"
                                elif code == 32:  # KEY_D
                                    ch = "\x04"
                                elif code == 38:  # KEY_L
                                    ch = "\x0c"
                            elif code in _KEY_SEQUENCES:
                                ch = _KEY_SEQUENCES[code]
                            elif code in _KEY_SPECIAL:
                                ch = _KEY_SPECIAL[code]
                            elif shift_held:
                                ch = _KEY_SHIFT.get(code)
                            else:
                                ch = _KEY_NORMAL.get(code)

                            if ch is not None:
                                logger.debug("Key 0x%02x -> %r", code, ch)
                                self._kbd_queue.put(ch)
            except OSError as exc:
                logger.error("Keyboard device error: %s", exc)

            # 设备断开或读取失败，等待后重新查找
            logger.info("Keyboard device lost; reconnecting...")
            time.sleep(1.0)

    def on_activate(self) -> None:
        logger.info("Console mode activated")
        self._lines = []
        self._buf = ""
        self._running = True
        try:
            pid, fd = pty.fork()
            if pid == 0:
                # 子进程：设置终端环境变量，确保 bash 正确进入交互模式并回显
                os.environ["COLUMNS"] = str(self.COLS)
                os.environ["LINES"] = str(MAX_LINES)
                os.environ["TERM"] = "linux"
                os.execvp("bash", ["bash", "--norc"])
            else:
                self._child_fd = fd
                # 设置 PTY 窗口大小，让 bash 行编辑和回显正确工作
                winsize = struct.pack("HHHH", MAX_LINES, self.COLS, 0, 0)
                fcntl.ioctl(fd, TIOCSWINSZ, winsize)
                fl = fcntl.fcntl(fd, fcntl.F_GETFL)
                fcntl.fcntl(fd, fcntl.F_SETFL, fl | os.O_NONBLOCK)
                self._reader_thread = threading.Thread(target=self._reader, daemon=True)
                self._reader_thread.start()
                self._lines.append("Console ready. Type commands below.")
                self._lines.append("Use top-right button to switch back.")
                self._lines.append("")
        except Exception as exc:
            logger.error("Failed to start console pty: %s", exc)
            self._lines = ["Console unavailable", str(exc)]
            return

        self._kbd_stop.clear()
        self._kbd_thread = threading.Thread(target=self._kbd_reader_thread, daemon=True)
        self._kbd_thread.start()

    def on_deactivate(self) -> None:
        logger.info("Console mode deactivated")
        self._running = False
        self._kbd_stop.set()
        if self._child_fd is not None:
            try:
                os.close(self._child_fd)
            except OSError:
                pass
            self._child_fd = None
        if self._reader_thread is not None:
            self._reader_thread.join(timeout=1.0)
            self._reader_thread = None
        if self._kbd_thread is not None:
            self._kbd_thread.join(timeout=1.0)
            self._kbd_thread = None

    def on_tick(self) -> None:
        if self._child_fd is None:
            return
        had_input = False
        while True:
            try:
                ch = self._kbd_queue.get_nowait()
            except Empty:
                break
            had_input = True
            try:
                os.write(self._child_fd, ch.encode("utf-8"))
            except OSError as exc:
                logger.warning("Failed to write to pty: %s", exc)
                break

        if had_input:
            # 给 shell 少量时间回显，然后立即刷新屏幕，使输入实时可见
            time.sleep(0.02)
            blit(self.render())

    def handle_touch(self, event: TouchEvent) -> bool:
        return False

    def render(self) -> Image.Image:
        img = Image.new("RGB", (W, H), CONSOLE_BG)
        draw = ImageDraw.Draw(img)
        font = get_font(FONT_SIZE, mono=True)

        draw.rectangle([0, 0, W, STATUS_BAR_H], fill="#111111")
        # 标题 "CONSOLE" 靠左对齐，避免与右上角按钮重叠
        draw.text((6, 2), "CONSOLE", font=font, fill=CONSOLE_GREEN)

        # ===== 绘制右上角模式切换按钮 =====
        # 按钮占用屏幕右上角区域，宽度 SWITCH_HOTZONE_SIZE，高度约 16px
        # 触摸区域在 panel.py 中定义，范围大于可视区域以便点击
        btn_x = W - SWITCH_HOTZONE_SIZE
        # 绘制按钮背景：蓝色矩形 + 边框
        draw.rectangle([btn_x, 2, W, STATUS_BAR_H - 2], fill="#1e3a5f", outline="#333333")
        # 绘制按钮文字：黄色 "MONITOR"，水平居中于按钮区域
        btn_text = "MONITOR"
        btn_tw = draw.textbbox((0, 0), btn_text, font=font)[2]
        draw.text((btn_x + (SWITCH_HOTZONE_SIZE - btn_tw) // 2, 3), btn_text, font=font, fill=COLORS["yellow"])
        # ===== 按钮绘制结束 =====

        # 主机名显示在按钮左侧与标题之间的空隙处，避免与按钮重叠
        try:
            hn = os.uname().nodename
            # 限制主机名可用区域：标题 "CONSOLE" 右侧到按钮左侧
            hostname_max_x = btn_x - 10
            hostname_w = len(hn) * 7
            hostname_x = max(80, hostname_max_x - hostname_w)
            if hostname_x + hostname_w <= hostname_max_x:
                draw.text((hostname_x, 2), hn, font=font, fill=CONSOLE_GRAY)
        except Exception:
            pass
        draw.rectangle([0, STATUS_BAR_H, W, STATUS_BAR_H], fill="#333333", width=1)

        with self._lock:
            # 保留一行空间给当前尚未换行的输入缓冲
            display_lines = self._lines[-(MAX_LINES - 1):]
            current_input = self._buf

        y = STATUS_BAR_H + 4
        for line in display_lines:
            draw.text((6, y), line, font=font, fill=CONSOLE_WHITE)
            y += LINE_H

        if current_input:
            # 显示 shell 回显的当前行（未遇到换行符的缓冲内容）
            prompt = current_input[-self.COLS:]
            draw.text((6, y), prompt, font=font, fill=CONSOLE_WHITE)

        draw.rectangle([0, H - 20, W, H], fill="#111111")
        draw.text((110, H - 18), "Powered by RichLi4307", font=font, fill=CONSOLE_GRAY)

        return img

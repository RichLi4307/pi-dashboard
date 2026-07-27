from __future__ import annotations

import logging
import signal
import threading
import time
from abc import ABC, abstractmethod
from queue import Empty, Queue

from PIL import Image

from .config import MODE_NAMES, REFRESH_INTERVAL, SWITCH_HOTZONE_HEIGHT, SWITCH_HOTZONE_SIZE, TOUCH_POLL_INTERVAL, W
from .ipc_server import IpcServer
from .render import blit, clear_framebuffer, render_boot_screen, render_overlay, wait_fb
from .touch import touch_reader_thread, TouchEvent

logger = logging.getLogger("pi_dashboard.panel")


class Mode(ABC):
    name: str

    @abstractmethod
    def on_activate(self) -> None:
        pass

    @abstractmethod
    def on_deactivate(self) -> None:
        pass

    @abstractmethod
    def render(self) -> Image.Image:
        pass

    @abstractmethod
    def handle_touch(self, event: TouchEvent) -> bool:
        pass

    @abstractmethod
    def on_tick(self) -> None:
        pass


class Panel:
    def __init__(self) -> None:
        self.modes: dict[str, Mode] = {}
        self.active_mode: str | None = None
        self.event_queue: Queue = Queue()
        self._running = True
        self._switch_request: str | None = None
        self._touch_stop = threading.Event()
        self._touch_thread: threading.Thread | None = None
        self._ipc_server = IpcServer(self)
        # 模式切换冷却：记录上次切换时间，时间窗口内忽略重复触发
        self._last_switch_time: float = 0.0
        # 模式切换冷却时间（秒），防止长按或抖动导致的多次切换
        self._switch_cooldown = 1.0

        signal.signal(signal.SIGTERM, self._handle_signal)
        signal.signal(signal.SIGINT, self._handle_signal)

    def _handle_signal(self, signum: int, _frame) -> None:
        logger.info("Received signal %s, shutting down...", signum)
        self._running = False

    def register_mode(self, mode: Mode) -> None:
        self.modes[mode.name] = mode

    def switch_mode(self, name: str) -> None:
        self._switch_request = name

    # 执行模式切换：停用旧模式 → 激活新模式 → 显示切换提示浮层
    def _do_switch(self, name: str) -> None:
        if name not in self.modes:
            logger.warning("Unknown mode: %s", name)
            return
        if name == self.active_mode:
            return

        # 停用当前模式（清理资源、停止子进程等）
        if self.active_mode is not None:
            self.modes[self.active_mode].on_deactivate()

        # 激活新模式
        self.active_mode = name
        self.modes[name].on_activate()

        # 在切换后的画面中央显示 "Switch to Monitor/Console Mode" 浮层，持续 1 秒
        overlay_text = "Monitor" if name == "monitor" else "Console"
        overlay_text = f"Switch to {overlay_text} Mode"
        current_img = self.modes[name].render()
        render_overlay(current_img, overlay_text, duration=1.0)

        logger.info("Switched to mode: %s", name)

    def run(self) -> None:
        if not wait_fb():
            logger.critical("Framebuffer unavailable; exiting.")
            return

        try:
            blit(render_boot_screen())
        except Exception as exc:
            logger.error("Failed to draw boot screen: %s", exc)

        if not self.modes:
            logger.critical("No modes registered; exiting.")
            return

        self._switch_request = MODE_NAMES[0]
        try:
            self._ipc_server.start()
        except Exception as exc:
            logger.warning("Failed to start IPC server: %s", exc)

        self._touch_thread = threading.Thread(
            target=touch_reader_thread,
            args=(self.event_queue, self._touch_stop),
            daemon=True,
        )
        self._touch_thread.start()

        # 上次渲染时间戳，控制渲染频率为 REFRESH_INTERVAL
        # 但触摸事件以 TOUCH_POLL_INTERVAL 高频轮询，确保即时响应
        last_render = 0.0
        while self._running:
            now = time.monotonic()

            if self._switch_request is not None:
                self._do_switch(self._switch_request)
                self._switch_request = None
                last_render = now  # 切换后立即刷新

            # 高频处理触摸事件（每 10ms 轮询一次），确保触摸无明显延迟
            self._drain_touch_events()

            # on_tick 高频调用，确保键盘等事件及时处理
            # render 仍按 REFRESH_INTERVAL 节流，避免刷屏
            if self.active_mode is not None:
                current = self.modes[self.active_mode]
                try:
                    current.on_tick()
                except Exception as exc:
                    logger.exception("Mode tick failed for %s", self.active_mode)

                if (now - last_render) >= REFRESH_INTERVAL:
                    try:
                        img = current.render()
                        blit(img)
                    except Exception as exc:
                        logger.exception("Mode render failed for %s", self.active_mode)
                        try:
                            from .render import render_boot_screen
                            blit(render_boot_screen())
                        except Exception:
                            logger.exception("Failed to render fallback screen")
                    last_render = now

            time.sleep(TOUCH_POLL_INTERVAL)

        self.shutdown()

    # 将触摸事件分发给当前活动模式处理
    # 优先级：当前模式的 handle_touch() 优先处理（如容器列表滚动）
    # 若模式未消费该事件，则检测是否为模式切换按钮点击
    def _drain_touch_events(self) -> None:
        if self.active_mode is None:
            return
        current = self.modes[self.active_mode]
        while True:
            try:
                event = self.event_queue.get_nowait()
            except Empty:
                break

            # 步骤一：让当前模式优先处理触摸事件
            consumed = current.handle_touch(event)
            # 步骤二：模式未消费 → 判断是否点击了模式切换按钮
            if not consumed:
                self._handle_mode_switch_touch(event)

    # 处理右上角模式切换按钮的触摸事件
    # 按钮渲染位置：屏幕右上角，宽度 SWITCH_HOTZONE_SIZE，高度约 22px
    # 触摸区域：水平范围 [W - SWITCH_HOTZONE_SIZE, W)，垂直范围 [0, SWITCH_HOTZONE_HEIGHT)
    # 注意：触摸区域高度 SWITCH_HOTZONE_HEIGHT(40px) 大于可视按钮高度(22px)，
    #       这是有意设计，方便手指点击时更容易命中
    def _handle_mode_switch_touch(self, event: TouchEvent) -> None:
        # 只处理按下事件，忽略抬起事件
        if not event.pressed:
            return
        # 冷却检测：上次切换后未满 _switch_cooldown 秒则忽略本次触摸
        # 防止手指长按按钮、触摸不稳定或多次按下导致连续切换
        now = event.timestamp or time.monotonic()
        if now - self._last_switch_time < self._switch_cooldown:
            return
        x, y = event.x, event.y
        # 计算按钮触摸区域右边界 = 屏幕右边缘，左边界 = 右边缘 - 按钮宽度
        x_start = W - SWITCH_HOTZONE_SIZE
        x_end = W
        y_end = SWITCH_HOTZONE_HEIGHT
        # 检测触摸坐标是否落在按钮触摸区域内
        if x_start <= x <= x_end and 0 <= y <= y_end:
            # 更新切换时间戳，进入冷却
            self._last_switch_time = now
            # 根据当前模式切换到另一个模式
            if self.active_mode == "monitor":
                self.switch_mode("console")
            elif self.active_mode == "console":
                self.switch_mode("monitor")

    def shutdown(self) -> None:
        logger.info("Shutting down panel...")
        self._touch_stop.set()
        if self._touch_thread is not None and self._touch_thread.is_alive():
            self._touch_thread.join(timeout=2.0)
        try:
            self._ipc_server.stop()
        except Exception as exc:
            logger.warning("Failed to stop IPC server: %s", exc)
        clear_framebuffer()

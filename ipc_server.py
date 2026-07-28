"""IPC server for external control of the Pi Dashboard panel.

Listens on a Unix domain socket and exposes a small JSON protocol for
screenshots and control commands. Designed to be consumed by
pi-dashboard-mcp.
"""

from __future__ import annotations

import base64
import io
import json
import logging
import os
import socket
import threading
import time
from typing import Any

from .metrics import get_ip_list, read_tailscale_status
from .touch import TouchEvent

logger = logging.getLogger("pi_dashboard.ipc")

DEFAULT_SOCKET_PATH = "/var/lib/pi-dashboard/pi_dashboard.sock"


class IpcServer:
    def __init__(self, panel, socket_path: str = DEFAULT_SOCKET_PATH) -> None:
        self.panel = panel
        self.socket_path = socket_path
        self._stop = threading.Event()
        self._thread: threading.Thread | None = None

    def start(self) -> None:
        self._thread = threading.Thread(target=self._serve, daemon=True)
        self._thread.start()
        logger.info("IPC server thread started")

    def stop(self) -> None:
        logger.info("Stopping IPC server...")
        self._stop.set()
        # Wake up accept() by connecting to ourselves.
        try:
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as wake:
                wake.connect(self.socket_path)
        except OSError:
            pass
        if self._thread is not None and self._thread.is_alive():
            self._thread.join(timeout=2.0)
        self._remove_socket()

    def _ensure_directory(self) -> None:
        directory = os.path.dirname(self.socket_path)
        if directory and not os.path.isdir(directory):
            try:
                os.makedirs(directory, mode=0o777, exist_ok=True)
            except OSError as exc:
                logger.warning("Failed to create socket directory %s: %s", directory, exc)

    def _remove_socket(self) -> None:
        try:
            if os.path.exists(self.socket_path):
                os.unlink(self.socket_path)
        except OSError as exc:
            logger.warning("Failed to remove socket %s: %s", self.socket_path, exc)

    def _set_socket_permissions(self) -> None:
        try:
            os.chmod(self.socket_path, 0o666)
        except OSError as exc:
            logger.warning("Failed to chmod socket %s: %s", self.socket_path, exc)

    def _serve(self) -> None:
        self._ensure_directory()
        self._remove_socket()

        sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        sock.settimeout(1.0)
        try:
            sock.bind(self.socket_path)
            self._set_socket_permissions()
            sock.listen(4)
            logger.info("IPC server listening on %s", self.socket_path)
        except OSError as exc:
            logger.error("Failed to bind IPC socket: %s", exc)
            return

        while not self._stop.is_set():
            try:
                conn, _ = sock.accept()
            except socket.timeout:
                continue
            except OSError:
                break

            try:
                self._handle_connection(conn)
            except Exception as exc:
                logger.exception("IPC connection handler failed: %s", exc)
            finally:
                try:
                    conn.close()
                except OSError:
                    pass

        try:
            sock.close()
        except OSError:
            pass
        self._remove_socket()

    def _handle_connection(self, conn: socket.socket) -> None:
        data = b""
        while True:
            chunk = conn.recv(4096)
            if not chunk:
                break
            data += chunk
            if b"\n" in data:
                break

        if not data:
            return

        try:
            request = json.loads(data.decode("utf-8").strip())
        except (json.JSONDecodeError, UnicodeDecodeError) as exc:
            self._send(conn, {"status": "error", "message": f"invalid json: {exc}"})
            return

        response = self._handle_request(request)
        self._send(conn, response)

    def _send(self, conn: socket.socket, response: dict[str, Any]) -> None:
        try:
            conn.sendall(json.dumps(response).encode("utf-8") + b"\n")
        except OSError as exc:
            logger.debug("Failed to send IPC response: %s", exc)

    def _handle_request(self, request: dict[str, Any]) -> dict[str, Any]:
        action = request.get("action")
        if action == "screenshot":
            return self._handle_screenshot()
        if action == "status":
            return self._handle_status()
        if action == "switch_mode":
            mode = request.get("mode", "monitor")
            return self._handle_switch_mode(mode)
        if action == "scroll_containers":
            return self._handle_scroll_containers()
        return {"status": "error", "message": f"unknown action: {action}"}

    def _handle_status(self) -> dict[str, Any]:
        """返回宿主机视角的 IP 与 Tailscale 状态，供容器内的 MCP 使用。"""
        try:
            return {
                "status": "ok",
                "ips": get_ip_list(),
                "tailscale": read_tailscale_status(),
            }
        except Exception as exc:
            logger.exception("Status request failed")
            return {"status": "error", "message": str(exc)}

    def _handle_screenshot(self) -> dict[str, Any]:
        try:
            active = self.panel.active_mode
            if active is None or active not in self.panel.modes:
                return {"status": "error", "message": "no active mode"}

            img = self.panel.modes[active].render()
            buffer = io.BytesIO()
            img.save(buffer, format="PNG")
            encoded = base64.b64encode(buffer.getvalue()).decode("utf-8")
            return {"status": "ok", "data": encoded}
        except Exception as exc:
            logger.exception("Screenshot failed")
            return {"status": "error", "message": str(exc)}

    def _handle_switch_mode(self, mode: str) -> dict[str, Any]:
        if mode not in self.panel.modes:
            return {"status": "error", "message": f"unknown mode: {mode}"}
        try:
            self.panel.switch_mode(mode)
            return {"status": "ok", "mode": mode}
        except Exception as exc:
            logger.exception("Switch mode failed")
            return {"status": "error", "message": str(exc)}

    def _handle_scroll_containers(self) -> dict[str, Any]:
        try:
            active = self.panel.active_mode
            if active != "monitor":
                return {"status": "error", "message": "not in monitor mode"}

            monitor = self.panel.modes["monitor"]
            total = len(monitor.containers)
            from .config import CONTAINER_PAGE_SIZE, DOCKER_LIST_Y

            max_offset = max(0, total - CONTAINER_PAGE_SIZE)
            if max_offset <= 0:
                return {"status": "ok", "offset": 0, "total": total}

            # Simulate a touch event in the container list area.
            event = TouchEvent(
                x=10, y=DOCKER_LIST_Y + 4, pressed=True, timestamp=time.time()
            )
            monitor.handle_touch(event)
            return {
                "status": "ok",
                "offset": monitor.container_scroll_offset,
                "total": total,
            }
        except Exception as exc:
            logger.exception("Scroll containers failed")
            return {"status": "error", "message": str(exc)}

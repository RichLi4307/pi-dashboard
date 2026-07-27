"""Shared system metrics collection for Pi Dashboard and MCP integration.

This module is intentionally free of framebuffer / rendering dependencies so
that it can be imported both by the local panel process and by external
consumers (e.g. pi-dashboard-mcp).
"""

from __future__ import annotations

import logging
import os
import subprocess
import time
from collections import defaultdict, deque
from typing import List, Tuple

from .config import CPU_SMOOTH_WINDOW, IP_FILTER_ENABLED

logger = logging.getLogger("pi_dashboard.metrics")


def _run(args: List[str], timeout: float = 5.0) -> str:
    try:
        result = subprocess.run(
            args,
            capture_output=True,
            text=True,
            timeout=timeout,
            check=False,
        )
        if result.returncode != 0:
            logger.debug(
                "Command %s exited %d: %s",
                args,
                result.returncode,
                result.stderr.strip(),
            )
        return result.stdout.strip()
    except (subprocess.TimeoutExpired, OSError) as exc:
        logger.warning("Command %s failed: %s", args, exc)
        return ""


def read_cpu_temp() -> str:
    try:
        with open("/sys/class/thermal/thermal_zone0/temp", "r") as fh:
            raw = fh.read().strip()
        return f"{int(raw) / 1000:.0f}C"
    except (OSError, ValueError) as exc:
        logger.debug("CPU temp read failed: %s", exc)
        return "N/A"


class CpuSampler:
    def __init__(self) -> None:
        self._prev: dict[str, tuple[int, int]] | None = None
        self._prev_time: float | None = None
        self._history: dict[str, deque[float]] = defaultdict(
            lambda: deque(maxlen=CPU_SMOOTH_WINDOW)
        )

    def read(self) -> dict[str, float]:
        now = time.time()
        curr = self._sample()
        if not curr:
            return {}
        if self._prev is None or self._prev_time is None:
            self._prev = curr
            self._prev_time = now
            return {core: 0.0 for core in curr}

        raw: dict[str, float] = {}
        for core in curr:
            if core not in self._prev:
                raw[core] = 0.0
                continue
            total_diff = curr[core][0] - self._prev[core][0]
            idle_diff = curr[core][1] - self._prev[core][1]
            if total_diff > 0:
                usage = 100.0 * (1.0 - idle_diff / total_diff)
                raw[core] = max(0.0, min(100.0, usage))
            else:
                raw[core] = 0.0

        self._prev = curr
        self._prev_time = now

        results: dict[str, float] = {}
        for core, usage in raw.items():
            self._history[core].append(usage)
            results[core] = sum(self._history[core]) / len(self._history[core])
        return results

    @staticmethod
    def _sample() -> dict[str, tuple[int, int]]:
        stats: dict[str, tuple[int, int]] = {}
        try:
            with open("/proc/stat", "r") as fh:
                for line in fh:
                    if not line.startswith("cpu"):
                        break
                    parts = line.split()
                    if len(parts) < 5:
                        continue
                    core = parts[0]
                    if core != "cpu" and not (
                        core.startswith("cpu") and core[3:].isdigit()
                    ):
                        continue
                    values = [int(v) for v in parts[1:]]
                    total = sum(values)
                    idle = values[3] + values[4]
                    stats[core] = (total, idle)
        except (OSError, ValueError, IndexError) as exc:
            logger.debug("CPU stat read failed: %s", exc)
        return stats


def read_mem_info() -> str:
    try:
        mi: dict[str, int] = {}
        with open("/proc/meminfo", "r") as fh:
            for line in fh:
                if ":" not in line:
                    continue
                key, val = line.split(":", 1)
                try:
                    mi[key.strip()] = int(val.strip().split()[0])
                except (ValueError, IndexError):
                    continue

        total = mi.get("MemTotal", 0)
        if total == 0:
            return "N/A"
        available = mi.get("MemAvailable", mi.get("MemFree", 0))
        used = total - available
        pct = 100.0 * used / total
        return f"{used // 1024}/{total // 1024}MB ({pct:.0f}%)"
    except OSError as exc:
        logger.debug("Mem info read failed: %s", exc)
        return "N/A"


def read_disk_usage() -> str:
    try:
        st = os.statvfs("/")
        if st.f_blocks == 0:
            return "N/A"
        pct = 100.0 * (1.0 - st.f_bfree / st.f_blocks)
        return f"{pct:.0f}%"
    except OSError as exc:
        logger.debug("Disk stat failed: %s", exc)
        return "N/A"


def read_docker_containers() -> List[Tuple[str, str, str]]:
    out = _run(
        ["docker", "ps", "-a", "--format", "{{.Names}}|{{.Status}}|{{.State}}"],
        timeout=5.0,
    )
    containers: List[Tuple[str, str, str]] = []
    for line in out.splitlines():
        parts = line.split("|", 2)
        if len(parts) != 3:
            continue
        name, status, state = parts
        containers.append((name[:18], status[:40], state))
    return containers


def read_tailscale_status() -> str:
    out = _run(["tailscale", "status", "--json"], timeout=3.0)
    return "ON" if '"BackendState": "Running"' in out else "OFF"


def get_ip_list() -> List[str]:
    out = _run(["hostname", "-I"])
    if not out:
        return ["No IP"]
    ips = [ip for ip in out.split() if not ip.startswith("127.")]
    if IP_FILTER_ENABLED:
        ips = [
            ip
            for ip in ips
            if ip.startswith("192.") or ip.startswith("10.") or ip.startswith("100.")
        ]
    return ips if ips else ["No IP"]


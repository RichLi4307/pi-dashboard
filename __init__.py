from __future__ import annotations

import logging
import sys

from .console_mode import ConsoleMode
from .monitor_mode import MonitorMode
from .panel import Panel

logger = logging.getLogger("pi_dashboard")


def main() -> None:
    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s [%(levelname)s] %(message)s",
        handlers=[logging.StreamHandler(sys.stderr)],
    )

    panel = Panel()
    panel.register_mode(MonitorMode())
    panel.register_mode(ConsoleMode())
    panel.run()




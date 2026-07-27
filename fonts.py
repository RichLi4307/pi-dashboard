from __future__ import annotations

import logging
import os

from PIL import ImageFont

from .config import FONT_PATHS, MONO_FONT_PATHS

logger = logging.getLogger("pi_dashboard.fonts")

_FONT_CACHE: dict[tuple[str, int], ImageFont.FreeTypeFont | ImageFont.ImageFont] = {}


def get_font(size: int, mono: bool = False) -> ImageFont.FreeTypeFont | ImageFont.ImageFont:
    cache_key = ("mono" if mono else "sans", size)
    cached = _FONT_CACHE.get(cache_key)
    if cached is not None:
        return cached

    paths = MONO_FONT_PATHS if mono else FONT_PATHS
    for path in paths:
        if os.path.exists(path):
            try:
                font = ImageFont.truetype(path, size)
                _FONT_CACHE[cache_key] = font
                return font
            except OSError as exc:
                logger.debug("Font load failed for %s: %s", path, exc)

    default = ImageFont.load_default()
    _FONT_CACHE[cache_key] = default
    return default

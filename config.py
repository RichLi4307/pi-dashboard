from __future__ import annotations

FB = "/dev/fb1"
W, H = 480, 320

# GitHub Dark 主题配色（primer primitives）
COLORS: dict[str, str] = {
    "bg": "#0d1117",      # canvas-default
    "panel": "#161b22",   # canvas-subtle
    "accent": "#30363d",  # border-default（进度条轨道/分隔线）
    "green": "#3fb950",   # success-fg
    "red": "#f85149",     # danger-fg
    "yellow": "#d29922",  # attention-fg
    "white": "#e6edf3",   # fg-default
    "gray": "#7d8590",    # fg-muted
    "cyan": "#39c5cf",
    "orange": "#f0883e",  # severe-fg
    "blue": "#58a6ff",    # accent-fg
}

# 用量渐变（0% → 100%）：绿 → 亮绿 → 黄 → 橙 → 红（GitHub Dark 同色系）
# 供 CPU 占用条 / 内存 / 磁盘按百分比连续取色，替代原先的三档硬切换
USAGE_GRADIENT = [
    (0.0, "#3fb950"),
    (0.45, "#7ee787"),
    (0.65, "#d29922"),
    (0.85, "#f0883e"),
    (1.0, "#f85149"),
]

# 温度渐变（映射区间见 TEMP_RANGE）：低温蓝 → 青 → 绿 → 黄 → 高温红
TEMP_GRADIENT = [
    (0.0, "#58a6ff"),
    (0.25, "#39c5cf"),
    (0.45, "#3fb950"),
    (0.7, "#d29922"),
    (1.0, "#f85149"),
]

# 温度映射区间（°C）：低于下限显示纯蓝，高于上限显示纯红
TEMP_RANGE = (25.0, 90.0)

FONT_PATHS = [
    "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
]

MONO_FONT_PATHS = [
    "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
    "/usr/share/fonts/truetype/ubuntu/UbuntuMono-R.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
]

REFRESH_INTERVAL = 0.2  # CPU 占用条刷新率 5 FPS
SLOW_RENDER_INTERVAL = 2.0  # 时间/温度/内存/磁盘/IP/容器等慢变内容刷新率 0.5 FPS
SLOW_DATA_INTERVAL = 2.0  # Docker / Tailscale / IP 等慢速数据更新间隔
CPU_SMOOTH_WINDOW = 5  # CPU 占用滑动平均窗口，保证高刷新下读数稳定
BOOT_TIMEOUT = 30

MODE_NAMES = ["monitor", "console"]
# 模式切换按钮的渲染宽度（像素），按钮靠右上角对齐
SWITCH_HOTZONE_SIZE = 80
# 模式切换按钮的触摸响应高度（像素）
# 注意：触摸区域有意大于可视按钮高度（按钮渲染高度约 22px，触摸区 40px），
#       让用户更容易点击到切换区域，无需精确对准按钮
SWITCH_HOTZONE_HEIGHT = 40

# 触摸事件轮询间隔（秒）。100 Hz 对电阻屏无意义且空转耗电，降到 20 Hz。
TOUCH_POLL_INTERVAL = 0.05

CONTAINER_PAGE_SIZE = 10
DOCKER_START_Y = 108
DOCKER_HEADER_Y = DOCKER_START_Y
DOCKER_LIST_Y = DOCKER_START_Y + 18
DOCKER_LINE_HEIGHT = 16

TOUCH_DEVICES = [
    "/dev/input/event1",
    "/dev/input/event0",
    "/dev/input/event2",
    "/dev/input/event3",
]

IP_FILTER_ENABLED = True

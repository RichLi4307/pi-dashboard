# Pi Dashboard

树莓派 4B + 微雪 3.5 寸 TFT 屏的系统仪表盘。开机自启，实时显示 CPU、温度、内存、磁盘、IP、Docker 容器、Tailscale 状态，并支持通过触摸屏切换监控模式 / 控制台模式。

## 功能特性

- **系统监控模式**：CPU 占用条、温度、内存、磁盘、IP 列表、Tailscale 状态、Docker 容器列表
- **控制台模式**：保留触摸入口，可切回监控模式
- **触摸支持**：基于 `python-evdev` + `touch-fix` 坐标修正，支持点击容器列表翻页
- **分层刷新**：CPU 条 5 Hz 高频刷新，慢变内容 0.5 Hz 刷新，降低 CPU 与发热
- **SPI 超频**：默认 32 MHz，物理刷新上限约 13 FPS
- **MCP 接入**：内置 Unix Domain Socket 接口，可被 [`pi-dashboard-mcp`](https://github.com/RichLi4307/pi-dashboard-mcp) 调用，向 AstrBot 暴露系统状态、截图、模式切换等 Tools

## 运行环境

- 树莓派 4B（aarch64）
- Ubuntu 24.04.4 LTS / Raspberry Pi OS 64-bit
- 微雪 3.5A 屏幕（`dtoverlay=waveshare35a:rotate=90,swapxy=1,speed=32000000`）
- `/dev/fb1` 帧缓冲，分辨率 480×320

## 项目结构

```text
.
├── config.py        # 屏幕尺寸、刷新间隔、颜色、容器分页等配置
├── panel.py         # 主面板，管理模式切换与 IPC server
├── monitor_mode.py  # 监控模式渲染与触摸处理
├── console_mode.py  # 控制台模式渲染
├── render.py        # 帧缓冲 RGB565 转换与 blit
├── fonts.py         # 字体加载
├── touch.py         # 触摸事件与坐标映射
├── metrics.py       # 无 framebuffer 依赖的公共指标采集
├── ipc_server.py    # Unix Domain Socket 控制接口（供 MCP Server 使用）
├── __main__.py      # 包入口
└── backups/         # 配置备份
```

## 安装

### 1. 安装系统依赖

```bash
sudo apt update
sudo apt install -y python3-pil python3-numpy fbset fonts-dejavu-core
```

### 2. 放置代码

```bash
cd ~
git clone git@github.com:RichLi4307/pi-dashboard.git
```

或手动把本仓库放到 `~/pi_dashboard/`。

### 3. 安装 systemd 服务

创建 `/etc/systemd/system/pi-dashboard.service`：

```ini
[Unit]
Description=Pi Dashboard for 3.5 inch TFT
After=network.target

[Service]
Type=simple
User=richli
Group=richli
WorkingDirectory=/home/richli/pi_dashboard
Environment=PYTHONUNBUFFERED=1
ExecStart=/usr/bin/python3 -m pi_dashboard
Restart=on-failure
RestartSec=3

# 为 IPC socket 提供 /run/pi_dashboard/ 目录
RuntimeDirectory=pi_dashboard
RuntimeDirectoryMode=0755

[Install]
WantedBy=multi-user.target
```

启用并启动：

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now pi-dashboard.service
```

也可以直接用仓库里的控制脚本：

```bash
~/bin/start-dashboard
~/bin/stop-dashboard
```

## 配置

主要配置在 `config.py`：

| 配置项 | 默认值 | 说明 |
| --- | --- | --- |
| `FB_DEVICE` | `/dev/fb1` | 帧缓冲设备 |
| `SCREEN_WIDTH` / `SCREEN_HEIGHT` | 480 / 320 | 屏幕分辨率 |
| `REFRESH_INTERVAL` | 0.2 | CPU 条刷新间隔（秒）|
| `SLOW_RENDER_INTERVAL` | 2.0 | 慢变内容刷新间隔（秒）|
| `TOUCH_POLL_INTERVAL` | 0.05 | 触摸轮询间隔（秒）|
| `CONTAINER_PAGE_SIZE` | 5 | 容器列表每页行数 |

## 日志

```bash
sudo journalctl -u pi-dashboard.service -f
```

## MCP 集成

本仓库为 [`pi-dashboard-mcp`](https://github.com/RichLi4307/pi-dashboard-mcp) 提供数据与控制接口：

- `metrics.py`：采集 CPU、温度、内存、磁盘、IP、Tailscale、Docker 容器，**不依赖 framebuffer**，可被 MCP Server 直接导入
- `ipc_server.py`：监听 `/run/pi_dashboard/pi_dashboard.sock`，支持以下 action：
  - `{"action": "screenshot"}` — 返回当前面板 PNG 截图（Base64）
  - `{"action": "switch_mode", "mode": "monitor"}` — 切换模式
  - `{"action": "scroll_containers"}` — 在监控模式容器列表中向下翻页

IPC server 随 `panel.py` 启动而启动，失败不会影响本地显示。

## 性能数据

- 渲染优化前：面板进程 CPU 约 15%
- 渲染优化后（32 MHz SPI + 分层刷新 + RGB565 位运算）：面板进程 CPU 约 8-9%，温度下降约 5°C
- IPC / MCP 接入后日常几乎零额外开销，仅在 AstrBot 调用时短暂占用

## 故障排查

| 现象 | 排查方向 |
| --- | --- |
| 屏幕黑屏 | 检查 `dtoverlay=waveshare35a` 是否加载、`/dev/fb1` 是否存在、背光 GPIO 18 是否拉高 |
| 触摸不准 | 运行 `~/touch_calibrate/touch_calibrate.py` 重新校准，并同步更新 `touch-fix.py` 映射参数 |
| 面板刷新卡顿 | 查看 `blit()` 耗时日志，确认 SPI 时钟是否为 32 MHz |
| MCP 截图失败 | 检查 `/run/pi_dashboard/pi_dashboard.sock` 是否存在且 `pi-dashboard-mcp` 容器挂载了 `/run/pi_dashboard` |

## 相关项目

- [pi-dashboard-mcp](https://github.com/RichLi4307/pi-dashboard-mcp) — AstrBot MCP Server，把 Pi Dashboard 封装成 Tools

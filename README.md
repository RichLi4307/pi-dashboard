# Pi Dashboard

树莓派 4B + 微雪 3.5 寸 TFT 屏的系统仪表盘。开机自启，实时显示 CPU、温度、内存、磁盘、IP、Docker 容器、Tailscale 状态，并支持通过触摸屏翻页容器列表。

> **当前版本：v0.2.1**（Rust 二进制；原 Python 实现已归档到 `backups/python-legacy/`）。

## 功能特性

- **系统监控模式**：CPU 占用条、温度、内存、磁盘、IP 列表、Tailscale 状态、Docker 容器列表
- **触摸支持**：裸解析 `input_event`，支持点击容器列表翻页
- **脏区驱动刷新**：仅把变化区域写入 `/dev/fb1`，静止时 SPI 写入量远小于全屏
- **SPI 超频**：48 MHz，主循环 15 FPS
- **MCP 接入**：内置 Unix Domain Socket 接口，可被 [`pi-dashboard-mcp`](https://github.com/RichLi4307/pi-dashboard-mcp) 调用，向 AstrBot 暴露系统状态、截图、模式切换等 Tools

## 运行环境

- 树莓派 4B（aarch64）
- Ubuntu 24.04.4 LTS / Raspberry Pi OS 64-bit
- 微雪 3.5A 屏幕（`/boot/firmware/config.txt`：`dtoverlay=waveshare35a:rotate=90,swapxy=1,speed=48000000`）
- `/dev/fb1` 帧缓冲，分辨率 480×320

## 项目结构

```text
.
├── rust/                  # Rust 实现（当前主代码）
│   ├── src/main.rs        # tokio current_thread 入口
│   ├── src/config.rs      # 常量、颜色、渐变 LUT
│   ├── src/fb.rs          # RGB565 帧缓冲 + 脏区刷新
│   ├── src/render.rs      # 几何原语
│   ├── src/text.rs        # 字体级 baseline 锚定文本引擎
│   ├── src/label.rs       # Label / Bar 字段控件
│   ├── src/metrics.rs     # 快/慢双通道指标采集
│   ├── src/pages/         # Page trait + PageManager + MonitorPage
│   ├── src/touch.rs       # 触摸事件解析
│   ├── src/ipc.rs         # Unix socket IPC server
│   ├── src/screenshot.rs  # RGB565 → PNG
│   └── Cargo.toml
├── docs/                  # 设计文档与报告
│   ├── ui-visual-language-spec.md   # v2 视觉语言规范
│   ├── ui-visual-spec-v3.md         # v3 细节修正包
│   └── ui-visual-spec-v3-report.md  # v3 实施报告
├── backups/               # 配置与旧 Python 实现备份
└── README.md
```

## 安装

### 1. 安装 Rust（Pi 上原生构建）

```bash
export RUSTUP_DIST_SERVER=https://mirrors.tuna.tsinghua.edu.cn/rustup
export RUSTUP_UPDATE_ROOT=https://mirrors.tuna.tsinghua.edu.cn/rustup/rustup
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
source "$HOME/.cargo/env"
```

### 2. 构建

```bash
cd ~/pi_dashboard/rust
CARGO_BUILD_JOBS=2 cargo build --release
```

### 3. 部署为 systemd 服务

创建 `/etc/systemd/system/pi-dashboard.service`：

```ini
[Unit]
Description=Pi Dashboard (Refactored)
After=multi-user.target network.target

[Service]
User=richli
Type=simple
ExecStart=/usr/local/bin/pi-dashboard-rust
WorkingDirectory=/home/richli
StateDirectory=pi-dashboard
Restart=always
RestartSec=3
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
```

启用并启动：

```bash
sudo cp target/release/pi-dashboard-rust /usr/local/bin/pi-dashboard-rust
sudo systemctl daemon-reload
sudo systemctl enable --now pi-dashboard.service
```

便捷控制脚本（如存在）：

```bash
~/bin/start-dashboard
~/bin/stop-dashboard
```

## 配置

主要配置在 `rust/src/config.rs`：

| 配置项 | 默认值 | 说明 |
| --- | --- | --- |
| `FB` | `/dev/fb1` | 帧缓冲设备 |
| `W` / `H` | 480 / 320 | 屏幕分辨率 |
| `REFRESH_INTERVAL_MS` | 67 | 主循环帧间隔（约 15 FPS）|
| `CPU_SMOOTH_WINDOW` | 5 | CPU 占用滑动平均窗口 |
| `SLOW_RENDER_INTERVAL` | 1.0 | 慢变内容刷新间隔（秒）|
| `CONTAINER_PAGE_SIZE` | 8 | 容器列表每页行数 |

## 调试

```bash
# IPC 四 action 冒烟测试
python3 - <<'EOF'
import json, socket
for req in [{"action":"status"},{"action":"scroll_containers"},
            {"action":"switch_mode","mode":"monitor"},{"action":"screenshot"}]:
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect("/var/lib/pi-dashboard/pi_dashboard.sock")
    s.sendall((json.dumps(req)+"\n").encode())
    data = b""
    while not data.endswith(b"\n"): data += s.recv(65536)
    r = json.loads(data.decode().strip()); s.close()
    print(req["action"], "->", r.get("status"), list(r.keys()))
EOF

# 日志
sudo journalctl -u pi-dashboard.service -f
```

## 性能数据

- Rust 版 + 48 MHz SPI + 15 FPS：面板进程 CPU 约 1.7–2.3%，内存约 11–13 MB
- 渲染层整改后：无脏区帧零 SPI 写入，多帧 screenshot diff 无残影跳动
- v3 UI 修正后：`pidstat 1 30` 平均 **1.69%** / 最大 **3.00%**
- 详见 `docs/rust-render-rework-report.md` 与 `docs/ui-visual-spec-v3-report.md`

## 故障排查

| 现象 | 排查方向 |
| --- | --- |
| 屏幕黑屏 | 检查 `dtoverlay=waveshare35a` 是否加载、`/dev/fb1` 是否存在、背光 GPIO 18 是否拉高 |
| 触摸不准 | 运行 `~/touch_calibrate/touch_calibrate.py` 重新校准，并同步更新 `touch-fix.py` 映射参数 |
| 面板刷新卡顿 | 查看 SPI 时钟是否为 48 MHz；检查 flush_dirty 字节数是否异常 |
| MCP 截图失败 | 检查 `/var/lib/pi-dashboard/pi_dashboard.sock` 是否存在且 `pi-dashboard-mcp` 容器挂载了对应路径 |

## 相关项目

- [pi-dashboard-mcp](https://github.com/RichLi4307/pi-dashboard-mcp) — AstrBot MCP Server，把 Pi Dashboard 封装成 Tools

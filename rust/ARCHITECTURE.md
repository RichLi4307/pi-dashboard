# Rust 版 Pi Dashboard 架构设计

> 本文档供架构师评审，**不包含业务代码实现**，只定义模块边界、数据流与接口。

## 1. 顶层目标

- **单一二进制**：`pi-dashboard-rust` 一个可执行文件，systemd 直接拉起。
- **单模式**：只保留 monitor 模式，console 模式移除。
- **低抖动主循环**：固定 10–12 FPS 渲染，子进程全部异步 + 缓存。
- **局部刷新**：仅在 dirty rectangle 范围内写 `/dev/fb1`，降低 SPI 流量。

## 2. 模块结构

```text
src/
├── main.rs        # 入口：初始化 tokio runtime、日志、framebuffer、metrics cache、touch、ipc、主循环
├── config.rs      # 常量：分辨率、颜色、渐变 LUT、刷新间隔、字体路径、socket 路径
├── fb.rs          # Framebuffer：mmap /dev/fb1，RGB565 buffer，dirty rectangle，flush
├── render.rs      # 2D 绘制：矩形、文本、渐变；fontdue glyph cache
├── metrics.rs     # CPU/温度/内存/磁盘同步读取 + 慢速数据（docker/tailscale/IP）异步缓存
├── docker.rs      # 容器列表：tokio::process::Command("docker", "ps", "-a", ...)
├── touch.rs       # evdev 读取、坐标映射、热区检测
├── ipc.rs         # Unix socket server，JSON 协议，PNG screenshot
├── screenshot.rs  # 帧缓冲区域编码为 PNG + base64
└── util.rs        # 小数/颜色/字符串小工具
```

## 3. 数据流

```text
┌─────────────┐     ┌──────────────┐     ┌─────────────────┐
│  /proc /sys │────▶│  metrics.rs  │────▶│  Monitor State  │
│  docker ps  │────▶│ (async cache)│     │  (Arc<RwLock>)  │
│ tailscale   │────▶│              │     └────────┬────────┘
└─────────────┘     └──────────────┘              │
                                                  │ 每 100ms
                                                  ▼
┌─────────────┐     ┌──────────────┐     ┌─────────────────┐     ┌─────────┐
│  touch dev  │────▶│   touch.rs   │────▶│  render loop    │────▶│ fb.rs   │
└─────────────┘     └──────────────┘     │  (dirty rect)   │     │ /dev/fb1│
                                         └─────────────────┘     └─────────┘
                                                  │
                                                  ▼
                                         ┌─────────────────┐
                                         │  ipc.rs server  │◀──── pi-dashboard-mcp
                                         └─────────────────┘
```

## 4. 关键设计决策

### 4.1 帧缓冲局部刷新

- 维护一块 `&mut [u16]` RGB565 buffer，大小 `W * H`。
- 每次绘制操作返回 `Option<Rect>` 表示受影响的屏幕区域。
- `flush()` 合并所有 dirty rectangles，对重叠区域做并集，只把变脏的像素 `write` 到 `/dev/fb1` 的对应偏移。
- 全屏刷新只在初始化、模式切换、IPC screenshot 请求时触发。

> 风险：部分 SPI 屏控制器在全屏连续扫描时，对非顺序写入可能产生撕裂。若实测有问题，退化为每帧全屏刷新，但仍保留 Rust 的 CPU 收益。

### 4.2 字体渲染

- 使用 `fontdue` 加载 `/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf` 等字体。
- 运行时构建一个 `HashMap<(char, u16), GlyphBitmap>` 缓存；只缓存常用 ASCII、数字、冒号、斜杠等，dashboard 上出现的字符集很小。
- 绘制文本时直接 `blit` 已缓存的灰度 bitmap 到 RGB565 buffer，按前景色做 alpha 混合。

### 4.3 指标采集

- **快速路径（每帧/每 100ms）**：
  - CPU：`/proc/stat` 解析，滑动窗口平滑。
  - 温度：`/sys/class/thermal/thermal_zone0/temp`。
  - 时间：`time::OffsetDateTime::now_utc()` 或本地时区。
- **慢速路径（tokio 后台任务，可配置间隔）**：
  - IP：`hostname -I`（通过 IPC status 给 MCP，dashboard 自身可复用同一缓存）。
  - Tailscale：`tailscale status --json`。
  - Docker：`docker ps -a --format ...`。
- 慢速数据放入 `Arc<tokio::sync::RwLock<Metrics>>`，主循环只读。

### 4.4 主循环时序

```rust
loop {
    let frame_start = Instant::now();

    // 1. 读触摸（非阻塞，evdev 已在 tokio 任务里解出事件）
    // 2. 读快速指标
    // 3. 若慢速指标到期，触发 background refresh（不等待）
    // 4. 绘制变化区域，合并 dirty rectangles
    // 5. flush 到 framebuffer
    // 6. sleep 到下一帧

    sleep_until(frame_start + FRAME_INTERVAL).await;
}
```

使用 `tokio::time::interval` 或 `sleep_until` 代替 Python 的 `time.sleep()`，降低 jitter。

### 4.5 IPC 协议兼容性

保持与现有 Python 版一致的 JSON 协议：

```json
{"action": "screenshot"}
{"action": "status"}
{"action": "scroll_containers"}
```

- `screenshot`：把当前 RGB565 buffer 编码为 PNG，base64 返回。
- `status`：返回 `{"status":"ok", "ips": [...], "tailscale": "ON|OFF"}`。
- `scroll_containers`：更新容器列表滚动偏移。

socket 路径保持 `/var/lib/pi-dashboard/pi_dashboard.sock`。

## 5. 与 Python 版本的对应关系

| Python 文件 | Rust 模块 | 备注 |
|---|---|---|
| `config.py` | `config.rs` | 颜色/渐变/间隔常量 |
| `render.py` | `fb.rs` + `render.rs` | RGB565 转换合并到绘制时，不再每帧 numpy |
| `monitor_mode.py` | `main.rs` 主循环 + `metrics.rs` | 静态背景/慢缓存概念保留，用 dirty rect 替代全拷贝 |
| `metrics.py` | `metrics.rs` + `docker.rs` | CPU sampler 算法复刻 |
| `touch.py` | `touch.rs` | 使用 `evdev` crate |
| `ipc_server.py` | `ipc.rs` + `screenshot.rs` | 协议兼容 |
| `console_mode.py` | — | 删除 |
| `panel.py` | `main.rs` | 模式框架简化 |

## 6. 部署形态

systemd service 示例（待细化）：

```ini
[Unit]
Description=Pi Dashboard (Rust)
After=multi-user.target network.target

[Service]
User=richli
Type=simple
ExecStart=/usr/local/bin/pi-dashboard-rust
Restart=always
RestartSec=3
StateDirectory=pi-dashboard
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
```

## 7. 待评审问题

1. 是否保留 `SLOW_RENDER_INTERVAL` 概念，还是让 time/temp/mem/disk 都走 100ms 快速路径？
2. 字体是否预先用 `fontdue` 离线生成位图贴图，运行时只做 blit？
3. IPC screenshot 是否需要独立的高优先级任务，避免阻塞主循环？
4. 是否需要引入 `clap` 做命令行参数，还是全部走环境变量/配置文件？

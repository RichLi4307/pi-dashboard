# Rust 版 Pi Dashboard 架构设计

> 本文档已经架构师评审修订，与 `../docs/rust-coder-brief.md` 共同作为施工依据。
> 评审要点：新增 Page 抽象（未来触屏交互/多页面不可写死）、tokio 改单线程 runtime、
> 移除 evdev/nix/mmap 依赖、IPC 与触摸逐字段对齐 Python 版。
> 2026-08-01 更新：v4 下钻仪表盘（TEMP/CPU/MEM/DISK/NET 详情页 + 电源弹窗）。

## 1. 顶层目标

- **单一二进制**：`pi-dashboard-rust`，systemd 直接拉起。
- **页面可扩展**：已实现 monitor + 五个详情页；页面注册、触摸路由、IPC 切页全部走 Page 抽象，加页面不改核心。
- **低抖动主循环**：15 FPS 节拍，子进程全部异步 + 缓存 + 超时；IO 采集走 /proc 文件解析。
- **脏区驱动刷新**：仅把 dirty rectangle 合并后写入 `/dev/fb1`，无脏区零写入，降低 SPI 流量与功耗。

## 2. 模块结构

```text
src/
├── main.rs        # 入口：tokio current_thread runtime、日志、组装各模块、主循环（无页面专属逻辑）
├── config.rs      # 常量：分辨率、颜色、渐变 LUT、刷新间隔、字体路径、socket 路径、TOUCH_DEVICES
├── fb.rs          # Framebuffer：RGB565 buffer + dirty rect 按重叠区域合并 + write_at 局部/全量写出
├── render.rs      # 几何原语：矩形、线、椭圆、圆角矩形、三角形；RGB565 混合；fill_rect 写前比较
├── text.rs        # 文本引擎：Fonts glyph 缓存（启动预热）、TextStyle、字体级 baseline 锚定的 draw/measure
├── label.rs       # Label/Bar 字段控件：值变化才擦除重绘，脏区 = 旧bbox∪新bbox（页面由字段组合而成）
├── chart.rs       # LineChart 折线图控件：固定/自动量程、网格、阈值线、脏区=图表矩形
├── metrics.rs     # 快通道 /proc//sys 采集 + 慢通道（docker/tailscale/IP）异步缓存 + History 环形缓冲
├── pages/
│   ├── mod.rs     # Page trait、PageAction、PageManager、页面注册表、60s 自动返回
│   ├── detail_common.rs  # 详情页共享模板：返回键、标题、大值、信息区
│   ├── monitor.rs # MonitorPage：v4 主页（四卡/顶栏按钮/CPU/Docker/电源弹窗）
│   ├── temp.rs    # TempPage：温度折线 + 传感器/Throttled/Trend
│   ├── cpu.rs     # CpuPage：总占用折线 + 负载/频率/ governor
│   ├── mem.rs     # MemPage：内存折线 + meminfo 详情
│   ├── disk.rs    # DiskPage：IO 折线 + 空间/挂载信息
│   └── net.rs     # NetPage：速率折线 + 接口/IP/Tailscale
├── touch.rs       # 裸解析 input_event（复刻 touch.py），pointercal 支持，AsyncFd 非阻塞
├── ipc.rs         # Unix socket server，逐行 JSON oneshot 协议
└── screenshot.rs  # RGB565 → RGB888 → PNG → base64
```

## 3. 数据流

```text
/proc/net/dev ──┐
/proc/diskstats─┼──▶ metrics IO 快通道 (1 Hz) ──┐
/proc/stat ─────┘                                │
/proc/meminfo ───────────────────────────────────┤
/sys/thermal ────────────────────────────────────┼──▶ History 环形缓冲 ──┐
                                                 │                       │
docker ps ────▶ metrics 慢通道 (5 s) ───┐        │                       │
tailscale ────▶ (tokio 任务)           ├─▶ watch::Sender<MetricsSnapshot>├─▶ 主循环（borrow 只读）
hostname -I ──▶                        ─┘                                └─▶ ipc.rs（status 用缓存）

touch fd ──▶ touch.rs ──mpsc──▶ 主循环 ──▶ PageManager.route_touch ──▶ 活跃 Page
                                         ──▶ Page.render(fb, snapshot) 标脏
                                         ──▶ fb.flush_dirty() ──▶ /dev/fb1（无脏跳过）

pi-dashboard-mcp ──▶ ipc.rs ──▶ screenshot：锁 fb → 克隆 → 解锁 → 编码 PNG
                              ──▶ switch_mode → PageManager.switch
                              ──▶ scroll_containers → 向主循环注入滚动指令
```

## 4. 关键设计决策

### 4.1 帧缓冲脏区刷新（不用 mmap）

- 应用内维护 `Vec<u16>` RGB565 buffer（480×320），用 `std::fs::File` + `FileExt::write_at` 按合并后的行范围写出，**不 mmap、不依赖 nix/libc fb API**。
- 每次绘制操作向 Framebuffer 上报受影响 `Rect`；flush 时合并重叠 rect、按行范围 `write_at`。
- `full_flush()` 用于初始化、切页、异常恢复；正常帧无脏区时完全不写设备。
- fbtft 驱动的 deferred I/O 会把写过的页推上 SPI；局部写减少了 shadow buffer 的脏页数量，SPI 流量随之下降。
- 回退路径：若实测局部刷新显示异常，配置项切回每帧 `full_flush()`，CPU 收益仍在。

### 4.2 Page 抽象（开闭原则的核心）

```rust
pub enum PageAction {
    None,
    Switch(&'static str), // 目标页面 id
}

pub trait Page {
    fn id(&self) -> &'static str;
    /// 绘制到 fb（内部标脏）；snapshot 为最新指标快照
    fn render(&mut self, fb: &mut Framebuffer, snapshot: &MetricsSnapshot) -> anyhow::Result<()>;
    /// 触摸事件路由；返回页面动作
    fn on_touch(&mut self, ev: TouchEvent) -> PageAction;
    /// 切页进入时调用：触发全屏重绘（默认实现即可）
    fn on_enter(&mut self, fb: &mut Framebuffer) { fb.mark_full_dirty(); }
    /// 切页离开时调用：默认空实现，未来页面可释放资源
    fn on_leave(&mut self, fb: &mut Framebuffer) { let _ = fb; }
}

pub struct PageManager {
    pages: HashMap<&'static str, Box<dyn Page>>,
    active: &'static str,
    home_id: &'static str,
    last_activity: Instant,
}
```

- 新页面 = 新文件实现 `Page` + 在 `main.rs` 注册一行。主循环、IPC、touch 均不感知具体页面。
- IPC `switch_mode` 的 `mode` 即页面 id；当前合法集合 `{monitor,temp,cpu,mem,disk,net}`，未知 id 回 error（协议兼容要求该 action 必须保留）。
- 触摸热区（容器列表滚动区、详情页返回键、主页四卡/CPU 区/电源按钮）由各 Page 内部处理；不引入通用 widget 框架。
- 详情页 60s 无触摸自动返回 monitor（`PageManager.check_idle_timeout`）。

### 4.3 指标采集与共享

- **快通道**（主循环内每帧同步读，开销微秒级）：
  - CPU：`/proc/stat` 解析 + 滑动窗口平滑（算法复刻 metrics.py）。
  - 温度：`/sys/class/thermal/thermal_zone0/temp`。
  - 内存/磁盘：`/proc/meminfo`、statvfs（间隔同 Python 版）。
  - 时间：`time` crate 本地时区。
- **IO 快通道**（metrics 后台任务 1 Hz，禁止子进程）：
  - 网络：`/proc/net/dev` 聚合非 lo/docker/veth/br-* 接口。
  - 磁盘：`/proc/diskstats` 取 `mmcblk0` 第 6/10 字段 × 512B。
  - 差分计算实际速率，写入 `History` 环形缓冲（8 序列 × 120 点）。
- **慢通道**（独立 tokio 任务，间隔 5s）：
  - IP（`hostname -I`）、Tailscale（`tailscale status --json`）、Docker（`docker ps -a`），全部 `tokio::process` + `timeout`（docker 3s、tailscale 5s），失败保留旧缓存记 warn。
  - 容器 CPU 改为读 cgroup v2 `cpu.stat`（`usage_usec`），避免 `docker stats` 高负载。
- 共享：`tokio::sync::watch::channel<MetricsSnapshot>`，主循环与 IPC 都只读 `borrow()`，无锁竞争。

### 4.4 主循环与运行时

```rust
// tokio::runtime::Builder::new_current_thread()
let mut ticker = tokio::time::interval(FRAME_INTERVAL); // 67ms (~15 FPS), MissedTickBehavior::Delay
loop {
    ticker.tick().await;
    // 1. 排空 touch mpsc → PageManager.route_touch（产生 PageAction 则切页）
    // 2. 排空 IPC 控制 mpsc（switch_mode / scroll_containers）
    // 3. 检查详情页 60s  idle 超时
    // 4. 读快通道指标，合并慢通道 snapshot
    // 5. active page render → 标脏；任何 Err：log + 跳过本帧
    // 6. fb.flush_dirty()（无脏零写入）
}
```

- **帧率**：主循环锁定 15 FPS（`REFRESH_INTERVAL_MS = 67`）。SPI 已超频到 48 MHz（`/boot/firmware/config.txt` `speed=48000000`），480×320 RGB565 全屏理论上限约 19 FPS，15 FPS 留有安全余量；脏区刷新让日常帧的 SPI 写入量远小于全屏。
- **current_thread 单线程 runtime**：dashboard 是无 CPU 密集任务的 I/O 型进程，多线程 runtime 的 worker 线程纯属浪费功耗与内存。同时这也是 `time` crate `local-offset` feature 的安全前提（其本地时区查询在多线程下有已知 soundness 问题）。
- 触摸与 IPC 均为 tokio 任务，经 mpsc 把指令送入主循环，状态变更单线程化，避免数据竞争。

### 4.5 触摸（复刻 touch.py，不用 evdev crate）

- 按 `TOUCH_DEVICES` 顺序选第一个存在的设备节点（优先虚拟校准设备 event1，回退 event0）。
- fd 置 `O_NONBLOCK` + `tokio::io::unix::AsyncFd`，手动解析 `input_event`（aarch64 24 字节，布局同 Python `struct.Struct("llHHI")`）。
- `/etc/pointercal` 存在则按同一矩阵公式映射，否则原样传递并钳制 479×319。
- 语义：按下期间坐标变化实时上报 pressed 事件；抬起以最后坐标上报一次 release。
- 设备缺失：任务内退避重试，不影响其余功能。
- v4 触摸热区：
  - 主页：四卡 → 详情页，CPU 区 → cpu 页，[RST]/[PWR] → 确认弹窗，Docker 表 → 滚动，[MENU] 占位 no-op。
  - 详情页：左上角返回键 → monitor，其余区域仅重置 60s 计时。
  - 弹窗：CANCEL / 点外取消，CONFIRM 执行，10s 无操作自动取消。

### 4.6 IPC 协议（逐字段对齐 ipc_server.py）

- `/var/lib/pi-dashboard/pi_dashboard.sock`；bind 前删旧文件、chmod 0666、backlog 4。
- 每连接读到首个 `\n` → 处理一个请求 → 回一行 JSON + `\n` → 关闭。
- 响应格式：
  - `screenshot` → `{"status":"ok","data":"<base64 png>"}`
  - `status` → `{"status":"ok","ips":[...],"tailscale":"..."}`（数据取自慢通道缓存，不实时跑子进程）
  - `switch_mode`（`mode` 字段）→ PageManager 切页；合法 mode 集合 `{monitor,temp,cpu,mem,disk,net}`，未知 mode → `{"status":"error","message":"unknown mode: ..."}`
  - `scroll_containers` → `{"status":"ok","offset":N,"total":M}`（不在 monitor 页时回 error，同 Python）
  - 未知 action / 畸形 JSON → `{"status":"error","message":...}`，listener 不中断
- screenshot：锁 fb → 克隆 `Vec<u16>` → 解锁 → RGB565 转 RGB888 编码 PNG。编码在锁外，不阻塞渲染帧。

### 4.7 文本引擎与字段控件（2026-07-29 整改）

> 细节与验收标准见 `../docs/rust-render-rework-directive.md`。

- `fontdue` 加载 MapleMono NF CN，glyph 缓存即"图案映射表"：单线程 `RefCell` 存储，启动预热 ASCII+数字+常用符号。
  - 字重默认 **Medium**；小字号（≤10 px）在 480×320 面板上出现笔画粘连，因此额外加载 **Regular**（使用 `pyftsubset` 生成的 ASCII 子集，约 87 KB，避免完整 TTF 的内存开销）。
  - `TextStyle` 携带 `FontWeight`，`Fonts::font(style)` 按字重选择对应 `Font`；cache key 包含字重。
- **锚点铁律**：垂直锚点只取 `font.horizontal_line_metrics(px)`（`baseline = y + ascent`），对给定 (font,size,weight) 恒定；禁止由字符串字形决定纵向位置。同一行所有文本段共享同一 `baseline_y`。
- `measure` 返回 advance 宽度，与 `draw` 共用同一光标推进实现。
- **Label 控件**：页面上所有文本字段都是 `Label { x, baseline_y, style, align, last_text, last_bbox, bg }` 实例；`set(text)` 值不变零操作，变化时擦除 旧bbox∪新bbox 后按共享 baseline 重绘，只标该并集为脏。CPU 条同理为 `Bar { last_pct }`。
- **变化驱动重绘**：删除一切 blanket `mark_dirty`；静止画面每秒 SPI 写入量必须远小于全屏。
- 质量保障：baseline 恒定单测、measure/draw 一致性单测、Label 值不变零脏区单测、headless 黄金图对比测试。

### 4.8 图表控件（2026-08-01 新增）

- `LineChart`：固定量程（如温度 20–90°C、CPU/MEM 0–100%）或自动量程（IO 类，nice_ceil）。
- 数据不变零操作；变化时先擦背景（图表矩形）再画网格/阈值线/折线，脏区 = 图表矩形。
- 双序列支持（net up/down、disk read/write），颜色走语义常量。

### 4.9 字体渲染质量风险

- 小字号观感若仍逊于 PIL，回退预渲染位图字体（只改 text.rs，不动架构）。

## 5. 与 Python 版本的对应关系

| Python 文件 | Rust 模块 | 备注 |
|---|---|---|
| `config.py` | `config.rs` | 颜色/渐变/间隔/设备列表常量 |
| `render.py` / `fonts.py` | `fb.rs` + `render.rs` + `text.rs` + `label.rs` | RGB565 转换合并进绘制路径；文本引擎与字段控件 |
| `monitor_mode.py` | `pages/monitor.rs` | v4 主页 + 电源弹窗 |
| `metrics.py` | `metrics.rs` | CPU 平滑算法复刻；IO 文件解析；History 缓冲 |
| `touch.py` | `touch.rs` | 裸解析 input_event，语义逐条对齐 |
| `ipc_server.py` | `ipc.rs` + `screenshot.rs` | 协议逐字段对齐；mode 集合扩展为 6 个 |
| `panel.py` | `pages/mod.rs`（PageManager） | 模式框架 → Page 抽象 |
| `console_mode.py` | — | 删除，不迁移 |

## 6. 部署形态

- 二进制安装到 `/usr/local/bin/pi-dashboard-rust`，修改现有 `pi-dashboard.service` 的 `ExecStart`（保留 `StateDirectory=pi-dashboard`、`Restart=always`），修改前备份到 `~/pi_dashboard/backups/`。
- Python 包保留作为回退，回退 = 恢复 ExecStart + 重启服务。
- v4 起详情页由触摸/IPC 进入，无需 MCP 改动。

## 7. 评审决议（原“待评审问题”已关闭）

1. **双通道采集保留**：time/temp/mem/disk 不快通道化；脏区驱动渲染使 1 Hz 变化的内容自然只在变化时重绘，功耗最优。
2. **运行时 glyph 缓存**，不做离线 atlas（字符集小，收益不抵复杂度）。
3. **screenshot 不设独立高优先级任务**：克隆 buffer 仅 ~300KB memcpy，亚毫秒级；编码放锁外即可。
4. **不引入 clap**：全部常量走 `config.rs`，仅 `RUST_LOG` 等个别环境变量做覆盖。

另有两条评审新增决议：

5. **单线程 runtime**：tokio `new_current_thread`，见 §4.4。
6. **零系统级 C 依赖**：不用 evdev/nix/mmap，最终依赖树纯 Rust + libc（libc 仅为 `O_NONBLOCK`）。

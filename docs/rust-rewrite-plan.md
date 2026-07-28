# Pi Dashboard Rust 重构计划

> 状态：待架构师评审，**尚未开始代码迁移**。  
> 目标：用 Rust 重写 monitor 模式，消除当前 Python 实现的周期性卡顿与过高 CPU 占用；console 模式确认删除。

## 1. 为什么现在重写

当前 Python 实现在 10 FPS 下 dashboard 进程约占用 **10% CPU**、温度约 **59°C**，且用户反馈“定期卡一下”。根因不是算法，而是运行时开销：

- **全帧刷新**：每帧都 `Image.new` + `img.copy()` + numpy RGB565 转换 + 全屏写 SPI。
- **子进程阻塞**：`docker ps`、`tailscale status`、`hostname -I` 每 2 秒在主循环路径上同步执行。
- **Python GC / PIL 分配**：大量短期 Image/ImageDraw/numpy 对象触发 GC 停顿。
- **sleep 量化**：主循环 `sleep(50ms)`，渲染只能落在 50ms 栅格上，动画不连续。

Rust 可以带来的确定性改进：

- `mmap` `/dev/fb1`，维护单一 RGB565 帧缓冲，**按 dirty rectangle 局部重绘**，不再每帧全刷。
- `tokio` 异步跑子进程 + 缓存，主循环零阻塞。
- 直接解析 `/proc/stat`、`/proc/meminfo`，零分配、无 GC。
- 位图/TTF 字体缓存，渲染路径可控。

## 2. 范围与非范围

**在范围内**

- monitor 模式的全部功能：CPU 条、温度/内存/磁盘、IP、Tailscale、容器列表、时间、触摸切换按钮。
- 帧缓冲局部刷新渲染核心。
- 触摸事件读取。
- IPC Unix socket server，兼容现有 MCP 协议（screenshot / status / switch_mode / scroll_containers）。
- systemd service 替换 Python 启动方式。

**不在范围内**

- console 模式：确认删除，代码不迁移。
- 复杂的窗口管理/多模式框架：只剩 monitor 单模式，架构可大幅简化。

## 3. 关键技术选型

| 能力 | 候选 crate | 选择 | 理由 |
|---|---|---|---|
| 异步运行时 | tokio / async-std | **tokio** | 生态最成熟，`tokio::process` 方便异步子进程，MCP 侧已是 Python 异步生态，对齐 |
| 字体渲染 | fontdue / rusttype / embedded-graphics bitmap | **fontdue + 自建 glyph cache** | 支持 TTF（DejaVu），渲染质量够，缓存后每帧只需 blit bitmap |
| 帧缓冲 | framebuffer crate / 裸 mmap | **裸 mmap + nix** | 代码少、无多余抽象，方便做 dirty rectangle 和局部更新 |
| 输入事件 | evdev crate / 裸读取 | **evdev** | 解析 `input_event` 结构稳定，支持同步/ABS 坐标 |
| PNG 截图 | image / png | **png + base64** | 体积小，满足 IPC screenshot 需求 |
| JSON IPC | serde_json | **serde_json** | 兼容现有协议 |
| 日志 | tracing / log | **tracing + tracing-subscriber** | 结构化日志，方便后续 journald 集成 |
| 错误处理 | anyhow / thiserror | **anyhow + thiserror** | anyhow 给应用入口，thiserror 给库模块 |

## 4. 预期收益（量化目标）

| 指标 | 当前 Python（10 FPS） | 目标 Rust（10–12 FPS） |
|---|---|---|
| dashboard 进程 CPU | ~10% | ≤ 4% |
| 单帧主循环耗时 | ~12–15ms（含 PIL/numpy/全刷） | ≤ 5ms（局部刷新时更低） |
| 周期性卡顿 | GC/子进程导致可见 | 消除 GC，子进程异步不阻塞 |
| 温度（同负载） | ~59°C | 预计下降 3–5°C |

> 屏幕物理上限仍是 SPI 32MHz 下的约 13 FPS，Rust 无法突破硬件，但可以把 CPU 从“接近满载”降到“轻松”。

## 5. 迁移阶段

建议**先跑通再替换**，分 6 个阶段：

1. **骨架 + 静态渲染**  
   Cargo 项目、framebuffer mmap、纯色/矩形/文字绘制、绘制 boot screen。
2. **metrics 异步采集**  
   `/proc/stat`、`/proc/meminfo`、`/sys/class/thermal/thermal_zone0/temp`、磁盘 statvfs；docker/tailscale/IP 走 `tokio::process` + cache。
3. **monitor UI + 主循环**  
   复刻现有布局，dirty rectangle 局部刷新，10 FPS 主循环。
4. **触摸 + 模式切换按钮**  
   读取 `/dev/input/event*`，识别右上角热区切换（当前只剩 monitor，按钮可保留为 future hook 或暂时显示）。
5. **IPC server + MCP 兼容**  
   实现 screenshot、status、scroll_containers，协议与现有 Python 版本一致。
6. **部署替换**  
   更新 `pi-dashboard.service` 指向 Rust binary；归档/删除 Python console 相关代码。

## 6. 风险与回退

- **字体渲染质量**：fontdue 在 10–16px 小字号下可能不如 PIL 平滑，需实测；可回退到预渲染位图字体。
- **局部刷新在 SPI 屏上的正确性**：需验证 Waveshare 35A 控制器是否能正确显示非全帧更新。若不行，则退化为全帧刷新，但仍有 Rust 本身的开销收益。
- **触摸坐标校准**：当前 Python 侧无校准参数，Rust 复刻相同读取逻辑即可；若后续加校准，需同步 touch-fix 服务。
- **构建环境**：Pi 4B 上 cargo build 较慢，建议交叉编译或 CI 构建 aarch64 binary 后下发。

## 7. 与现有 Python MCP 的关系

MCP 容器（`pi-dashboard-mcp`）只通过 Unix socket 与 dashboard 交互。只要 Rust IPC server 保持协议一致，MCP 端**无需改动**。

## 8. 下一步

等待架构师评审本计划及 `rust/ARCHITECTURE.md` 后，再决定是否进入 Phase 1。

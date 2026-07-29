# Pi Dashboard Rust 重构计划

> 状态：**架构评审已通过**（2026-07-28），评审修订已合并进 `../rust/ARCHITECTURE.md`，
> 施工依据为 `rust-coder-brief.md` + `ARCHITECTURE.md`，本文档保留为背景与目标说明。
> 目标：用 Rust 重写 monitor 模式，消除当前 Python 实现的周期性卡顿与过高 CPU 占用；console 模式确认删除。

## 1. 为什么现在重写

当前 Python 实现在 10 FPS 下 dashboard 进程约占用 **10% CPU**、温度约 **59°C**，且用户反馈“定期卡一下”。根因不是算法，而是运行时开销：

- **全帧刷新**：每帧都 `Image.new` + `img.copy()` + numpy RGB565 转换 + 全屏写 SPI。
- **子进程阻塞**：`docker ps`、`tailscale status`、`hostname -I` 每 2 秒在主循环路径上同步执行。
- **Python GC / PIL 分配**：大量短期 Image/ImageDraw/numpy 对象触发 GC 停顿。
- **sleep 量化**：主循环 `sleep(50ms)`，渲染只能落在 50ms 栅格上，动画不连续。

Rust 可以带来的确定性改进：

- 应用内 RGB565 帧缓冲 + **dirty rectangle 局部写出**，不再每帧全刷。
- `tokio`（单线程 runtime）异步跑子进程 + 缓存，主循环零阻塞。
- 直接解析 `/proc/stat`、`/proc/meminfo`，零 GC。
- fontdue glyph 缓存，渲染路径可控。

## 2. 范围与非范围

**在范围内**

- monitor 模式的全部功能：CPU 条、温度/内存/磁盘、IP、Tailscale、容器列表、时间、触摸热区。
- 帧缓冲脏区刷新渲染核心。
- 触摸事件读取。
- IPC Unix socket server，兼容现有 MCP 协议（screenshot / status / switch_mode / scroll_containers）。
- **Page 抽象**（页面注册、触摸路由、切页）：本期只实现 monitor 一个页面，但抽象必须到位。
- systemd service 替换 Python 启动方式。

**不在范围内**

- console 模式：确认删除，代码不迁移。
- 新页面的具体实现（多页面、触屏交互页）：本期不做，但**架构禁止把单页面写死**——后续加页面只允许新增 Page 实现，不允许改主循环/IPC/touch（评审新增约束）。

## 3. 关键技术选型（评审后定稿）

| 能力 | 选择 | 理由 |
|---|---|---|
| 异步运行时 | **tokio（current_thread 单线程）** | 无 CPU 密集任务，多线程 runtime 浪费功耗；同时满足 time crate local-offset 的单线程安全前提 |
| 字体渲染 | **fontdue + 运行时 glyph cache** | 支持 TTF（DejaVu），字符集小，缓存后纯 blit |
| 帧缓冲 | **std File + `write_at` 局部写** | 无 mmap/nix 依赖，代码最少；脏区合并后按行范围写出 |
| 输入事件 | **裸解析 input_event + AsyncFd** | 与 Python `touch.py` 完全同构，行为逐条对齐；不引入 evdev crate |
| PNG 截图 | **png + base64** | 满足 IPC screenshot 需求 |
| JSON IPC | **serde_json** | 兼容现有协议 |
| 日志 | **tracing + tracing-subscriber** | 结构化日志，journald 集成 |
| 错误处理 | **anyhow + thiserror** | anyhow 给应用入口，thiserror 给库模块 |

> 最终依赖树纯 Rust + libc（libc 仅为 `O_NONBLOCK` 常量），无任何系统级 C 库依赖，Pi 上原生构建零 apt 依赖。

## 4. 预期收益（量化目标）

| 指标 | 当前 Python（10 FPS） | 目标 Rust（10 FPS） |
|---|---|---|
| dashboard 进程 CPU | ~10% | ≤ 4% |
| 周期性卡顿 | GC/子进程导致可见 | 消除 GC，子进程异步不阻塞 |
| 温度（同负载） | ~59°C | 预计下降 3–5°C |

> 屏幕物理上限仍是 SPI 32MHz 下的约 13 FPS，Rust 无法突破硬件，但可以把 CPU 从“接近满载”降到“轻松”。

## 5. 迁移阶段

详见 `rust-coder-brief.md` §5，共 6 阶段：骨架+静态渲染 → metrics 双通道 → Page 框架+MonitorPage → 触摸 → IPC → 部署替换。每阶段带验收标准，先跑通再替换。

## 6. 风险与回退

- **字体渲染质量**：fontdue 小字号可能不如 PIL 平滑，需实测；回退为预渲染位图字体（只改 render.rs）。
- **局部刷新正确性**：fbtft deferred I/O 天然支持局部写；若实测异常，配置项退化全帧刷新，CPU 收益仍在。
- **部署回退**：Python 包保留不删，回退 = 恢复 service 的 ExecStart + 重启。
- **构建**：Pi 4B 原生构建，限并行 `CARGO_BUILD_JOBS=2` 控制温升。

## 7. 与现有 Python MCP 的关系

MCP 容器（`pi-dashboard-mcp`）只通过 Unix socket 与 dashboard 交互。Rust IPC server 逐字段复刻 `ipc_server.py` 协议（含 `switch_mode`），MCP 端**无需改动**。

## 8. 下一步

按 `rust-coder-brief.md` 进入 Phase 1。

# Pi Dashboard Rust 重写 —— Coder 实施简报

> 面向执行编码的 Agent。架构已评审定稿（见 `../rust/ARCHITECTURE.md`），本文档是施工说明书。
> 评审修改了原计划的几处关键点，**以本文档与 ARCHITECTURE.md 为准**，`rust-rewrite-plan.md` 仅作背景。

## 0. 运行环境约束（所有设计决策的前提）

- 硬件：树莓派 4B（aarch64），Ubuntu 24.04，计算与散热资源紧缺。
- 屏幕：微雪 3.5A SPI 屏，`/dev/fb1`，480×320，SPI 已超频 32 MHz，全屏理论上限约 13 FPS。
- 目标：功能与现有 Python 版**完全一致**，CPU 从 ~10% 降到 ≤ 4%，消除周期性卡顿，温度下降。
- 原则：**够用就好**。不引入重依赖、不造通用框架、不预先实现未要求的功能；但扩展点（Page 抽象）必须留好，禁止把单页面写死。

## 1. 动手前必读（行为与协议的 source of truth）

按顺序阅读，Rust 版逐项复刻其行为：

1. `../config.py` —— 全部布局/颜色/渐变/间隔常量、`TOUCH_DEVICES` 设备优先级列表。
2. `../monitor_mode.py` —— monitor 页面布局、静态背景 + 慢变缓存思路、容器列表分页/滚动逻辑。
3. `../metrics.py` —— CPU 滑动窗口平滑算法、温度/内存/磁盘/IP/Tailscale 采集方式与缓存间隔。
4. `../render.py` / `../fonts.py` —— 渐变 LUT、字体加载与字号。
5. `../touch.py` —— 触摸事件语义（见 §4.4，必须逐条对齐）。
6. `../ipc_server.py` —— IPC 协议精确格式（见 §4.5，逐字段对齐）。
7. `/home/richli/pi-dashboard-mcp/src/pi_dashboard_mcp/ipc_client.py` —— 消费方，验证兼容性时对照。

`panel.py`、`console_mode.py`、`__main__.py` 不需要复刻（模式框架由 Page 抽象取代，console 模式删除）。

## 2. 工程环境搭建（第一步）

```bash
# 校园网建议先走清华镜像
export RUSTUP_DIST_SERVER=https://mirrors.tuna.tsinghua.edu.cn/rustup
export RUSTUP_UPDATE_ROOT=https://mirrors.tuna.tsinghua.edu.cn/rustup/rustup
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
source "$HOME/.cargo/env"
```

- **直接在 Pi 上原生构建**，不折腾交叉编译（`.cargo/config.toml` 里的 linker 配置仅备用）。
- crates.io 拉取慢时，启用 `.cargo/config.toml` 中已备好的清华 sparse 镜像注释段。
- 构建限并行降温：`CARGO_BUILD_JOBS=2 cargo build --release`（或 `nice -n 19`）。
- 无需任何 apt 系统依赖（最终依赖树纯 Rust + libc）。
- 安装 rustup 于 `~/.cargo`，属于用户目录，无需 sudo，符合规范。

## 3. 依赖清单（已锁定，未经说明不得新增）

见 `rust/Cargo.toml`。要点：

- tokio 用 **current_thread 单线程 runtime**（feature 只有 `rt` 没有 `rt-multi-thread`）。time crate 的 `local-offset` 只在单线程下安全，这是硬约束。
- 无 evdev crate、无 nix、无 mmap：framebuffer 用 `std::fs::File` + `FileExt::write_at`；触摸用裸 fd 手动解析 `input_event`（与 Python `touch.py` 完全同构）。libc 仅为 `O_NONBLOCK` 常量保留。

## 4. 必须遵守的实现约束

### 4.1 可扩展性（开闭原则，本次评审新增的最高优先级要求）

- 实现 `pages/mod.rs` 中的 `Page` trait + `PageManager`（接口见 ARCHITECTURE.md §4.2）。
- 当前只实现 `MonitorPage` 一个页面，但**页面注册、触摸路由、IPC `switch_mode` 必须全部走 PageManager**，main.rs 不出现任何 monitor 专属逻辑。
- 未来加页面 = 新增一个文件 + 在注册表加一行，不改主循环、不改 IPC、不改 touch。
- 触摸热区由各 Page 内部处理；不要造通用 widget 框架，Page trait 就是全部抽象。

### 4.2 功耗与性能

- 渲染脏区驱动：每帧结束合并 dirty rect，**无脏区则完全不写 /dev/fb1**。
- 局部写用 `write_at` 按行范围合并写出；保留 `full_flush()` 用于初始化、切页、异常恢复。
- 主循环 `tokio::time::interval(100ms)`，`MissedTickBehavior::Delay`。
- 慢速指标（docker/tailscale/IP）间隔与 Python `config.py` 一致，不得提高频率。
- 子进程统一包 `tokio::time::timeout`（docker 3s、tailscale 5s），超时/失败保留旧缓存，UI 显示旧数据，记 warn 日志。

### 4.3 健壮性

- 运行时路径禁止 `unwrap/expect/panic`（main 初始化除外）；任何单帧错误 log + 跳过该帧。
- `/dev/fb1` 打开失败：退避重试（0.5s 起，封顶 5s），进程不退出。
- 触摸设备不存在：禁用触摸继续运行，touch 任务内部自持重试循环。
- IPC 畸形 JSON：回 `{"status":"error","message":...}`，listener 不中断。
- 建议加 `#![warn(clippy::unwrap_used)]` 辅助自查。

### 4.4 触摸语义（逐条复刻 touch.py）

- 设备选择：按 `TOUCH_DEVICES` 顺序取第一个存在的（优先虚拟校准设备 event1，回退 event0）。
- 解析 `input_event`，aarch64 布局等同 Python `struct.Struct("llHHI")`（24 字节）。
- 若 `/etc/pointercal` 存在（7 个浮点），按同一公式做校准映射；否则原样传递并钳制到 479×319。
- 事件语义：按下期间坐标变化实时上报 pressed 事件；抬起时用最后坐标上报一次 release 事件。
- 非阻塞读取：fd 置 `O_NONBLOCK`，包 `tokio::io::unix::AsyncFd`，事件经 mpsc 送主循环。

### 4.5 IPC 协议（逐字段复刻 ipc_server.py）

- socket：`/var/lib/pi-dashboard/pi_dashboard.sock`，bind 前删旧文件，chmod 0666，listen backlog 4。
- 逐行 JSON：每连接读到第一个 `\n` 为止，处理一个请求，回一行 JSON + `\n`，关闭。
- action 集：`screenshot` → `{"status":"ok","data":"<base64 png>"}`；`status` → `{"status":"ok","ips":[...],"tailscale":"..."}`；`switch_mode`（带 `mode` 字段，映射为 PageManager 切页，未知 mode 回 error，**必须保留**）；`scroll_containers` → `{"status":"ok","offset":N,"total":M}`；未知 action 回 error。
- screenshot：锁 framebuffer、克隆 buffer、**解锁后再编码** PNG（RGB565→RGB888），编码不占用渲染锁。

## 5. 实施阶段与验收

| 阶段 | 内容 | 验收标准 |
|---|---|---|
| 1 | 骨架 + fb.rs + render.rs 原语 + boot 画面 | `cargo build --release` 零警告；屏幕显示测试画面 |
| 2 | metrics.rs（快/慢双通道，watch channel 发布） | 数据与 Python 版一致；慢速刷新不阻塞 10 FPS 节拍 |
| 3 | pages 框架 + MonitorPage 完整布局 + 主循环 | 视觉与 Python 版一致；无脏区帧零 SPI 写入 |
| 4 | touch.rs + 容器列表滚动 + 切页按钮热区 | 触摸响应与 Python 版一致 |
| 5 | ipc.rs + screenshot.rs | §6 兼容性测试全部通过 |
| 6 | systemd 替换部署 | §6 部署步骤完成，回退路径验证过 |

每阶段完成后自测再进下一阶段；发现架构文档与现实冲突时，停下来在报告中提出，不要自行改架构。

## 6. 验证与部署（Phase 6 必做）

```bash
# 1. IPC 四 action 逐一验证（响应字段与 Python 版完全一致）
python3 - <<'EOF'
import json, socket
for req in [{"action":"status"},{"action":"screenshot"},
            {"action":"scroll_containers"},{"action":"switch_mode","mode":"monitor"},
            {"action":"bogus"}]:
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect("/var/lib/pi-dashboard/pi_dashboard.sock")
    s.sendall((json.dumps(req)+"\n").encode())
    data = b""
    while not data.endswith(b"\n"): data += s.recv(65536)
    r = json.loads(data.decode().strip()); s.close()
    print(req["action"], "->", {k: (v[:60]+"..." if isinstance(v,str) and len(v)>60 else v) for k,v in r.items()})
EOF

# 2. MCP 端到端：确认容器侧工具正常
docker exec pi-dashboard-mcp python3 -c "..."  # 或直接观察 AstrBot 调用 pi_get_dashboard_screenshot

# 3. 性能实测（连续 10 分钟）
pidstat -p $(pgrep pi-dashboard-rust) 60 10   # CPU 均值应 ≤ 4%
cat /sys/class/thermal/thermal_zone0/temp     # 与 Python 版 59°C 基线对比
```

部署步骤：

1. 备份：复制现有 service 文件与 `~/pi_dashboard` Python 包清单到 `~/pi_dashboard/backups/`（带时间戳），**Python 代码保留不删**，作为回退。
2. `sudo cp target/release/pi-dashboard-rust /usr/local/bin/`。
3. 修改 `pi-dashboard.service` 的 `ExecStart` 指向 Rust 二进制（保留 `StateDirectory=pi-dashboard`、`Restart=always`），`daemon-reload` 后重启。
4. 验证服务重启后屏幕、IPC、MCP 均正常；在报告中写明回退命令（恢复 ExecStart + 重启即可）。
5. **不执行任何 git 提交/推送**，版本操作由用户决定。

## 7. 交付报告格式

完成后在回复中给出：各阶段验收结果、CPU/温度实测数据、IPC 测试输出、部署与回退命令、遇到的架构冲突或偏差及处理方式。

# pi-dashboard-rust

Pi Dashboard 的 Rust 实现，当前主代码。

架构与整改记录：

- `ARCHITECTURE.md` —— 模块边界、数据流、已定稿的设计决策
- `../docs/rust-coder-brief.md` —— 实施简报（环境搭建、约束、阶段验收）
- `../docs/rust-render-rework-directive.md` —— 渲染层整改指令
- `../docs/rust-render-rework-report.md` —— 整改验收报告

## 范围

- 仅实现 **monitor 页面**（console 模式已删除），但页面能力通过 Page 抽象注册，未来可扩展多页面/触屏交互。
- 通过 Unix socket 逐字段兼容 `pi-dashboard-mcp` 的 IPC 协议。

## 环境准备（在 Pi 上原生构建，推荐）

```bash
# 1. 安装 Rust（校园网建议先设清华镜像）
export RUSTUP_DIST_SERVER=https://mirrors.tuna.tsinghua.edu.cn/rustup
export RUSTUP_UPDATE_ROOT=https://mirrors.tuna.tsinghua.edu.cn/rustup/rustup
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
source "$HOME/.cargo/env"

# 2. 无需任何 apt 系统依赖（依赖树纯 Rust + libc）
# crates.io 慢时启用 .cargo/config.toml 中的清华 sparse 镜像注释段
```

## 构建

```bash
cd /home/richli/pi_dashboard/rust

# 限并行降温（Pi 4B 散热紧张）
CARGO_BUILD_JOBS=2 cargo build --release

# 交叉编译（备用，在 x86 机器上）：
# sudo apt-get install -y gcc-aarch64-linux-gnu && rustup target add aarch64-unknown-linux-gnu
# cargo build --release --target aarch64-unknown-linux-gnu
```

## 运行

```bash
# 直接运行（需要 /dev/fb1、/dev/input/event* 权限）
sudo ./target/release/pi-dashboard-rust

# 或部署为 systemd 服务（替换现有 pi-dashboard.service 的 ExecStart）
sudo cp target/release/pi-dashboard-rust /usr/local/bin/pi-dashboard-rust
sudo systemctl restart pi-dashboard.service
journalctl -u pi-dashboard.service -f
```

## 调试

```bash
# IPC 四 action 冒烟测试（响应格式须与 Python 版一致）
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

# 日志级别
RUST_LOG=debug sudo ./target/release/pi-dashboard-rust
```

## 目录说明

- `src/main.rs`：tokio `current_thread` 入口，组装各模块、主循环
- `src/config.rs`：常量、颜色、渐变 LUT、刷新间隔、字体路径
- `src/fb.rs`：RGB565 帧缓冲 + 脏区合并 + `write_at` 局部刷新
- `src/render.rs`：几何原语（矩形、线、椭圆）与 RGB565 混合
- `src/text.rs`：字体级 baseline 锚定文本引擎，支持 Regular/Medium 字重
- `src/label.rs`：`Label` / `Bar` 字段控件，值变化才擦除重绘
- `src/metrics.rs`：快/慢双通道指标采集
- `src/pages/`：`Page` trait、`PageManager` 与 `MonitorPage`
- `src/touch.rs`：裸解析 `input_event`，语义对齐 Python `touch.py`
- `src/ipc.rs`：Unix socket server，逐行 JSON oneshot 协议
- `src/screenshot.rs`：RGB565 → RGB888 → PNG → base64

详见 `ARCHITECTURE.md`。

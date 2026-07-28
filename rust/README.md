# pi-dashboard-rust

Pi Dashboard 的 Rust 重写版本（当前为架构/环境准备阶段，尚未开始业务代码迁移）。

## 范围

- 仅保留 **monitor 模式**。
- **console 模式删除**，不再迁移。
- 通过 Unix socket 兼容现有 `pi-dashboard-mcp` 的 IPC 协议。

## 环境准备

```bash
# 1. 安装 Rust（如果尚未安装）
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"

# 2. 添加 aarch64 目标（在 x86 开发机上交叉编译时使用）
rustup target add aarch64-unknown-linux-gnu

# 3. 安装系统依赖（Pi / Debian 类系统）
sudo apt-get update
sudo apt-get install -y libevdev-dev pkg-config
```

## 开发/构建

```bash
cd /home/richli/pi_dashboard/rust

# 本地检查（Pi 上）
cargo check

# 发布构建（Pi 上，较慢）
cargo build --release

# 交叉构建（推荐：在 x86 机器或 CI 上构建后下发）
# 需要配置 linker，见 .cargo/config.toml
cargo build --release --target aarch64-unknown-linux-gnu
```

## 运行

```bash
# 直接运行（需要 /dev/fb1、/dev/input/event* 等权限）
sudo ./target/release/pi-dashboard-rust

# 或作为 systemd 服务
sudo systemctl enable --now /path/to/pi-dashboard-rust.service
```

## 调试

```bash
# 查看日志
journalctl -u pi-dashboard-rust -f

# 测试 IPC status
python3 - <<'EOF'
import json, socket
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.connect("/var/lib/pi-dashboard/pi_dashboard.sock")
s.sendall(b'{"action": "status"}\n')
data = b""
while not data.endswith(b"\n"):
    data += s.recv(4096)
print(json.loads(data.decode().strip()))
EOF
```

## 目录说明

- `src/main.rs`：入口占位，业务代码待评审后填充。
- `src/config.rs`：常量与主题配置（待填充）。
- `src/fb.rs`：framebuffer mmap 与局部刷新（待填充）。
- `src/render.rs`：字体缓存与绘制（待填充）。
- `src/metrics.rs`：系统指标读取（待填充）。
- `src/ipc.rs`：Unix socket server（待填充）。
- `src/touch.rs`：触摸输入（待填充）。

详见 `ARCHITECTURE.md`。

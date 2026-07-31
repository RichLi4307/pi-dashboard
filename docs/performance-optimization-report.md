# Pi Dashboard CPU 周期波动优化报告

> 分析时间：2026-07-31  
> 涉及文件：`rust/src/config.rs`、`rust/src/metrics.rs`、`rust/src/pages/monitor.rs`  
> 部署目标：`FocusRasPi4B`（树莓派 4B，aarch64）

---

## 1. 问题定位（基于已有采样）

用户通过 `pidstat 1 120` + `mpstat -P ALL 1 120` 采样发现：

- 最规律的周期波动 ≈ **2 秒**，与面板 `SLOW_DATA_INTERVAL = 2.0s` 完全吻合。
- 尖峰主要由 `dockerd` / `containerd` 产生，来源是面板每 2 秒执行 `docker stats --no-stream` 采集容器 CPU%。
- 面板进程自身 `pi-dashboard-ru` 平均仅约 2%，不是主因。
- AstrBot 群聊消息造成更大的非周期性尖峰，属于业务负载，不在面板优化范围。

原始数据：

| 进程 | 平均 CPU | 最大 CPU | 尖峰间隔 |
|---|---|---|---|
| dockerd | 25.5% | 74% | ~2s |
| containerd | 25.0% | 69% | ~2s |
| pi-dashboard-ru | ~2.1% | 4% | - |

---

## 2. 优化方案与实施

### 2.1 降低慢速刷新频率

`rust/src/config.rs`：

```rust
pub const SLOW_DATA_INTERVAL: f32 = 5.0;   // 原为 2.0
```

IP、Tailscale、容器列表、磁盘等数据不需要 2 秒一刷，5 秒对监控面板足够实时。

### 2.2 容器 CPU% 从 `docker stats` 改为 cgroup v2

`rust/src/metrics.rs`：

- `ContainerInfo` 新增 `id: String` 字段。
- `docker ps` 使用 `--no-trunc` 获取完整 64 位容器 ID。
- 新增 `ContainerCpuSampler`，读取 `/sys/fs/cgroup/system.slice/docker-<id>.scope/cpu.stat` 中的 `usage_usec`。
- 通过 `(usage_now - usage_prev) / wall_time` 计算 CPU%。
- 删除 `docker stats --no-stream` 调用，不再每周期 fork docker CLI。

### 2.3 容器 CPU 显示格式化

`rust/src/pages/monitor.rs`：

- cgroup 计算可能得到 >100%（多核累计），显示时限制为 `100%`，避免越出 4 字符列宽。
- 数据尚未就绪时显示 `-`（GRAY），与规范一致。

---

## 3. 优化效果

### 3.1 面板进程自身

`pidstat 1 120 -u -p <pi-dashboard-pid>`：

```text
Average:     1000    304201    0.98    1.08    0.00    0.19    2.07     -  pi-dashboard-ru
```

- 平均 CPU 从 2.32% 降至 **2.07%**（本身负载就很低）。

### 3.2 Docker 子系统（系统级 pidstat）

| 进程 | 优化前平均 | 优化前最大 | 优化后平均 | 优化后最大 |
|---|---|---|---|---|
| dockerd | 25.5% | 74% | **13.99%** | **26%** |
| containerd | 25.0% | 69% | **17.64%** | **25%** |

- dockerd 平均 CPU 下降约 **45%**，峰值下降约 **65%**。
- containerd 平均 CPU 下降约 **29%**，峰值下降约 **64%**。

### 3.3 周期特征

优化前自相关主周期为 **2s**；优化后 2s 仍有一定相关性（残余 `docker ps` 开销 + 其他容器心跳），但 5s 周期更明显，与新的 `SLOW_DATA_INTERVAL` 吻合。`docker stats` 引入的密集 2s 尖峰已被消除。

---

## 4. 功能验证

- `cargo test --release`：**29/29 通过**。
- IPC 四 action（`status` / `scroll_containers` / `switch_mode` / `screenshot`）全部正常。
- 面板截图中容器 CPU% 正常显示（0.1%–2.3%），四列布局、趋势箭头、hero 卡等均未受影响。

---

## 5. 部署与回退

### 部署命令（已执行）

```bash
cd /home/richli/pi_dashboard/rust && cargo build --release

TIMESTAMP=$(date +%Y%m%d-%H%M%S)
cp /etc/systemd/system/pi-dashboard.service \
   /home/richli/pi_dashboard/backups/pi-dashboard.service.$TIMESTAMP.bak

sudo systemctl stop pi-dashboard.service
sudo cp /home/richli/pi_dashboard/rust/target/release/pi-dashboard-rust /usr/local/bin/pi-dashboard-rust
sudo systemctl start pi-dashboard.service
```

### 回退命令

```bash
LATEST=$(ls -t /home/richli/pi_dashboard/backups/pi-dashboard.service.*.bak | head -1)
sudo cp "$LATEST" /etc/systemd/system/pi-dashboard.service
sudo systemctl daemon-reload
sudo systemctl restart pi-dashboard.service
```

---

## 6. 交付物

- 本报告：`docs/performance-optimization-report.md`
- 优化后截图：`docs/screenshots/monitor-optimized-20260731-224721.png`
- 原始采样日志：`/tmp/pidstat_system_final.log`、`/tmp/mpstat_system_final.log`
- 分析输出：`/tmp/analyze_final.out`
- service 备份：`backups/pi-dashboard.service.20260731-224609.bak`

---

## 6. 追加：Docker Engine REST API 替代 `docker ps` CLI（2026-07-31 已撤回）

### 6.1 尝试

`rust/src/metrics.rs`：

- 新增 `DOCKER_SOCK` + `docker_api_get` + `read_docker_containers_api`。
- `read_docker_containers_with_cpu` 优先调用 API；失败时 `warn` 并 fallback 到 `docker ps` CLI。
- 零新增 crate。

### 6.2 验证与结论

- `cargo test --release`：**29/29 通过**。
- `pidstat 1 60` 与 `pidstat 1 120` 对比显示：API 路径确实消除了 `docker ps` CLI 进程，但 `dockerd`/`containerd` 的 5 秒周期尖峰**没有明显下降**。
- 主要瓶颈是 dockerd 遍历容器状态的内部开销，而非 CLI fork/exec。
- 在 5 秒刷新周期下，CLI fork 开销极小，不值得引入额外代码路径。

### 6.3 撤回

已恢复为原 `docker ps` CLI 路径，删除所有 Docker Engine API 辅助代码。面板回到经过验证的稳定实现。

## 7. 后续可选方向

按用户决策，保留 Docker 轮询本身，不再拆分列表刷新频率，避免损失面板功能。若未来仍需进一步压低 dockerd/containerd 波动，可考虑：

1. ~~**把 `docker ps` 也改为 Docker Engine API（`/var/run/docker.sock`）**：已评估，收益不显著，已撤回。~~
2. **分离容器列表与 CPU 采样周期**：列表 10s，CPU 2s，但会牺牲列表新鲜度。
3. **cgroup 路径枚举发现容器**：彻底绕过 dockerd，但无法直接获得容器名称/状态，需要额外映射。
4. **Docker events 事件流 + 兜底刷新**：长期最优，但长连接稳定性需充分测试。

当前方案在功能完整性与性能之间取得较好平衡。

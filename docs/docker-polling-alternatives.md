# Pi Dashboard Docker 轮询优化方案探讨

> 目标：在**不损失面板功能**的前提下，进一步降低 `dockerd` / `containerd` 因容器列表刷新而产生的周期性 CPU 尖峰。  
> 现状：已通过 `SLOW_DATA_INTERVAL=5.0s` + `cgroup v2 cpu.stat` 替代 `docker stats`，dockerd/containerd 峰值从 74%/69% 降至 26%/25%。剩余尖峰主要由每 5 秒一次的 `docker ps` 轮询造成。

---

## 0. 约束与前提

- 运行环境：Ubuntu 24.04 + Docker 29.1.3 + containerd + cgroup v2。
- 面板以 `richli` 用户运行，`richli` 已在 `docker` 组，可读写 `/var/run/docker.sock`。
- `richli`**不在** `root` 组，**不能直接访问** `/run/containerd/containerd.sock`（权限 `root:root 660`）。
- 当前代码零新增 crate，保持该纪律可降低维护风险。

---

## 方案 1：Docker Engine REST API 替代 `docker ps` CLI（推荐，中期）

### 思路
不再 `fork/exec` `docker ps` 子进程，而是直接通过 `tokio::net::UnixStream` 连接 `/var/run/docker.sock`，发送 HTTP/1.1 `GET /containers/json?all=1`，解析 JSON 得到 name/state/id。

### 优点
- **消除 fork/exec 开销**：无需启动 docker CLI 进程，减少 containerd/dockerd 压力。
- **权限现成**：`richli` 已在 docker 组。
- **零新增系统依赖**：纯 Rust + tokio，与现有技术栈一致。
- **稳定性高**：Docker Engine API 是 Docker 官方接口，长期稳定。
- **收益可预期**：dockerd 处理本地 unix socket REST 请求比处理 CLI 子进程轻量，预计 dockerd 负载再降 20–40%。

### 缺点
- 仍经过 dockerd，无法彻底消除 dockerd 尖峰。
- 需要手写极简 HTTP/1.1 client 或引入轻量 HTTP crate（如 `hyper`/`ureq`/`minreq`），会新增依赖。
- 需要维护连接复用（keep-alive）才能获得最佳性能。

### 实现要点
```rust
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

async fn docker_api_containers() -> Option<Vec<ContainerInfo>> {
    let mut stream = UnixStream::connect("/var/run/docker.sock").await.ok()?;
    let req = "GET /containers/json?all=1 HTTP/1.1\r\nHost: localhost\r\nConnection: keep-alive\r\n\r\n";
    stream.write_all(req.as_bytes()).await.ok()?;
    // read headers + body, parse JSON
}
```

### 风险
**低**。接口稳定，权限已具备，改动范围小。

---

## 方案 2：containerd API / CRI（性能最优，但权限成本高）

### 思路
绕过 dockerd，直接通过 gRPC 调用 containerd 的 `/run/containerd/containerd.sock`，使用 `containers`/`tasks` 服务获取容器列表、状态、PID。

### 优点
- **性能最好**：彻底绕过 dockerd，直接访问运行时。
- **权威数据**：containerd 是容器实际管理者，状态最准确。

### 缺点
- **权限问题**：`containerd.sock` 属 `root:root 660`，面板进程无法访问。解决方式：
  - 把 `richli` 加入 `root` 组（**不推荐**，扩大攻击面）。
  - 修改 `containerd.sock` 权限或加 ACL（**不推荐**，影响系统安全）。
  - 面板服务以 root 运行（**不推荐**）。
  - 用 systemd `SocketGroup=`/`SupplementaryGroups=` 给面板服务单独授权（较复杂，但相对安全）。
- **新增依赖**：需要 gRPC client，如 `containerd-client` 或 `tonic`。
- **与 Docker 解耦**：若未来不用 containerd，需重新适配。

### 风险
**中到高**。主要风险是权限配置；API 本身稳定，但实现复杂度和依赖增加明显。

---

## 方案 3：事件驱动 + 兜底刷新（理论最优，长期）

### 思路
打开 Docker Engine `/events` HTTP 长连接，监听 `start`/`stop`/`die`/`create`/`destroy` 等容器事件。只在事件发生时刷新容器列表；平时不轮询。同时每 30–60 秒做一次兜底全量刷新，防止连接断线期间数据陈旧。

### 优点
- **理论最优**：无事件时 dockerd 零负载（面板角度）。
- **实时性最高**：容器状态变化立即反映到面板。

### 缺点
- **长连接稳定性**：树莓派网络/电源环境不稳定，连接可能断开，需要完善的断线检测和重连。
- **事件丢失风险**：连接断开期间发生的事件会丢失，必须依赖兜底刷新。
- **CPU 数据仍需单独采样**：cgroup CPU 采样继续每 2–5 秒执行（与事件流独立）。
- **实现复杂度高于轮询**：需要维护 stream parser、心跳/超时、兜底逻辑。

### 实现要点
```rust
// GET /events?filters=%7B%22type%22%3A%5B%22container%22%5D%7D
// 流式响应：每行一个 JSON 事件对象
```

### 风险
**中**。若断线重连 + 兜底机制不完善，面板可能长时间显示 stale 数据。需要充分测试。

---

## 方案 4：cgroup 枚举 + 元数据缓存（野路子）

### 思路
1. 启动时通过一次 Docker API 或 `docker ps` 建立 `container_id → name/state` 映射表。
2. 正常运行时只扫描 `/sys/fs/cgroup/system.slice/docker-*.scope/` 目录，判断哪些容器还活着。
3. 当 cgroup 集合发生变化（新增/减少容器）时，再调用一次 Docker API 更新映射表。
4. 状态字符串（如 "Up 15 hours"）用启动时间戳自己计算，或从缓存中沿用旧值。

### 优点
- **大部分时间零 dockerd 负载**：只读 cgroup 目录即可。
- 结合已有的 cgroup CPU 采样，路径统一。

### 缺点
- **实现复杂**：需要维护 id→metadata 缓存、集合差分、状态推断。
- **状态信息不完整**：cgroup 只能告诉你容器是否存在，无法直接得到 `exited`、`restarting`、`paused` 等状态；需要额外逻辑。
- **兜底刷新仍需要**：Docker API 或 `docker ps` 偶尔仍需调用。

### 风险
**中**。缓存一致性和状态推断是主要挑战。

---

## 方案 5：文件系统/进程监控（不推荐）

### 5a. inotify `/var/lib/docker/containers/`
- **不可行**：该目录需要 root 权限，且 Docker 内部结构可能变化。
- 只能检测容器创建/删除，无法准确反映 running/exited 状态。

### 5b. 扫描 `/proc` 中的 `containerd-shim`
- **不可行/脆弱**：exited 容器没有 shim 进程；从 shim 反向映射到容器 name/state 复杂且不稳定。

### 风险
**高**。不推荐作为正式方案。

---

## 方案 6：继续加大轮询间隔（最简单，已部分实施）

### 思路
把 `SLOW_DATA_INTERVAL` 从 5s 进一步加大到 10s/30s。容器列表变化不频繁，对监控面板来说 10–30 秒延迟可接受。

### 优点
- **最简单**：改一个常量即可。
- **最稳定**：无新增逻辑。

### 缺点
- **不能根治**：dockerd 尖峰仍在，只是频率降低。
- **损失实时性**：用户明确不希望损失功能性，30 秒容器状态延迟可能难以接受。

### 风险
**低**。但收益边际递减。

---

## 综合评估

| 方案 | 性能提升潜力 | 实现复杂度 | 稳定性 | 权限/依赖成本 | 推荐度 |
|---|---|---|---|---|---|
| 1. Docker Engine REST API | 中（再降 20–40%） | 低 | 高 | 低 | **★★★★★** |
| 2. containerd API | 高 | 高 | 中 | 高 | ★★★☆☆ |
| 3. Docker events + 兜底 | 很高 | 中 | 中 | 低 | ★★★★☆ |
| 4. cgroup 枚举 + 缓存 | 高 | 高 | 中 | 低 | ★★★☆☆ |
| 5. fs/proc 监控 | 中 | 高 | 低 | 高 | ★☆☆☆☆ |
| 6. 继续加大间隔 | 低 | 极低 | 高 | 无 | ★★★☆☆ |

---

## 建议路线

1. **已实施**：`SLOW_DATA_INTERVAL=5.0s` + `cgroup v2 CPU%`。dockerd/containerd 峰值已大幅下降。
2. **下一步建议**：**方案 1（Docker Engine REST API）**。在保持功能完整、权限不变、依赖可控的前提下，进一步消除 fork/exec 开销。这是投入产出比最高的路径。
3. **再下一步**：若仍不满足，再考虑 **方案 3（事件驱动）** 或 **方案 2（containerd API）**，但需接受更高的实现/维护成本。

用户可据此选择方向，再进入具体实施。

# Pi Dashboard UI 规范 v4 —— 可下钻仪表盘（详情页 + 电源控制）

> 2026-07-31，架构师定稿。基于用户需求：主页加 NET/DISK IO、五个可点进去的详情页（折线图+硬件信息）、主页关机/重启按钮、IP 移入网络详情。
> 前置：v3 修正包（`ui-visual-spec-v3.md`）已合入。本文档与 v2/v3 冲突处以本文档为准。
> 实现分期见 §9；全部坐标/格式为定案，禁止自行发挥。

## 1. 信息架构

```text
monitor（主页）
 ├── 点 TEMP 卡 ──▶ temp  详情（温度折线 + 传感器信息）
 ├── 点 MEM 卡  ──▶ mem   详情（内存折线 + meminfo）
 ├── 点 DISK 卡 ──▶ disk  详情（IO 折线 + 空间/挂载信息）
 ├── 点 NET 卡  ──▶ net   详情（速率折线 + 接口/IP/Tailscale）
 ├── 点 CPU 区  ──▶ cpu   详情（总占用折线 + 硬件/负载信息）
 ├── 点 [RST]   ──▶ 确认弹窗 ──▶ systemctl reboot
 └── 点 [PWR]   ──▶ 确认弹窗 ──▶ systemctl poweroff
```

- 页面 id：`"monitor" "temp" "cpu" "mem" "disk" "net"`，全部走 `PageManager` 注册表；`main.rs` 零页面专属逻辑（开闭原则，当初预留的扩展点在此兑现）。
- IPC `switch_mode` 的 mode 集合扩展为上述 6 个；未知 id 仍回 error，其余协议不变，MCP 无需改动。
- **导航纪律**：详情页只有唯一出口——左上角返回键（回 monitor）；详情页 60 秒无触摸自动回 monitor（实现：Page 记录 `last_activity`，PageManager 每帧检查超时并切页）。
- 触摸仍走 touch-fix 校准后的 TouchEvent，tap-only，无手势。

## 2. 主页布局 v4（480×320）

```text
y=0   ┌────────────────────────────────────────────────┐
      │ FocusRasPi4B  [●TS]      00:06:48  [RST][PWR]  │ 0..32  PANEL
y=40  │ ┌TEMP───┐ ┌MEM────┐ ┌DISK───┐ ┌NET──────┐      │ 40..80 四卡 w=108
      │ │ 55C ▲ │ │ 38%   │ │ 11%   │ │ ▼1.2M   │      │ x=12/128/244/360
y=88  │ CPU0 ▓▓▓░░░  14%    CPU1 ▓▓▓▓▓  46%             │ 2×2（v3 布局不变）
      │ CPU2 ▓░░░░░  13%    CPU3 ▓▓▓░░  31%             │ 整区可点 → cpu 页
y=156 │ NAME           UPTIME   STATE    CPU            │ Docker 表（v3 布局不变）
      │ ● astrbot        9h    running   0.2%           │
y=300 │ Powered by RichLi4307   load 0.42      15 FPS   │ 底栏
      └────────────────────────────────────────────────┘
```

### 2.1 顶栏（电源按钮入场）

- host：`(12, 8)` 16 Medium CYAN（源自 `config::hostname()`，v3 已定）。
- TS chip：x=134..180，y=7（v3 样式不变，纯状态展示）。
- **[MENU]**：chip x=188..230（w=42），y=7，h=18，r9，ACCENT 描边，文字 11px **GRAY**（与 TS chip 同形同高；灰色表示未激活）。**占位按钮，当前点按无任何动作**（no-op），为未来设置页预留入口，见 §11。
- time：16 Medium WHITE，右对齐 **388**（为按钮让位，原 468）。
- **[RST]**：chip x=398..428，y=4，h=24，r6，ACCENT 描边，文字 11px WHITE。
- **[PWR]**：chip x=434..464，y=4，h=24，r6，ACCENT 描边，文字 11px **ALARM**。
- 用文字不用图标：字体是 ASCII 子集，⏻/↻ 不可用；文字按钮在 3.5 寸屏上反而更清晰（教训：不为符号破字体约束）。

### 2.2 Hero 四卡（挤一挤方案）

- x=12/128/244/360，w=108，h=40，r4，PANEL。间距 8，右缘正好 468。
- 卡内沿用 v3 居中：标签 11 GRAY 于 `(x+8, y+3)`，值 16 Medium 于 `(x+8, y+19)`。
- **TEMP**：值 `55C` + 趋势三角（v3 原子控件）。点按 → temp 页。
- **MEM**：值 `38%`。点按 → mem 页。
- **DISK**：值 `11%`（只放使用率——它是稳定可扫读信号；磁盘 IO 速率进 disk 详情页图表与信息区）。点按 → disk 页。
- **NET**：标签行 `NET` + 上三角 7×5 + 上行速率（11px GRAY）；值行 = 下三角 10×9（CYAN）+ 下行速率 16px WHITE（如 `1.2M`）。点按 → net 页。
  - 速率格式化 `fmt_rate`（§5.3）：`812B` / `12K` / `1.2M`。
- 四卡整体是触摸热区（卡片矩形即热区）。

### 2.3 CPU 区与 Docker 表

- 布局完全沿用 v3（cells 12..224/256..468，bar w=143，填充内缩 1px）。
- CPU 区整体热区（x=12..468，y=88..148）→ cpu 页。
- Docker 表不变；行触摸仍是滚动，不与详情导航冲突。

### 2.4 底栏

- 左：署名（不动）。中：`load 0.42`（/proc/loadavg 1 分钟值，11 GRAY，x=170）。右：`15 FPS`。
- IP 不再上主页——进 net 详情页（用户已批准）。

## 3. 电源确认弹窗（安全关键，约束最高优先级）

- 触发：点 [RST] / [PWR] 后进入**弹窗状态**（monitor 页内部状态机：idle → confirming(reboot|poweroff) → executing），不新建页面。
- 弹窗几何：面板 300×110 居中 (90,105)，r6，PANEL 底 + ALARM 描边；标题 `Reboot?` / `Power off?` 16 WHITE 居中 y=120；按钮：
  - [CANCEL]：x=110..210，y=160，100×32，GRAY 描边 WHITE 文字（默认项）。
  - [CONFIRM]：x=270..370，y=160，100×32，ALARM 描边 ALARM 文字。
- 约束：
  1. **必须二次确认**，禁止单击直接执行；点弹窗外区域 = 取消。
  2. **10 秒无操作自动取消**（防误触悬留）。
  3. 执行用固定参数子进程：`sudo systemctl reboot` / `sudo systemctl poweroff`，tokio timeout 5s，禁止拼接 shell。
  4. 取消/超时后：monitor 页强制全量重绘（`bg_done=false` + `mark_full_dirty`），恢复原画面。
  5. 执行中显示 `Executing...`；失败显示 `Failed` 2 秒后回 idle。

## 4. 详情页共享模板

五个详情页共用一套几何（视觉语言统一的关键）：

```text
y=0   │ [< BACK]  TITLE                                    │ 0..28
      │ ──────────────────────────────────────────────── │ 下划线 y=28
y=36  │ 55C                    current / max / min        │ 大值区 36..64
y=72  │ ┌──────────────────────────────────────────────┐ │
      │ │              折线图（456×112）                │ │ 72..184
      │ └──────────────────────────────────────────────┘ │
y=192 │ LABEL   value            LABEL   value            │ 信息区 192..296
      │ ...（两列 key-value，行高 14，11px）              │
y=300 │ 底栏（与主页一致）                                  │
```

- **返回键**：chip x=12，y=4，64×20，r6，ACCENT 描边，内容 = ASCII `<` + `BACK`（11px）。热区 = chip 矩形。**ASCII `<` 可用，禁止为此画左向三角形或换字体。**
- **标题**：16 Medium WHITE，x=88，y=6，如 `TEMPERATURE`。
- **大值**：22 Medium，语义色，x=12，y=36；右侧 11 GRAY 辅助信息（如 `max 63C / min 41C`）。
- **折线图**：x=12..468，y=72..184（456×112）。网格：25%/50%/75% 三条 ACCENT 发丝线 + 纵轴最大值标签（11 GRAY 左上）。固定量程（temp 20–90、cpu/mem 0–100）或自动量程（IO 类，取窗口内 max 经 `nice_ceil` 到 1/2/5×10ⁿ，标签显示实际最大值）。
- **信息区**：两列 key-value，行高 14：左列 label x=12 / value x=130，右列 label x=246 / value x=364；label 11 GRAY、value 11 WHITE。
- 数据纪律：折线图控件遵循 Label 同纪律——有新样本或切页进入才整体擦除重绘（每 1 Hz 至多一次，脏区 = 图表矩形）。

## 5. 五个详情页内容规格

### 5.1 temp（温度）

- 大值：当前温度（temp_band_color）；辅助：`max/min`（本会话窗口内）。
- 图表：温度历史，固定量程 20–90°C，WHITE 折线；80°C 处画 ALARM 阈值发丝线。
- 信息区：Sensor `thermal_zone0`；Throttled（`vcgencmd get_throttled`，失败显示 `n/a`）；Trend（升/降/平 + 30s Δ）；Max today（进程生命周期内峰值）。

### 5.2 cpu（处理器）

- 大值：总占用 %；辅助：`load 0.42 0.31 0.28`（1/5/15）。
- 图表：总占用历史，固定 0–100，OK 绿折线。
- 信息区：Model（/proc/cpuinfo `Hardware`，如 `BCM2711`）；Cores `4`；Governor（`scaling_governor`）；Freq（`scaling_cur_freq` → `1.5G`）；四核当前值一行（`C0..C3 14 46 13 31`）。

### 5.3 mem（内存）

- 大值：已用 %；辅助：`3.2G / 7.8G`。
- 图表：占用 % 历史，固定 0–100，OK 绿折线。
- 信息区：Total / Used / Available / Buffers / Cached / Swap（Total+Free），全部来自 /proc/meminfo，格式 `x.xG` 或 `xxxM`。

### 5.4 disk（磁盘）

- 大值：`/` 使用率 %；辅助：`11G / 29G`（statvfs）。
- 图表：**IO 速率**双线：读 CYAN、写 ORANGE，自动量程（`nice_ceil`，标签 `max 2.4M/s`）。
- 信息区：Mount `/`；FS（/proc/mounts，如 `ext4`）；Device `mmcblk0`；Size / Used / Avail；Read `2.1M/s`；Write `0.4M/s`。

### 5.5 net（网络）

- 大值：下行速率；辅助：`up 0.3K/s`。
- 图表：下行 CYAN + 上行 OK 绿双线，自动量程。
- 信息区（每行一个接口，label=接口名）：`eth0 192.168.137.10`、`wlan0 192.168.1.250`、`tailscale0 100.118.236.1`；外加 `TS ON`（Tailscale 状态）。IP 枚举复用 metrics 现有 `get_ip_list` 的过滤逻辑（排除 lo/docker*/veth*/br-*）。

## 6. 数据层规格（metrics.rs + 新 chart.rs）

1. **NET/DISK IO 采集**：`/proc/net/dev`、`/proc/diskstats`（mmcblk0 第 6/10 字段 ×512B）每 1 Hz 直接文件解析（微秒级，**禁止为采集频率起子进程**）；速率 = 差值/实际 Δt。
2. **历史环形缓冲**：`History` 结构，8 条序列（temp、cpu_total、mem_pct、disk_pct、net_down、net_up、disk_r、disk_w），每条 `VecDeque<f32>` 容量 120（≈2 分钟），1 Hz 压入；内存 < 4KB。详情页只读，主页无感。
3. **`fmt_rate`**：`<1024` → `{n}B`；`<10K` → `{:.1}K`；`<1M` → `{:.0}K`；否则 `{:.1}M`。配单测（边界 1023/1024/10240/1M）。
4. **`nice_ceil`**：自动量程取整到 1/2/5×10ⁿ。配单测。
5. **`render.rs` 新增 `draw_line(x1,y1,x2,y2,color)`**（Bresenham，写前比较、诚实标脏）+ 单测。
6. **`chart.rs` 新增 `LineChart` 控件**：持有 rect、量程模式（Fixed/Auto）、系列颜色表；`set(series)` 数据不变零操作，变化先擦（BG）后画（网格→阈值线→折线），脏区=图表矩形。
7. 大值 22px 字号加入字体缓存预热列表。

## 7. 触摸规格

| 热区 | 矩形 | 动作 |
|---|---|---|
| TEMP/MEM/DISK/NET 卡 | 各自卡片矩形 | Switch 对应详情页 |
| [MENU] | (188,7)..(230,25) | **no-op（占位预留）** |
| CPU 区 | (12,88)..(468,148) | Switch cpu 页 |
| [RST] / [PWR] | (398,4)..(428,28) / (434,4)..(464,28) | 进入确认弹窗 |
| 弹窗 CANCEL / CONFIRM | (110,160)..(210,192) / (270,160)..(370,192) | 取消 / 执行 |
| 弹窗外任意处 | — | 取消 |
| 详情页 [< BACK] | (12,4)..(76,24) | Switch monitor |
| 详情页其他位置 | — | 仅重置 60s 自动返回计时 |
| Docker 列表 | （v3 原样） | 滚动 |

- 最小触摸目标：导航/按钮 ≥ 44×24；电源确认按钮 ≥ 100×32（已满足）。
- 触摸热区表集中定义在各 Page 内；主页热区不与 Docker 滚动区重叠。

## 8. 边界约束（历次返工教训固化，违反即返工）

1. **文本锚定**：只用字体级行度量；同行共享 baseline；禁止字符串级锚定。
2. **擦除纪律**：值变化才擦除重绘，先擦后画；任何字段禁止无擦除叠画。
3. **脏区诚实**：无 blanket `mark_dirty`；fill 系原语写前比较；图表每 1 Hz 至多一个矩形脏区。
4. **字体约束**：ASCII 子集不可破；箭头/图标一律几何原语或 ASCII（`<`、`!`）。
5. **语义色**：只用 `config.rs` 语义常量，禁止裸色号；非警告不用警告色（电源按钮/确认键的 ALARM 是真危险语义，合法）。
6. **对齐**：右/居中对齐一律 `measure` 实宽；禁手估字符宽。
7. **性能**：CPU ≤ 4%（详情页同预算）；IO 采集走 /proc 文件解析不走子进程；历史缓冲 1 Hz；详情页空闲时除 1 Hz 图表外零绘制。
8. **开闭原则**：新页面 = 新文件 + 注册表一行；main.rs/PageManager 不感知具体页面（60s 自动返回是 PageManager 通用机制，不写页面名）。
9. **电源安全**：§3 五条约束逐条落实。
10. **协议兼容**：IPC 四 action 语义不变，仅 mode 集合扩展；MCP 零改动验收。
11. **测试**：`fmt_rate`/`nice_ceil`/`draw_line`/LineChart 擦除与脏区/弹窗状态机/60s 自动返回/新页面注册，全部单测；更新黄金图（主页 + 至少 temp 详情页）。
12. **零新增 crate**（Bresenham、环形缓冲、弹窗全部手写，体量都很小）。

## 9. 实施分期

- **Phase A**（前置，已派发）：v3 修正包合入并部署。
- **Phase B**：数据层（IO 采集、History、fmt_rate/nice_ceil）+ `draw_line` + `LineChart` + 五个详情页 + 导航/自动返回。验收：五页可点进点出、折线正确、60s 自动返回。
- **Phase C**：顶栏电源按钮 + 确认弹窗 + 执行。验收：误触防护（点外取消、10s 超时）、reboot/poweroff 实机验证（由用户择机执行）。
- **Phase D**：黄金图更新、性能复测（主页与详情页 pidstat ≤4%）、部署、文档收尾（AGENTS.md/CHANGELOG）。

## 10. 验收清单

- [ ] 主页四卡（TEMP/MEM/DISK/NET）齐整，NET 卡下行大值+上行小行，右缘 468
- [ ] 顶栏 [RST]/[PWR] 位置正确且不挤压 time/TS chip
- [ ] 五个详情页：模板几何一致、折线正确（量程/网格/阈值线）、信息区数据真实
- [ ] IP 出现在 net 详情页；主页底栏显示 load
- [ ] 弹窗：点外取消、10s 超时、CONFIRM 才执行；执行命令固定参数带 timeout
- [ ] 60s 无触摸自动回主页；IPC `switch_mode` 接受六个页面 id、未知 id 报错
- [ ] `cargo test --release` 全绿；pidstat 主页/详情页均 ≤4%；截图 A/B 交付

## 11. 预留与折叠决策（架构师先斩后奏，2026-07-31）

### 11.1 本期落地的预留

- **[MENU] 占位按钮**（§2.1）：no-op，灰色文字标识未激活。它是未来 `settings` 页的入口，热区已划定（§7），实现设置页时只需：新建 `pages/settings.rs` + 注册表一行 + 把 MENU 热区动作改为 `Switch("settings")` + 文字转 WHITE。
- **保留页面 id**：`"settings"`（设置页）、`"container"`（容器详情页）为保留名，本期不实现；IPC `switch_mode` 对其仍按未知 id 报错，实现后自然生效，协议无需再改。
- **`Page::on_leave`**：trait 增加默认空实现的 `on_leave()`，PageManager 切页时调用。本期页面无需用到，但容器详情页（停定时器、释放大缓存）等未来页面会需要——先留钩子，零成本。

### 11.2 折叠决策（主页只做"可扫读"，其余一律进子页）

| 元素 | 决策 | 理由 |
|---|---|---|
| IP 列表 | 已折入 net 详情页 | 用户已批准 |
| 磁盘 IO 速率 | 折入 disk 详情页 | 主页 DISK 卡只留使用率（稳定信号） |
| FPS | **暂留主页底栏**，settings 页落地后迁入其 About 区 | 当前调优期需要它；底栏暂无更优内容 |
| load 均值 | 留主页底栏 | 一字符宽的负载信号，可扫读，不占视觉 |
| TS 状态 | 留主页 chip | 用户明确要求 |
| 背光/亮度、自动熄屏 | 未来 settings 页（gpio=18 背光可PWM） | 与"功耗优先"路线一致，但不在本期 |
| 容器操作（restart/logs） | 未来 container 详情页（点行进入；届时滚动改由列表两侧热区承担） | 重操作必须有确认弹窗，沿用 §3 模式 |

### 11.3 未来 settings 页内容清单（仅备案，不实现）

About（版本/FPS/uptime）、背光亮度与自动熄屏、主题强调色、主机名显示来源。届时按 §4 模板几何搭建，不新增视觉语言。

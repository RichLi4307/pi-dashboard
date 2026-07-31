# Pi Dashboard 视觉语言规范（monitor 页整改指令）

> 架构师/UI 定稿，2026-07-31。来源：用户评审反馈（温度趋势、语义配色、状态缩写、行间对比度、视觉语言统一）。
> 本文档色号与坐标均为**定案**，coder 照此实施；只改 `config.rs` / `monitor.rs`（必要时 `metrics.rs` 加解析函数），不动架构、不加依赖。

## 1. 设计原则（统一视觉语言的总纲）

**颜色即语义，不作装饰。** 全屏只允许四类颜色角色：

| 角色 | 用途 | 色号 |
|---|---|---|
| 中性 | 数值、正文（WHITE `#e6edf3`）；标签、次要信息（GRAY `#7d8590`） | 现有 |
| 标识 | 静态身份信息：主机名、IP（CYAN `#39c5cf`） | 现有 |
| 状态 | OK=GREEN `#3fb950`；CAUTION=AMBER `#d29922`；过渡橙=ORANGE `#f0883e` | 现有 |
| 报警 | ALARM **`#ff0000`**（用户指定，取代 `#f85149` 作为唯一报警色） | **新增常量** |

铁律：**不表示警告的文字一律不使用警告色**。数值默认中性色（WHITE），只有突破阈值才变色（见 §3/§4 阈值表）。已停容器、正常运行时间等不是警告，不得用红/黄。

在 `config.rs` 新增语义别名常量，页面代码只许引用语义名，禁止散落裸色号：

```rust
pub const OK: u32 = GREEN;          // 0x3fb950
pub const CAUTION: u32 = YELLOW;    // 0xd29922
pub const ALARM: u32 = 0xff0000;    // 新增，唯一报警色
pub const COOL: u32 = BLUE;         // 0x58a6ff，低温/信息蓝
pub const ROW_STRIPE: u32 = 0x131a24;   // 斑马条，介于 BG 与 PANEL 之间
pub const SCROLL_TRACK: u32 = 0x21262e; // 滚动轨道
```

> 关于"50°C 是否 `#0000FF`"：纯蓝 `#0000FF` 在深色底上亮度过低、几乎不可读，否决。低温用调色板蓝 `#58a6ff`。

## 2. 温度：分档 + 趋势箭头 + 报警标志

### 2.1 分档配色（替换 TEMP_GRADIENT/TEMP_COLOR_LUT，删除渐变）

Pi 4B 85°C 降频，原 25–90 渐变在 56°C 常态区几乎不变色、报警区过晚。改为硬分档：

| 温度 | 颜色 | 语义 |
|---|---|---|
| < 50°C | COOL `#58a6ff` | 偏凉 |
| 50–64 | WHITE | 正常工况（本机常态 ~56°C） |
| 65–74 | CAUTION `#d29922` | 偏热，关注 |
| 75–79 | ORANGE `#f0883e` | 高热，预警 |
| ≥ 80 | ALARM `#ff0000` + 报警标志 | 报警 |

实现为 `pub fn temp_band_color(t: i32) -> u32`，配单元测试覆盖边界（49/50/64/65/74/75/79/80）。

### 2.2 趋势箭头

- **采样**：温度沿用快通道每帧读取（~15 Hz）；`MonitorPage` 在 1 Hz 慢节拍把当前温度压入 60 秒环形历史。
- **判定**：当前值 vs 30 秒前样本，死区 ±1.0°C：Δ ≥ +1.0 升、≤ −1.0 降、其余平。常量：`TEMP_TREND_WINDOW_SECS=60`、`TEMP_TREND_COMPARE_SECS=30`、`TEMP_TREND_DEADBAND=1.0`。
- **绘制**：字体为 ASCII 子集，**没有箭头字符，禁止为此换字体**。用几何原语画 7×5 小三角（升=上三角、降=下三角、平=短横杠 7×2），位置紧跟温度值右侧 4px，垂直居中对齐数值 baseline 中线。
- **颜色**：与温度值同档色。历史不足 30 秒（启动初期）不画。
- 做成与 Label 同纪律的小控件（记录上次状态与 bbox，状态不变零操作，变化先擦后画），不污染脏区机制。

### 2.3 报警标志

≥ 80°C 时在趋势箭头右侧追加 ALARM 色 `!`（ASCII，字体里有）；< 80 擦除。与趋势箭头合并为同一个 `TempTrend` 控件即可。

## 3. 用量类指标（CPU/MEM/DISK）配色

- 数值文字：`pub fn usage_text_color(pct: i32) -> u32` —— < 80% WHITE；80–89% CAUTION；≥ 90% ALARM。配边界单测。
- CPU 百分比 Label、MEM/DISK 值 Label 全部改用此函数（替换 `usage_color` 里直接套 LUT 的做法）。
- **CPU 进度条填充保留 USAGE_COLOR_LUT 渐变**——仪表条是量表惯例，不违反"非警告不用警告色"。

## 4. 容器列表

### 4.1 STATUS 列缩写（`metrics.rs` 新增 `abbreviate_status(&str) -> String` + 单测）

| docker 原始 | 显示 |
|---|---|
| `Up 45 seconds` / `Up 1 second` | `45s` / `1s` |
| `Up N minutes/hours/days/weeks` | `Nm` / `Nh` / `Nd` / `Nw` |
| `Up N months` / `Up N years` | `Nmo` / `Ny` |
| `Up About an hour` | `1h`；`Up Less than a second` | `0s` |
| `Up 2 hours (healthy)` | `2h`（丢弃 healthy 后缀） |
| `Up 2 hours (unhealthy)` | `2h`，且 STATE 圆点与文字转 ALARM |
| `Exited (0) 3 hours ago` | `Ex0 3h`（其他退出码同理 `Ex137 5m`） |
| `Restarting (1) 5 seconds ago` | `Rst 5s` |
| `Created` / `Paused` | `New` / `Paus` |
| 其他无法解析 | 原样，按列宽截断 |

STATUS 列文字固定中性 GRAY（运行时间不是警告）；unhealthy 的报警表达只落在 STATE 列与圆点。

### 4.2 STATE 列配色（替换现有 match）

| state | 颜色 | 理由 |
|---|---|---|
| running | OK | 健康 |
| exited | **GRAY** | 正常终止，非警告（原 RED 违反铁律，必改） |
| created / paused / restarting | CAUTION | 过渡态 |
| dead，或 status 含 `(unhealthy)` | ALARM | 真异常 |

圆点颜色与 STATE 文字一致。

### 4.3 行间区分：斑马条 + 结构线（像素级定案）

docker 区块视觉边界统一为 **x=4..476，y=108..286**；任何装饰元素不得超出此范围，也不得压表头（y=108）与底栏（y≥300）。

- **斑马条**：10 个行带 `y = 126 + 16*i`（i=0..9）高 16，奇数行填 `ROW_STRIPE`，偶数行保持 `BG`；x 范围 4..470。在静态背景阶段绘制，零每帧开销。
- **Label 背景同步**：奇数行的 name/status/state 三个 Label 的 `bg` 必须设为 `ROW_STRIPE`（否则 clear 会在斑马条上打出 BG 色洞）。这是最容易漏的点。
- **表头下划线**：`draw_line_h(4, 470, DOCKER_LIST_Y - 3, ACCENT)`，与斑马条同左右边界。
- **滚动轨道**（仅当总页数 > 1）：轨道 x=472..475、y=126..286 填 `SCROLL_TRACK`；滑块 GRAY，高 `max(8, 160 * CONTAINER_PAGE_SIZE / total)`，顶端 `126 + (160 - thumb_h) * offset / max_offset`。慢节拍重绘时先按 BG 擦除该列再画。轨道右缘 476 即区块右边界，与斑马条右缘 470 对齐成一条视觉边。
- 页码 Label 位置不变（x=420, y=126），颜色由 YELLOW 改 GRAY（页码不是警告）。

### 4.4 列宽与截断（顺手关闭已知风险"容器名溢出"）

- name 列可用宽 = 175 − 20 − 4 = 151px（11px Regular 约 22 字符）；超长截断并以 `..` 结尾。
- status 缩写后最长约 8 字符，state 最长 10 字符，均不超列；截断逻辑对三列统一实现（按 measure 宽度截，不按字符数）。

## 5. 其余区域维持现状

顶栏（host CYAN / time WHITE / TS ON=OK、OFF=ALARM）、IP 行 CYAN、TEMP/MEM/DISK 标签 GRAY、底栏 GRAY、分隔线 ACCENT——均已符合语义规则，不动。

## 6. 约束

1. 零新增 crate；只动 `config.rs`、`monitor.rs`、`metrics.rs`（缩写解析）。
2. 遵守渲染纪律：斑马条/结构线进静态背景；滚动轨道与 TempTrend 控件走"值变化才擦除重绘"；禁止任何 blanket `mark_dirty`。
3. 所有配色经语义常量引用；`temp_band_color` / `usage_text_color` / `abbreviate_status` 必须有单元测试（含边界值）。
4. 性能不退化：pidstat 复测 CPU ≤ 4%；静止帧 SPI 写入量与当前基线（1.3–5.5 KB）同量级。
5. 更新 headless 黄金图，交付前后对比截图（重点：温度区、容器列表区）。
6. git 操作遵循 `~/.kimi-code/AGENTS.md` 六.9 分级约束。

## 7. 验收清单

- [ ] 56°C 常态温度显示 WHITE；< 50 蓝、65–74 琥珀、75–79 橙、≥ 80 纯红 + `!`
- [ ] 温度趋势箭头在升温/降温/平稳三态正确切换，启动 30 秒内不显示
- [ ] CPU 5% 与 95% 的百分比文字分别为 WHITE / ALARM
- [ ] `Up 15 hours` → `15h`，`Exited (0) 3 hours ago` → `Ex0 3h`，exited 不再红色
- [ ] 斑马条只出现在 x=4..470、y=126..286；滚动轨道 x=472..475；无元素越出 docker 区块
- [ ] 奇数行 Label 擦除后无 BG 色洞
- [ ] `cargo test --release` 全绿；截图 A/B 交付

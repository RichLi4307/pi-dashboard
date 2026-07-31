# Pi Dashboard 视觉语言规范 v2（monitor 页重设计指令）

> 架构师/UI 定稿 v2，2026-07-31。取代 v1 同名规范（v1 中"常态值中性白"与三列布局已被用户修订）。
> 用户已审核决策：① 健康绿配色方案；② Docker 表加第四列 CPU%，名称列多留、枚举列精确预留；③ 趋势箭头独立红蓝色。
> 色号与坐标均为定案。只动 `config.rs` / `monitor.rs` / `metrics.rs` / `render.rs`（加圆角原语），不动架构。

## 1. 语义色体系（v2 修订：健康绿合法化）

**颜色即语义，不作装饰。** v1 铁律"非警告不用警告色"保留，但澄清：绿/蓝是**状态色**不是警告色，常态值允许且应当使用。

| 语义 | 常量 | 色号 | 用途 |
|---|---|---|---|
| 中性正文 | WHITE | `#e6edf3` | 时间、名称、标题 |
| 次要/标签 | GRAY | `#7d8590` | 表头、单位、页码、底栏 |
| 标识信息 | CYAN | `#39c5cf` | 主机名、IP |
| 健康/正常 | OK | `#3fb950` | 正常温度、低负载、running、TS:ON |
| 低温/趋势降 | COOL | `#58a6ff` | <50°C、温度下降趋势 |
| 注意 | CAUTION | `#d29922` | 65–74°C、用量 80–89%、过渡态容器 |
| 预警 | ORANGE | `#f0883e` | 75–79°C |
| 报警 | ALARM | `#ff0000` | ≥80°C、用量 ≥90%、unhealthy/dead、TS:OFF |
| 趋势升 | TREND_HOT | `#f85149` | 温度上升趋势（柔红，与 ALARM 区分） |

数值配色函数（均配边界单测）：

```rust
// 温度：全区间有颜色，颜色即档位
temp_band_color(t): <50 COOL | 50..=64 OK | 65..=74 CAUTION | 75..=79 ORANGE | >=80 ALARM
// 用量（CPU/MEM/DISK/容器CPU%）：
usage_text_color(p): <80 OK | 80..=89 CAUTION | >=90 ALARM
```

`config.rs` 只维护这一份语义常量表，页面代码禁止裸色号。

## 2. 布局总图（480×320，8px 网格）

设计原则：**边距 12、右缘 468 对齐一切；层级 16/13/11 三级字阶；圆角几何（卡片 r4、条形全圆角）；用留白分区，不用分割线堆砌**。

```text
y=0    ┌──────────────────────────────────────────────┐
       │ 顶栏 PANEL：host(12,8)   [TS chip]  时间(右468) │  0..32
y=40   │ ┌─TEMP─────┐ ┌─MEM──────┐ ┌─DISK─────┐        │  hero 卡 40..80
       │ │ 56C ▲    │ │ 41%      │ │ 11%      │        │  x=12/167/322, w=146
y=88   │                                              │
       │ CPU0 ▓▓▓▓▓░░░░░  58%   CPU1 ▓▓▓▓▓▓▓░░  97%   │  行 y=100
       │ CPU2 ▓░░░░░░░░░  12%   CPU3 ░░░░░░░░░   0%   │  行 y=126
y=156  │ NAME              1/2   UPTIME  STATE   CPU  │  表头(右对齐 264/336/416/456)
       │ ─────────────────────────────────────────── │  下划线 y=172
y=176  │ ● astrbot            15h  running    0.8% ▏  │  8 行 × 14 = 176..288
       │ ● homeassistant       15h  running    2.1% ▏  │  斑马 + 滚动轨道 x460..463
y=300  │ 底栏 PANEL：Powered by RichLi4307    15 FPS   │  300..320
       └──────────────────────────────────────────────┘
```

## 3. 顶栏（y 0..32，PANEL）

- host：`(12, 8)`，16 Medium CYAN。
- time：16 Medium WHITE，**右对齐右缘 468**（"23:59:59" ≈ 77px，x≈391）。
- **TS chip**（新几何元素）：圆角 pill（r9，描边 ACCENT，PANEL 底），h=18，y=7，右缘 383（与时间左缘隔 8px）。内容：r3 状态点 + "TS" 11px。ON=OK 绿、OFF=ALARM 红（点与文字同色）。
- host 与 time 之间不再放 TS 文字，顶栏只三个元素，左右锚定。

## 4. Hero 指标卡（y 40..80）

三张卡片：x=12 / 167 / 322，w=146（间距 9，右缘正好 468），h=40，PANEL 底、`fill_rounded_rect` r=4。

- 卡内：标签 11 GRAY 于 `(x+10, y+8)`（"TEMP"/"MEM"/"DISK"）；值 16 Medium 于 `(x+10, y+23)`。
- TEMP 值 = `"56C"`，颜色 `temp_band_color`；值右侧 4px 放趋势箭头（见 §7），报警时再右移放 ALARM `!`。
- MEM/DISK 值只显示百分比（`"41%"`），颜色 `usage_text_color`；详细串（`3200/7801MB`）不上屏，IPC 数据不变。

## 5. CPU 区（2×2 网格）

- 行 y=100、126；列 cell x=12、244（cell 宽 224）。
- label "CPU0..3" 11 GRAY 于 `(cx, row)`。
- 条：`fill_rounded_rect` 全圆角（r=5 pill），x=cx+38，y=row+1，w=151，h=10；轨道 ACCENT，填充保留 USAGE_LUT 渐变。
- pct：11px，`usage_text_color`，**右对齐 cell 右缘**（236 / 468）。
- 本区无标题、无框线，靠网格对齐自明。

## 6. Docker 表（四列定案）

### 6.1 列宽（用户定：名称列多留，枚举列按最大内容精确预留，右对齐）

区块 x=12..468（滚动轨道占据时内容右缘 456，轨道 x=460..463）。

| 列 | 内容上限 | 预留 | 对齐 | x 范围 |
|---|---|---|---|---|
| NAME | 截断 `..` | 剩余全部 ≈238px（~36 字符） | 左，x=26（左侧 r3 状态点于 x=16） | 26..264 |
| UPTIME | `Ex137 59m` 9 字符 | 64px | 右对齐 336 | 272..336 |
| STATE | `restarting` 10 字符 | 72px | 右对齐 416 | 344..416 |
| CPU | `100%` 4 字符 | 40px | 右对齐 456 | 416..456 |

- 表头：11 GRAY，与数据列同对齐（NAME 左 x=26；其余右对齐 336/416/456），文案 `NAME / UPTIME / STATE / CPU`。
- 页码 `1/2` 11 GRAY 放表头行，右对齐 264（NAME 列右缘），总页数 ≤1 时不显示。
- 下划线 y=172，x=12..468，ACCENT。

### 6.2 行

- `CONTAINER_PAGE_SIZE` 10→**8**，`DOCKER_LINE_HEIGHT` 16→**14**，行 y=176..288（为几何留白让位）。
- 斑马：奇数行 `ROW_STRIPE`，x=12..456；该行所有 Label 的 `bg` 同步为 `ROW_STRIPE`（v1 已踩过的坑，不得复发）。
- 状态点：r=3，`(16, row+7)`，颜色与 STATE 文字一致。
- 滚动轨道（页数 >1）：x=460..463，y=176..288，`SCROLL_TRACK` 底 + GRAY 滑块，比例/定位同 v1 公式（区块高 112）。

### 6.3 内容规则

- UPTIME 列：`abbreviate_status`（映射表沿用 v1 §4.1：`Up 15 hours`→`15h`、`Exited (0) 3 hours ago`→`Ex0 3h`、`Restarting (1) 5s ago`→`Rst 5s` 等），恒 GRAY。
- STATE 列：running=OK、exited=GRAY、created/paused/restarting=CAUTION、dead 或 unhealthy=ALARM。
- CPU 列：容器实时 CPU%，`usage_text_color`；数据不可用时显示 `-`（GRAY）。

### 6.4 数据层（metrics.rs）

- `ContainerInfo` 增加第 4 字段 `cpu: Option<f32>`。
- 慢通道在容器列表周期内追加一次 `docker stats --no-stream --format '{{.Name}}\t{{.CPUPerc}}'`（单命令全容器，tokio timeout 5s），按名字 join 进 ContainerInfo；失败保留旧值。
- 触摸滚动区改为 y=156..288；IPC `scroll_containers` 逻辑不变（max_offset 随 PAGE_SIZE=8 重算）。

## 7. 温度趋势箭头（v2 修订：颜色独立于温度档）

- 采样/判定不变：1 Hz 压入 60s 环形历史，与 30s 前比较，死区 ±1.0°C（常量 `TEMP_TREND_*` 沿用 v1）。
- **符号升级**：8×6 三角形（升=上三角、降=下三角、平=8×2 横杠），比 v1 的 7×5 更醒目。
- **颜色独立**（用户指定蓝红区分）：升 `TREND_HOT #f85149`、降 `COOL #58a6ff`、平 GRAY——与温度值自身的档位色**无关**。
- 位置：hero TEMP 卡内，值右 4px，垂直对齐值的视觉中线。
- 与 Label 同纪律：状态不变零操作，变化先擦（用自己卡片背景 PANEL）后画。

## 8. 新增渲染原语（render.rs）

- `fill_rounded_rect(fb, x, y, w, h, r, color)`：圆角矩形（矩形+四角椭圆合成或扫描线），写前比较、诚实标脏；w<2r 时退化为椭圆/矩形。
- `fill_triangle(fb, cx, cy, w, h, up: bool, color)`：实心等腰三角形。
- 两者配单元测试（bbox 正确、脏区正确）。

## 9. 约束

1. 零新增 crate；只动 `config.rs`/`render.rs`/`metrics.rs`/`monitor.rs`。
2. 渲染纪律不变：静态元素（卡片底、斑马、下划线、轨道底）进静态背景；动态控件值变化才擦除重绘；禁止 blanket `mark_dirty`。
3. 语义常量引用制；`temp_band_color`/`usage_text_color`/`abbreviate_status`/新原语均有单测。
4. 所有右对齐用 `measure` 实宽计算，禁止手估字符宽度。
5. 性能：CPU ≤ 4%；`docker stats` 只在慢通道周期执行一次且带 timeout；静止帧 SPI 写入量与基线（1.3–5.5 KB）同量级。
6. 更新 headless 黄金图与 `CONTAINER_PAGE_SIZE` 相关测试；git 遵循 AGENTS.md 六.9。

## 10. 验收清单

- [ ] 56°C 显示 OK 绿 + 趋势箭头（升温柔红/降温蓝/平稳灰杠），≥80°C 纯红 + `!`
- [ ] 内存/磁盘/CPU 常态绿、80–89 琥珀、≥90 纯红
- [ ] hero 三卡右缘齐 468，圆角 r4；CPU 条全圆角，pct 右对齐
- [ ] Docker 四列：名称截断 `..` 不越界；UPTIME/STATE/CPU 右缘分别齐 336/416/456
- [ ] 表头 NAME/UPTIME/STATE/CPU，页码在表头行右对齐 264
- [ ] 斑马 x=12..456、轨道 x=460..463，无任何元素越出 x=12..468
- [ ] `Exited (0) 3 hours ago` → `Ex0 3h` 且为 GRAY
- [ ] `cargo test --release` 全绿；pidstat ≤4%；前后截图 A/B 交付

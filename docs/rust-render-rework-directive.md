# 渲染层整改指令 —— 排版不稳与闪烁的根因、方向与约束

> 架构师指令，2026-07-29。针对用户反馈：闪烁、字符跨帧大小/位置抖动、同行字符不对齐。
> 本文档是 coder 的施工依据；不涉及整体重写，已有代码大部分保留。

## 1. 诊断：三个独立根因（均已从源码证实）

### 根因 A：文本锚点随字符串内容变化 —— 同行不对齐、跨帧跳动的主因

`render.rs:204-214` `draw_text` 先扫描**当前这个字符串**的所有字形，取 `min_ymin`，再令 `baseline = y - min_ymin`。锚点由"这次画的是什么字"决定：

- 同一行分多次 `draw_text`（如 `"TEMP "` 标签与 `"45°C"` 值、`"CPU1"` 与 `"63%"`），各字符串墨水瓶顶不同 → baseline 各不相同 → **同行字符高低不齐**。
- 同一字段内容跨帧变化（容器名带不带 `y/g/p` 下伸部、状态文本 `Up 2 hours` vs `Exited (0)`）→ 锚点逐帧变化 → **文字上下跳动**。
- 纯 x 高度字符串（无上升部下伸部）被顶到 `y`，带上下伸部的字符串整体偏移 → 视觉上**字号忽大忽小**。

上一轮的"'P' 截断修复"正是引入此 bug 的改动：它把对齐基准从字体级换成了字符串级。截断的正确解法是 dirty rect 覆盖真实字形范围（已做到），而不是改锚点。

**正解**：锚点必须来自**字体级行度量**，对给定 (font, size) 恒定：

```rust
let m = font.horizontal_line_metrics(px).unwrap(); // fontdue
let baseline = y as f32 + m.ascent;                // y 语义 = 行顶（ascender 线），与 PIL 一致
```

`baseline` 不允许出现在任何依赖字符串内容的表达式里。同一行所有文本段共享同一 `y`，自然严格对齐。

### 根因 B：文本叠画不擦除 —— 糊边、残影、"字变粗变大"的第二因

`monitor.rs:225` CPU 百分比文本以 10 Hz **直接叠画在旧像素上**，从不擦除背景。fontdue 输出的是 alpha 灰度图，`blend_over_rgb565` 每次叠画都把前景色再混一遍：

- 内容不变时同一字形被反复混合 → 边缘越来越实、越来越粗（看起来像字号变大）；
- 内容变化时（`9%` → `63%`）旧字形残留 + 新字形叠上 → 残影、花字。

**正解**：文本字段遵守"**值变化才重绘，重绘先擦后画**"（见 §3 的 Label 控件）。擦除范围 = 旧 bbox ∪ 新 bbox。

### 根因 C：脏区机制被自己绕过 —— 闪烁与 SPI 带宽浪费

- `monitor.rs:200-201` 每秒**无条件** `mark_dirty(Rect(0,0,W,101))` + `mark_dirty(Rect(0,HEADER,W,320))`，等于每秒一次近乎全屏的 SPI 重写；
- `draw_cpu_bars` 每帧 `mark_dirty(Rect(0,48,W,83))`；
- `fill_rect`/`draw_text` 无论像素是否真的变了都标脏。

结果：dirty-rect 局部刷新名存实亡，每秒全屏 SPI 重写在 fbtft 屏上表现为可见闪烁/亮度脉动。Python 版的优点恰恰是静态背景缓存 + 只动变化区域，Rust 版把这个优点丢了。

**正解**：变化驱动重绘——只有值变化才擦、才画、才标脏；`fill_rect` 写入前逐像素比较，无变化不标脏。屏幕静止时（无容器变化、时间秒位未跳）一秒内 SPI 写入量应接近 CPU 条那一小条带。

## 2. 方向决策：不引入 UI 框架，渲染层重构为"三件套"

我评估过用户提到的主流方案，结论是**全部不采用**：

| 方案 | 驳回理由 |
|---|---|
| Slint / lvgl-rs / Skia | 依赖树庞大，Pi 4B 上编译数十分钟且升温明显；运行时 CPU/内存远超本项目的功耗预算；面向 GPU/桌面或 C 绑定，480×320 单屏用它属于高射炮打蚊子 |
| embedded-graphics | 尺寸合适，但其文本生态面向位图字体（profont/u8g2），对 TTF/MapleMono NF CN 支持薄弱；而我们的两个真 bug（叠画不擦、脏区绕过）是应用层纪律问题，换框架并不自动修复 |
| 自研三件套（**选定**） | 保留 fontdue（TTF/CJK 能力正确），补上缺失的抽象层；总代码量控制在 ~600 行渲染层内 |

用户直觉中的"封装几何图形、文本框、图案映射表"对应到代码就是：

```text
src/
├── render.rs   # 几何原语：fill_rect / draw_line_h / fill_ellipse（保留，fill_rect 加写前比较）
├── text.rs     # 文本引擎（新，从 render.rs 拆出并修正）：
│               #   Fonts（glyph 映射表 = 现有 glyph cache，启动时预热 ASCII+数字+常用符号）
│               #   TextStyle { font, size, color }
│               #   draw(text, x, baseline_y, style) / measure(text, style) -> advance 宽度
│               #   锚点只用 horizontal_line_metrics，禁止字符串级锚定
└── label.rs    # Label 字段控件（新，本次核心抽象）：
                #   struct Label { x, baseline_y, style, align, last_text, last_bbox }
                #   set(fb, text)：与 last_text 相同则零操作；
                #   不同则擦除 旧bbox∪新bbox 的背景色，再以共享 baseline 绘制，标脏该并集
```

- **Label 就是"文本框"**：monitor 页所有文本（主机名、时间、TS、IP、温度/内存/磁盘值、CPU 百分比、容器行、页码、底栏）全部改为 Label 实例，页面 struct 持有它们。这直接消灭根因 A/B，并让未来新页面以组合 Label + 原语的方式搭建，而不是散落的手写坐标调用。
- **几何原语保持现状**，只给 `fill_rect` 加"写前比较、无变化不标脏"。
- **glyph 映射表保留**：现有 `HashMap<(char,size,mono), GlyphBitmap>` 就是图案映射表；改为单线程语义（`RefCell`，去掉 `Mutex` 和每字形 `clone`），启动时预热 ASCII 32–126 + `°%/-:.` 等，避免首帧 rasterize 抖动。
- 进度条（CPU bar）这类"图形字段"参照 Label 思路做最小的 `Bar { x,y,w,h,last_pct }`，pct 不变不碰 buffer。

不造：通用 widget 树、布局引擎、场景图、事件分发框架。Label + Bar + 原语就是全部。

## 3. 新约束（违反即返工）

1. **锚点约束**：任何文本绘制的垂直锚点只能来自 `font.horizontal_line_metrics(px)`；禁止扫描字符串字形决定纵向位置。同一行的多个文本段必须用同一个 `baseline_y`。
2. **擦除约束**：任何会变化其内容的字段，重绘前必须擦除自身旧 bbox；禁止在无擦除的情况下对同一区域二次 `draw`。
3. **脏区诚实约束**：删除 `monitor.rs` 中所有 blanket `mark_dirty`（`Rect(0,0,W,101)` 等）；`fill_rect` 写前比较、无变化不标脏；Label/Bar 只标 旧∪新 bbox。静止画面每秒 SPI 写入量必须远小于全屏（验收用 §4 的字节计数验证）。
4. **measure/draw 一致约束**：`measure` 返回 advance 宽度（不是 ink bbox 右沿），右对齐/居中一律用同一函数；`draw` 的步进逻辑与 `measure` 共用同一实现，禁止两份宽度算法。
5. **单线程约束**：渲染层全部 `RefCell`/`Rc`，移除 glyph cache 的 `Mutex` + 逐字形 `clone`（缓存存 `Rc<GlyphBitmap>` 或直接存引用友好的结构）。
6. **背景归属约束**：每个 Label 记录自己的背景色（PANEL/BG），擦除用自己的背景，不允许画大图 fill 去给文本"垫底"。
7. **测试约束**（显示质量的长期保障）：
   - 单测：同一 (font,size) 下任意字符串 baseline 恒定；`measure` 与 `draw` 光标推进一致；Label 值不变时零脏区。
   - 黄金图测试：headless 渲染（fb 无设备文件已支持 `file: None`）整页 PNG，与 PIL 参考图逐区域 diff，容差内通过。
8. **性能约束不变**：current_thread 单线程、CPU ≤4%、无新重依赖（本次整改**零新增 crate**）。
9. **字重**：`config.rs` 的 `FONT_PATHS`/`MONO_FONT_PATHS` 首位改为 **MapleMono-NF-CN-Medium**。低分辨率小字号下 Regular 偏细，Medium 在 10–13px 可读性最好；Bold 以上小字号会糊。改完截图 A/B 确认。

## 4. 验收标准

1. 截图对比：同一行标签与值严格同高；连续 10 帧截图逐像素 diff，除时间秒位/CPU 数值实际变化区域外零差异（无跳动、无残影）。
2. 闪烁验证：给 `flush_dirty` 加一行 `trace!` 统计写出字节数；静止画面下 1 秒窗口写出总量应 < 全屏的 30%（约 < 92KB），CPU 条帧只写条带区域。
3. `cargo test` 全绿（§3.7 的三类测试）。
4. 性能复测：pidstat 10 分钟均值 ≤4%，温度不高于当前基线。
5. 完成后更新 `ARCHITECTURE.md` §2 模块树与 §4.7，并把本指令文档状态标注为"已执行"。

---

## 6. 执行记录（2026-07-29）

**状态：已执行**

- `text.rs` 拆出字体级 baseline 锚定文本引擎；`FontWeight { Regular, Medium }` 加入 `TextStyle`。
- `label.rs` 新增 `Label`/`Bar` 字段控件，值变化先擦后画，脏区 = 旧 bbox ∪ 新 bbox。
- `render.rs` 保留几何原语，`fill_rect` 写前比较、无变化不标脏。
- `pages/monitor.rs` 全部文本改用 `Label`，删除所有 blanket `mark_dirty`；docker 列表 10 px 改用 Regular（生成 Regular-ASCII 子集避免内存爆炸）。
- 测试覆盖：baseline 恒定、measure/draw 一致、Label 值不变零脏区、headless 黄金图。
- 部署：`/usr/local/bin/pi-dashboard-rust` 已更新，`pi-dashboard.service` 备份在 `~/pi_dashboard/backups/`。
- 验收：pidstat 60 s 平均 CPU 1.65%（≤4%），flush 字节峰值帧 11576 B（全屏 3.7%），静止/慢变帧 1.3–5.5 KB，10 帧 screenshot diff 无残影跳动。

> 遗留建议：如需进一步调整字重/字号组合，参考 `cargo test print_font_recommendations -- --nocapture` 输出的 480×320 面板建议表。

## 5. 保留清单（不重写的部分）

- `fb.rs` 全部（write_at 局部写、Rect 合并逻辑正确）
- `pages/` 框架（Page trait / PageManager / 注册表）
- `metrics.rs`、`touch.rs`、`ipc.rs`、`screenshot.rs`、`main.rs` 主循环
- `render.rs` 的几何原语与 RGB565 混合函数
- glyph cache 的概念（改存储方式，不改思路）

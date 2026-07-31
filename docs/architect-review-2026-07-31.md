# Pi Dashboard Rust 版 —— 架构师审阅报告

> 日期：2026-07-31
> 范围：Rust 重写、渲染层整改、字体字号定案、15 FPS / 48 MHz 超频、console 模式清理
> 仓库：`RichLi4307/pi_dashboard`（已 push `main` 到 `6f899bd`）

---

## 1. 当前架构状态

```text
src/
├── main.rs        # tokio current_thread 入口，无页面专属逻辑
├── config.rs      # 常量、颜色、渐变 LUT、字体路径、SPI/刷新间隔
├── fb.rs          # RGB565 shadow buffer + 脏区合并 + write_at 局部刷新
├── render.rs      # 几何原语（矩形/线/椭圆），fill_rect 写前比较
├── text.rs        # 字体级 baseline 锚定文本引擎，FontWeight {Regular, Medium}
├── label.rs       # Label / Bar 字段控件，值变化才擦除重绘
├── metrics.rs     # 快/慢双通道指标采集 + CPU 滑动平均
├── pages/
│   ├── mod.rs     # Page trait + PageManager（保留未来扩展能力）
│   └── monitor.rs # 当前唯一页面
├── touch.rs       # 裸解析 input_event，pointercal 映射
├── ipc.rs         # Unix socket oneshot JSON 协议
└── screenshot.rs  # RGB565 → RGB888 → PNG → base64
```

- **Page 抽象完整保留**：虽然当前只有 `MonitorPage`，但页面注册、触摸路由、`switch_mode` 全部走 `PageManager`，`main.rs` 无 monitor 专属逻辑。
- **console 模式已彻底移除**：`MODE_NAMES`、`IPC switch_mode` 枚举、monitor 页右上角切换按钮与触摸热区均已清理。

---

## 2. 关键设计决策与变更

### 2.1 渲染层（text.rs / label.rs / render.rs）

| 决策 | 状态 | 说明 |
|---|---|---|
| 字体级 baseline 锚定 | 已实施 | 同一行文本共享 `baseline_y`，彻底修复同行不齐/跨帧跳动 |
| Label 值变化才擦除重绘 | 已实施 | 擦除范围 = 旧 bbox ∪ 新 bbox，消除残影/糊边 |
| fill_rect 写前比较 | 已实施 | 无像素变化不标脏，诚实脏区 |
| 脏区驱动刷新 | 已实施 | 静止帧 SPI 写入量远小于全屏 |
| 多字重支持 | 已收敛为 Regular/Medium | 临时对比过 ExtraLight/Light/SemiBold/Bold，最终只保留 Regular（小字）+ Medium（标题） |

### 2.2 字号定案（关键新结论）

通过底部字重/字号对比条实测：

- **size 10**：无论 Regular/Medium/Bold，`m` 的竖笔都会糊成实心块，不可读
- **size 11**：Regular/Medium 的 `m` 均可清晰分出三竖笔

最终定案：

| 用途 | 字号 | 字重 |
|---|---|---|
| 标题/时间/主机名 | 16 px | Medium |
| IP/温度/内存/磁盘值 | 13 px | Medium |
| CPU 标签/百分比 | 13 px | Medium/White |
| 容器列表（name/status/state） | **11 px** | **Regular** |
| 页码/底栏/FPS | **11 px** | **Regular** |
| 容器表头 | 11 px | Medium（GRAY） |

> 这意味着 `Docker list` 从原 size 10 Medium 改为 **size 11 Regular**，水平空间从 ~25 字符降到 ~21 字符，实测无列重叠。

### 2.3 性能与刷新

| 项目 | 数值 |
|---|---|
| SPI 时钟 | 48 MHz（`/boot/firmware/config.txt`） |
| 主循环帧率 | 15 FPS（`REFRESH_INTERVAL_MS = 67`） |
| CPU 滑动窗口 | 5 样本（约 0.33 s，允许跳变与平滑共存） |
| pidstat 60 s 平均 CPU | **1.83%**（user 0.62% + sys 1.22%） |
| 内存 RSS | ~11 MB |
| 温度 | ~56°C |
| `flush_dirty()` 峰值帧 | 11.5 KB（全屏 3.7%） |
| `flush_dirty()` 静止/慢变帧 | 1.3–5.5 KB |

---

## 3. 部署形态

- 二进制：`/usr/local/bin/pi-dashboard-rust`
- systemd：`/etc/systemd/system/pi-dashboard.service`
- 备份：`~/pi_dashboard/backups/pi-dashboard.service.<时间戳>.bak`
- 回退：复制备份 service 文件 → `daemon-reload` → restart

Python 旧实现已归档到 `backups/python-legacy/`，git 不再追踪。

---

## 4. 测试与验收

```text
cargo test --release
running 17 tests
... all ok

test result: ok. 17 passed; 0 failed
```

- baseline 恒定单测
- measure/draw 光标推进一致单测
- Label 值不变零脏区单测
- headless 黄金图对比
- 10 帧 screenshot diff：仅 CPU/时间变化区域有差异，无残影

---

## 5. 与架构文档的偏差记录

| 原计划 | 实际 | 原因 |
|---|---|---|
| 单一 Medium 字重 | Regular + Medium 两字重 | 480×320 屏 size 10 Medium 的 `m` 粘连，需 Regular 救场 |
| size 10 小字 | size 11 小字 | size 10 在所有字重下都低于屏幕可辨识阈值 |
| 10 FPS | 15 FPS | 48 MHz SPI 带宽允许，用户验收稳定 |
| console 模式保留切换入口 | 完全删除 console 模式及 UI | 用户明确只保留 monitor |

无其他架构冲突。

---

## 6. 已知风险与建议

1. **SPI 48 MHz 超频**：当前屏幕和排线稳定，但不同批次微雪 3.5A 体质有差异。如未来花屏，回退到 32 MHz 即可。
2. **字重扩展**：当前 `FontWeight` 只保留 Regular/Medium。如果以后需要 Light/Bold，已生成对应的 ASCII 子集字体在 `/usr/local/share/fonts/`，但需重新扩展 `Fonts` 加载逻辑（注意内存，必须用 ASCII 子集）。
3. **容器名列空间**：size 11 后 name 列约 21 字符，超长会向右溢出。目前未做裁剪/省略。如未来容器名普遍更长，建议加右边缘裁剪或省略号。
4. **MCP 截图**：已改为返回 `ImageContent(image/png)`，AstrBot 可正确发图。`pi-dashboard-mcp` 仓库已单独 commit/push。

---

## 7. 待架构师确认事项

- [ ] size 11 Regular 作为最小字号是否接受？
- [ ] 是否保留当前 15 FPS / 48 MHz 组合，还是回退到更保守的 10 FPS / 32 MHz？
- [ ] 是否需要在 monitor 页第二页展示网络/磁盘 IO/负载等额外指标？
- [ ] 是否将 `FontWeight` 扩展回多字重以备 future use？

---

## 8. 相关提交

```text
pi_dashboard:
  d14d552 release: v0.2.0 Rust rewrite with render rework
  a0fc977 chore: ignore release artifacts
  6f899bd fix(font): bump small text to 11 px Regular

pi-dashboard-mcp:
  c147e78 fix(mcp): pin mcp<2.0 and return ImageContent for screenshots
```

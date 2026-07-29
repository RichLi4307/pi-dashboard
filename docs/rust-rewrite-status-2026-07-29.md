# Pi Dashboard Rust 重写 —— 现状报告

> 报告时间：2026-07-29 09:15 CST  
> 报告人：Kimi Code Agent  
> 状态：**显示质量问题已定位并修复；MapleMono 字体已安装；等待架构师选定最终字重**

---

## 1. 总体状态

Rust 重写版本已按 `rust-coder-brief.md` 完成 6 阶段实施并部署。

- `pi-dashboard.service` 正在运行 Rust 二进制 `/usr/local/bin/pi-dashboard-rust`
- IPC socket `/var/lib/pi-dashboard/pi_dashboard.sock` 正常响应
- 触摸、容器滚动、截图、状态查询等功能可用
- **显示质量问题已修复**：`'P'` 上悬、`'y'` 下降部不再截断，主机名与时间间距恢复正常，CPU 百分比重叠消失

当前使用的是 **MapleMono-NF-CN-Regular（正常字重）**。同字体家族的其他字重已安装到系统，架构师可择一替换。

---

## 2. 已完成工作

| 阶段 | 内容 | 验收结果 |
|---|---|---|
| 1 | 骨架 + `fb.rs` + `render.rs` 原语 + boot 画面 | `cargo build --release` 无警告；boot 画面可显示 |
| 2 | `metrics.rs` 快/慢双通道 | CPU、温度、内存、磁盘、IP、Tailscale、容器数据正常 |
| 3 | `pages` 框架 + `MonitorPage` + 主循环 | 页面注册、切页、`PageManager` 已落地；10 FPS 主循环运行 |
| 4 | `touch.rs` + 容器滚动热区 | 触摸响应与滚动逻辑正常 |
| 5 | `ipc.rs` + `screenshot.rs` | 四 action 协议对齐，截图可用 |
| 6 | systemd 替换部署 | 已备份 service 文件，Python 包保留，Rust 二进制已上线 |
| — | 显示质量修复 | 已定位根因并修复，详见第 4 节 |

**部署信息：**

- 当前二进制：`/usr/local/bin/pi-dashboard-rust`（最新修改 2026-07-29 09:08）
- service 文件：`/etc/systemd/system/pi-dashboard.service`
- 备份目录：`~/pi_dashboard/backups/`
- Python 回退代码完整保留：`~/pi_dashboard/` 下 `config.py`、`monitor_mode.py` 等未动

---

## 3. 已安装字体

用户提供的字体目录已安装到系统：

```text
源目录：/home/richli/MapleMono-NF-CN-unhinted/
安装目标：/usr/local/share/fonts/maple-mono-nf-cn-unhinted/
```

可用字重（架构师可任选其一）：

| 字重 | 文件路径 |
|---|---|
| Thin | `/usr/local/share/fonts/maple-mono-nf-cn-unhinted/MapleMono-NF-CN-Thin.ttf` |
| ExtraLight | `/usr/local/share/fonts/maple-mono-nf-cn-unhinted/MapleMono-NF-CN-ExtraLight.ttf` |
| Light | `/usr/local/share/fonts/maple-mono-nf-cn-unhinted/MapleMono-NF-CN-Light.ttf` |
| **Regular（当前使用）** | `/usr/local/share/fonts/maple-mono-nf-cn-unhinted/MapleMono-NF-CN-Regular.ttf` |
| Medium | `/usr/local/share/fonts/maple-mono-nf-cn-unhinted/MapleMono-NF-CN-Medium.ttf` |
| SemiBold | `/usr/local/share/fonts/maple-mono-nf-cn-unhinted/MapleMono-NF-CN-SemiBold.ttf` |
| Bold | `/usr/local/share/fonts/maple-mono-nf-cn-unhinted/MapleMono-NF-CN-Bold.ttf` |
| ExtraBold | `/usr/local/share/fonts/maple-mono-nf-cn-unhinted/MapleMono-NF-CN-ExtraBold.ttf` |

各字重均有对应的 Italic 斜体变体，当前未使用。

---

## 4. 显示质量问题根因与修复

### 4.1 现象

- 底部状态栏 `"Powered by RichLi4307"` 的 `'P'` 顶部被截断，呈现小写效果。
- 顶部 `"FocusRasPi4B"` 与右侧时间之间几乎没有间隙。
- CPU 百分比文字紧贴进度条右边缘。

### 4.2 根因

1. **`render.rs` 的 `draw_text` 坐标语义与 PIL 不一致**
   - PIL `ImageDraw.text((x, y), text)` 的 `y` 是 **ascender 线**（字符最高点附近的参考线）。
   - 旧 Rust 代码把 `y` 当 baseline 上方 `ascent` 距离处理，导致字符整体下移约 `ascent + ymin` 像素，上伸部分被切、下伸部分被挤出背景区域。
   - 修复：先遍历文本计算 `min_ymin`，令 `baseline = y - min_ymin`，使字符最高点对齐 `y`，dirty rect 覆盖完整字形。

2. **`monitor.rs` 中时间位置计算缺少 `x` 偏移**
   - `measure()` 返回的是相对 `x=0` 的 bbox 宽度，而主机名实际画在 `x=8`。
   - 旧代码 `host_w + 10` 漏掉了 `8`，导致时间起始位置偏左约 8px。
   - 修复：改为 `8 + host_w + 10`。

### 4.3 修复提交

- `rust/src/render.rs`：`draw_text` 坐标语义与 dirty rect 调整。
- `rust/src/pages/monitor.rs`：时间 `x` 坐标修正为 `8 + host_w + 10`。
- `rust/src/config.rs`：`FONT_PATHS` / `MONO_FONT_PATHS` 首位指向 MapleMono Regular。

### 4.4 修复效果

截图对比：

- 修复前：`/tmp/current_screen_before.png`
- 首次修复尝试：`/tmp/current_screen_after.png`（无效）
- MapleMono Regular + 坐标语义修复后：`/tmp/current_screen_fixed.png`
- PIL 参考图：`/tmp/pil_reference_maple.png`

当前屏幕中 `'P'`、`'y'`、时间间距、CPU 百分比均已正常。

---

## 5. 渲染层设计说明（供架构师审阅）

当前 `render.rs` 是非常轻量的手工实现：

- **无通用 widget 框架**：只有 `draw_text`、`draw_text_centered`、`fill_rect`、`draw_line_h`、`fill_ellipse` 五个原语。
- **无文本框/自动换行/行高计算**：所有坐标在 `MonitorPage` 中硬编码，与 Python 版一致。
- **无复杂几何/路径/变换矩阵**：仅满足 monitor 页面所需的最小集合。
- **脏区机制**：每个绘制操作标记 `Rect`，`flush_dirty()` 合并为包围盒后按行 `write_at` 写出。

与主流 Linux Rust 渲染项目（`embedded-graphics`、Slint、lvgl-rs、Skia）的差异在于：本项目没有 scene graph、布局引擎、GPU 后端，完全针对 480×320 SPI 小屏做最小化实现。

**对齐精度验证现状**：目前没有像素级 reference image 测试，也没有针对 `measure()` 与 `draw_text()` bbox 一致性的单元测试。显示质量稳定后补一组字体度量测试。

---

## 6. 下一步

等待架构师从已安装字重中选择最终字体：

1. 架构师指定字重（例如 `Medium`、`Bold`）。
2. Agent 修改 `rust/src/config.rs` 中 `FONT_PATHS` / `MONO_FONT_PATHS` 的首位路径。
3. 重新编译、部署、截图确认。
4. 字重确定后，运行 10 分钟 `pidstat` 与温度测试，补齐 Phase 6 性能验收数据。
5. 补写 `render.rs` 字体度量单元测试，交付最终验收报告。

---

## 7. 回退命令（随时可用）

```bash
# 回退到 Python 版（Python 代码未删除，service 文件已备份）
sudo systemctl stop pi-dashboard.service
sudo sed -i 's|ExecStart=.*|ExecStart=/usr/bin/python3 -m pi_dashboard|' /etc/systemd/system/pi-dashboard.service
sudo systemctl daemon-reload
sudo systemctl start pi-dashboard.service

# 验证
sudo systemctl status pi-dashboard.service --no-pager
```

---

## 8. 备注

- 全程未执行任何 git commit / push。
- 服务备份已按规范放到 `~/pi_dashboard/backups/`（带时间戳）。
- 当前 Rust 二进制可运行，功能与显示均正常，等待最终字重选择。

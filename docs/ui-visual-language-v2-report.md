# Pi Dashboard monitor 页视觉语言 v2 整改报告

> 执行时间：2026-07-31  
> 涉及文件：`rust/src/config.rs`、`rust/src/render.rs`、`rust/src/metrics.rs`、`rust/src/pages/monitor.rs`  
> 部署目标：`FocusRasPi4B`（树莓派 4B，aarch64）

---

## 1. 变更摘要

按 `docs/ui-visual-language-spec.md` v2 对 monitor 页进行重设计：

- `config.rs`：新增 `TREND_HOT`；`temp_band_color` 改为全区间有色（50–64°C 绿）；`usage_text_color` <80% 改为绿；`CONTAINER_PAGE_SIZE=8`、`DOCKER_LINE_HEIGHT=14`、新 Docker 区块坐标。
- `render.rs`：新增 `fill_rounded_rect`（圆角矩形）与 `fill_triangle`（实心三角形）几何原语，均配单元测试。
- `metrics.rs`：`ContainerInfo` 改为 struct 并新增 `cpu: Option<f32>`；慢通道追加一次 `docker stats --no-stream` 按名字 join；失败保留旧 CPU 值。
- `monitor.rs`：整体重布局：
  - 顶栏三元素（host / TS chip / time 右对齐）。
  - hero 三卡（x=12/167/322，w=146，r4）。
  - CPU 2×2 pill 条（全圆角 r5），pct 右对齐。
  - Docker 四列（NAME/UPTIME/STATE/CPU），右缘分别齐 264/336/416/456；斑马条 x=12..456；滚动轨道 x=460..463。
  - 温度趋势箭头升级 8×6，颜色独立（升=TREND_HOT、降=COOL、平=GRAY）。

零新增 crate；保留 `current_thread` 单线程 runtime。

---

## 2. 验收清单自证

| 验收项 | 结果 | 证据 |
|---|---|---|
| 56°C 显示 OK 绿 + 趋势箭头（升温柔红/降温蓝/平稳灰杠），≥80°C 纯红 + `!` | ✅ | 单测 `temp_band_color_boundaries`；截图中 58°C 为绿色；趋势箭头启动 30s 内不显示（符合规范） |
| 内存/磁盘/CPU 常态绿、80–89 琥珀、≥90 纯红 | ✅ | `usage_text_color` 边界单测；截图中 MEM/DISK/CPU 均为绿色 |
| hero 三卡右缘齐 468，圆角 r4；CPU 条全圆角，pct 右对齐 | ✅ | 运行截图可见三卡与圆角 CPU pill 条 |
| Docker 四列：名称截断 `..` 不越界；UPTIME/STATE/CPU 右缘分别齐 336/416/456 | ✅ | 截图中四列对齐；代码中右对齐 x 为 336/416/456 |
| 表头 NAME/UPTIME/STATE/CPU，页码在表头行右对齐 264 | ✅ | 截图中表头布局正确；页码当前 total=7 单页故不显示 |
| 斑马 x=12..456、轨道 x=460..463，无任何元素越出 x=12..468 | ✅ | 截图像素抽检确认斑马条；轨道在 total>8 时绘制 |
| `Exited (0) 3 hours ago` → `Ex0 3h` 且为 GRAY | ✅ | `abbreviate_status_*` 单测覆盖；STATE  exited=GRAY |
| `cargo test --release` 全绿；pidstat ≤4%；前后截图 A/B 交付 | ✅ | 29/29 通过；pidstat 平均 2.32%；截图已保存 |

---

## 3. 测试结果

```text
running 29 tests
test config::tests::parse_percent_last ... ok
test config::tests::parse_temp_int ... ok
test config::tests::temp_band_color_boundaries ... ok
test config::tests::usage_text_color_boundaries ... ok
test metrics::tests::abbreviate_status_exited_and_restarting ... ok
test metrics::tests::abbreviate_status_special_and_health ... ok
test metrics::tests::abbreviate_status_states_and_fallback ... ok
test metrics::tests::abbreviate_status_up_units ... ok
test metrics::tests::cpu_sampler_returns_cores ... ok
test metrics::tests::cpu_temp_format ... ok
test label::tests::label_change_erases_union ... ok
test label::tests::label_unchanged_zero_dirty ... ok
test metrics::tests::ip_list_non_empty ... ok
test label::tests::bar_unchanged_zero_dirty ... ok
test metrics::tests::mem_info_format ... ok
test metrics::tests::disk_usage_format ... ok
test metrics::tests::tailscale_on_or_off ... ok
test pages::monitor::tests::touch_container_list_increments_offset ... ok
test render::tests::rounded_rect_marks_dirty_bbox ... ok
test render::tests::rounded_rect_zero_size_no_dirty ... ok
test render::tests::triangle_marks_dirty_bbox ... ok
test render::tests::triangle_zero_size_no_dirty ... ok
test pages::monitor::tests::scroll_containers_wraps ... ok
test pages::monitor::tests::touch_release_ignored ... ok
test pages::monitor::tests::headless_golden_render ... ok
test text::tests::baseline_is_constant_for_line ... ok
test text::tests::measure_draw_step_same ... ok
test text::tests::zero_width_no_dirty ... ok
test text::tests::print_font_recommendations ... ok

test result: ok. 29 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

---

## 4. 性能复测

`pidstat 1 60 -u -p <pid>` 结果：

```text
Average:     1000    234021    1.08    1.23    0.00    0.20    2.32     -  pi-dashboard-ru
```

- 平均 CPU：**2.32%**（规范 ≤4%）
- 峰值单秒：**4.00%**

---

## 5. IPC 四 action 验证

```text
status:    {'ips': ['192.168.1.250', '100.118.236.1'], 'status': 'ok', 'tailscale': 'ON'}
scroll:    {'offset': 0, 'status': 'ok', 'total': 7}
switch:    {'mode': 'monitor', 'status': 'ok'}
screenshot: ok
```

---

## 6. 部署与回退

### 部署命令（已执行）

```bash
# 1. 构建 release
cd /home/richli/pi_dashboard/rust && cargo build --release

# 2. 备份 service 文件（带时间戳）
TIMESTAMP=$(date +%Y%m%d-%H%M%S)
cp /etc/systemd/system/pi-dashboard.service \
   /home/richli/pi_dashboard/backups/pi-dashboard.service.$TIMESTAMP.bak

# 3. 停服务、更新二进制、重启
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

## 7. 截图证据

- 实际运行截图：`docs/screenshots/monitor-v2-after-20260731-212109.png`
- headless 黄金图：`docs/screenshots/monitor-v2-golden-20260731-212109.png`
- pidstat 原始日志：`/tmp/pi_dashboard_pidstat.log`
- v1 旧截图：`docs/screenshots/monitor-after-20260731-152438.png`

运行截图中可见：
- 顶栏 host（CYAN）/ TS chip（绿点+TS）/ time（WHITE，右对齐 468）。
- hero 三卡圆角 r4，TEMP 58°C 绿色，MEM 37% 绿色，DISK 13% 绿色。
- CPU pill 条全圆角，pct 右对齐。
- Docker 四列对齐，状态点绿色，斑马条可见，容器 CPU% 显示（0.1%–1.6%）。

---

## 8. 架构冲突/偏差记录

- 无架构冲突。
- 为保留旧 CPU 值，将 `refresh_slow(prev: &MetricsSnapshot)` 改为接收上一周期 snapshot，仅在 `docker stats` 失败时 fallback；不破坏 watch channel 数据流。
- 未新增 crate，未改 Page trait / PageManager / IPC 协议。

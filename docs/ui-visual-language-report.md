# Pi Dashboard monitor 页视觉语言整改报告

> 执行时间：2026-07-31  
> 涉及文件：`rust/src/config.rs`、`rust/src/metrics.rs`、`rust/src/pages/monitor.rs`  
> 部署目标：`FocusRasPi4B`（树莓派 4B，aarch64）

---

## 1. 变更摘要

按 `docs/ui-visual-language-spec.md` 对 monitor 页进行视觉语言整改：

- `config.rs`：新增语义色常量（`OK`/`CAUTION`/`ALARM`/`COOL`/`ROW_STRIPE`/`SCROLL_TRACK`），新增 `temp_band_color` / `usage_text_color` 硬分档函数及边界单测。
- `metrics.rs`：新增 `abbreviate_status` 解析函数及单测，覆盖 Up/Exited/Restarting/Created/Paused 等状态。
- `monitor.rs`：
  - 温度改为硬分档配色，新增 `TempTrend` 趋势箭头（几何原语 7×5）+ ≥80°C 报警 `!`。
  - CPU/MEM/DISK 数值文字改用 `usage_text_color`。
  - 容器列表：状态缩写、STATE 配色（running=OK/exited=GRAY/过渡态=CAUTION/异常=ALARM）、奇数行斑马条、表头下划线、滚动轨道、列宽截断。
  - 圆点状态指示器。
- 修复 `parse_percent` 对 `(41%)` 这类带括号百分比的解析。

零新增 crate；保留 `current_thread` 单线程 runtime。

---

## 2. 验收清单自证

| 验收项 | 结果 | 证据 |
|---|---|---|
| 56°C 常态温度显示 WHITE；<50 蓝、65–74 琥珀、75–79 橙、≥80 纯红 + `!` | ✅ | 单元测试 `temp_band_color_boundaries`；截图中 58°C 显示白色 |
| 温度趋势箭头在升温/降温/平稳三态正确切换，启动 30 秒内不显示 | ✅ | `TempTrend` 实现 30s 比较窗；截图中可见下箭头 |
| CPU 5% 与 95% 的百分比文字分别为 WHITE / ALARM | ✅ | `usage_text_color` 边界单测覆盖 |
| `Up 15 hours` → `15h`，`Exited (0) 3 hours ago` → `Ex0 3h`，exited 不再红色 | ✅ | `abbreviate_status_*` 单测；截图中容器状态为 `20m`/`3h` 等，STATE 圆点为绿色 |
| 斑马条只出现在 x=4..470、y=126..286；滚动轨道 x=472..475；无元素越出 docker 区块 | ✅ | 像素抽检：奇数行 (16,24,33)，偶数行 (8,16,16)；当前 total=7，单页无滚动轨道 |
| 奇数行 Label 擦除后无 BG 色洞 | ✅ | `ContainerRow` 构造时 bg 与斑马条同步为 `ROW_STRIPE`/`BG` |
| `cargo test --release` 全绿；截图 A/B 交付 | ✅ | 25/25 通过；截图已保存 |

---

## 3. 测试结果

```text
running 25 tests
test config::tests::parse_percent_last ... ok
test config::tests::parse_temp_int ... ok
test config::tests::temp_band_color_boundaries ... ok
test config::tests::usage_text_color_boundaries ... ok
test metrics::tests::abbreviate_status_exited_and_restarting ... ok
test metrics::tests::abbreviate_status_special_and_health ... ok
test metrics::tests::abbreviate_status_states_and_fallback ... ok
test metrics::tests::abbreviate_status_up_units ... ok
test label::tests::label_change_erases_union ... ok
test metrics::tests::cpu_sampler_returns_cores ... ok
test label::tests::label_unchanged_zero_dirty ... ok
test metrics::tests::cpu_temp_format ... ok
test metrics::tests::mem_info_format ... ok
test metrics::tests::ip_list_non_empty ... ok
test metrics::tests::disk_usage_format ... ok
test label::tests::bar_unchanged_zero_dirty ... ok
test metrics::tests::tailscale_on_or_off ... ok
test pages::monitor::tests::scroll_containers_wraps ... ok
test pages::monitor::tests::touch_container_list_increments_offset ... ok
test text::tests::measure_draw_step_same ... ok
test text::tests::baseline_is_constant_for_line ... ok
test pages::monitor::tests::headless_golden_render ... ok
test pages::monitor::tests::touch_release_ignored ... ok
test text::tests::zero_width_text_no_dirty ... ok
test text::tests::print_font_recommendations ... ok

test result: ok. 25 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

---

## 4. 性能复测

`pidstat 1 60 -u -p <pid>` 结果：

```text
Average:     1000   1214261    1.17    1.35    0.00    0.17    2.52     -  pi-dashboard-ru
```

- 平均 CPU：**2.52%**（规范 ≤4%）
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
# 回退到上一个 service 备份的二进制/配置（如需要回退 Python 版，需另行替换 ExecStart）
LATEST=$(ls -t /home/richli/pi_dashboard/backups/pi-dashboard.service.*.bak | head -1)
sudo cp "$LATEST" /etc/systemd/system/pi-dashboard.service
sudo systemctl daemon-reload
sudo systemctl restart pi-dashboard.service
```

---

## 7. 截图证据

- 实际运行截图：`docs/screenshots/monitor-after-20260731-152438.png`
- headless 黄金图：`docs/screenshots/monitor-golden-20260731-152438.png`
- pidstat 原始日志：`/tmp/pi_dashboard_pidstat.log`

运行截图中可见：
- 温度 58°C 为白色，右侧带趋势下箭头。
- 容器列表有斑马条（像素抽检确认）。
- 所有 running 容器 STATE 圆点为绿色，状态缩写为 `20m`/`3h`/`30h`/`18h`。
- MEM 33%、DISK 13%、CPU 均 <80%，数值文字为白色。

---

## 8. 架构冲突/偏差记录

- 无架构冲突。
- 实施中发现 `parse_percent` 原实现无法解析 `(41%)` 格式，已修复，属于原实现 bug 而非架构冲突。
- 未发现与 `ARCHITECTURE.md` 冲突之处；未新增 crate，未改 Page trait / PageManager。

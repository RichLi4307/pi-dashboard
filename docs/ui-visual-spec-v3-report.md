# Pi Dashboard 视觉规范 v3 实施报告

> 实施时间：2026-08-01  
> 涉及文件：`rust/src/config.rs`、`rust/src/render.rs`、`rust/src/label.rs`、`rust/src/pages/monitor.rs`  
> 部署目标：`FocusRasPi4B`（树莓派 4B，aarch64）

---

## 1. 变更摘要

按 `docs/ui-visual-spec-v3.md` 逐项修正 monitor 页 UI 细节：

| 编号 | 内容 | 状态 |
|---|---|---|
| v3-1 | `fill_triangle` 方向修复（`1.0 - ratio` → `ratio`）并补方向单测 | ✅ |
| v3-2 | Hero 卡文字垂直居中：label `y+3`、value `y+19` | ✅ |
| v3-3 | 温度趋势控件原子化：10×9 三角/10×3 横杠，`cy = value_top + 9`，值+箭头+`!` 同 bbox 同帧重绘 | ✅ |
| v3-4 | Docker 表内容内缩、三列（UPTIME/STATE/CPU）按中心 274/358/430 居中 | ✅ |
| v3-5 | CPU 区 cell 拉开、bar `x=cx+34`/`w=143`、pct 右对齐 cell 右缘 | ✅ |
| v3-6 | CPU 条填充内缩 1px，补像素级单测 | ✅ |
| v3-7 | 底栏中央显示 IP（最多 2 个，` · ` 连接），`y = H-17` | ✅ |
| v3-8 | `config.rs` 新增 `hostname()`，monitor.rs 无硬编码主机名 | ✅ |

---

## 2. 验证结果

### 2.1 单元测试

```bash
cargo test --release
```

结果：**33 passed; 0 failed**（新增 4 个测试：三角形方向×2、CPU 条内缩、hostname 回退链）

### 2.2 性能复测

```bash
pidstat 1 30
```

| 进程 | 平均 CPU | 最大 CPU |
|---|---|---|
| `pi-dashboard-ru` | **1.69%** | **3.00%** |
| `dockerd` | 15.62% | 24% |
| `containerd` | 14.79% | 25% |

面板进程自身 **≤4%**，满足性能预算。

### 2.3 部署后截图

- 路径：`docs/screenshots/monitor-v3-20260801-0038.png`
- IPC screenshot action 获取，原始尺寸 480×320

目视检查：
- [x] Hero 卡文字块垂直居中，值不溢出卡片底缘
- [x] 61°C 显示 OK 绿 + 蓝色下降箭头（v3-1 方向修复后方向正确）
- [x] Docker 三列内容与表头同中心对位
- [x] CPU 条填充内缩，左缘绿色轨道轮廓可见
- [x] 底栏：左侧署名、中央 IP、右侧 FPS
- [x] 顶栏主机名 `FocusRasPi4B` 来自 `/etc/hostname`

---

## 3. 回退

```bash
LATEST=$(ls -t /home/richli/pi_dashboard/backups/pi-dashboard.service.*.bak | head -1)
sudo cp "$LATEST" /etc/systemd/system/pi-dashboard.service
sudo systemctl daemon-reload
sudo systemctl restart pi-dashboard.service
```

---

## 4. 交付物

- 代码改动：`rust/src/config.rs`、`rust/src/render.rs`、`rust/src/label.rs`、`rust/src/pages/monitor.rs`
- 本报告：`docs/ui-visual-spec-v3-report.md`
- 部署后截图：`docs/screenshots/monitor-v3-20260801-0038.png`

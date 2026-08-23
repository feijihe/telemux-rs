# 运维指南（阶段 6：可观测性与运维）

本文档覆盖网关的日志、停机、守护进程与健康检查能力（阶段 6）。

## 1. 日志（6.1）

### 控制台

默认输出到 stdout，级别由 `--log-level` 或 `RUST_LOG` 控制（默认 `info`）：

```bash
telemux --config config/example.toml --log-level debug
```

### 滚动文件

在配置的 `[general]` 段设置 `log_dir` 后，日志同时写入滚动文件：

```toml
[general]
log_level = "info"
log_dir = "logs"        # 按日轮转，文件名 telemux.YYYY-MM-DD
log_max_files = 7       # 保留最近 7 个文件，自动删除最旧
```

- 文件层记录 **TRACE 级全量**（不受 stdout 过滤级别限制），便于排障；
- 按日轮转（`Rotation::DAILY`），超过 `log_max_files` 自动清理；
- 退出时日志 guard flush 保证最后一条日志落盘（见 6.2）。

## 2. 优雅停机（6.2）

停止顺序：收到信号 → 主循环退出 → 通知协议/采集任务停止 →
等待全部任务退出 → flush 日志 → 进程退出。

- **Linux/macOS**：`SIGINT`（Ctrl+C）与 `SIGTERM`（`systemctl stop` / `kill`）均支持；
- **Windows**：控制台 `Ctrl+C`；作为服务时由 SCM 的 Stop/Shutdown 控制事件触发。

## 3. 守护进程（6.3）

### Linux systemd

`deploy/telemux.service` 已提供，安装步骤见文件头注释：

```bash
cargo build --release
sudo cp target/release/telemux /usr/local/bin/
sudo mkdir -p /etc/telemux /var/log/telemux
sudo cp config/example.toml /etc/telemux/telemux.toml
# 在 telemux.toml 中设置 log_dir = "/var/log/telemux"
sudo cp deploy/telemux.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now telemux
sudo systemctl status telemux
```

### Windows Service

Windows 构建产物支持作为系统服务运行（`windows-service` crate）：

```powershell
# 安装（需管理员 PowerShell；默认 LocalSystem 账户）
telemux.exe --install-service --config D:\path\to\telemux.toml
# 启动 / 停止
sc start telemux
sc stop telemux
# 查看状态
sc query telemux
# 卸载
sc stop telemux
telemux.exe --uninstall-service
```

- 服务名 `telemux`，自启动（AutoStart），崩溃后 SCM 自动重启；
- 服务会话无控制台，**必须在配置中设置 `log_dir`**，否则日志不可见；
- `--service` 参数由 SCM 在启动时自动追加，请勿手动以该参数运行。

## 4. 健康/就绪端点（6.4）

独立 HTTP 服务（默认 `127.0.0.1:8081`，配置见 `[endpoints]`）：

```toml
[endpoints]
health_enabled = true
health_port = 8081
```

| 端点 | 语义 | 返回 |
|---|---|---|
| `GET /healthz` | 存活探测（进程活着） | `200 {"status":"ok"}` |
| `GET /readyz` | 就绪探测（至少一台设备近期有数据） | `200` + 设备/端点状态，否则 `503` |

`/readyz` 示例：

```json
{
  "status": "ready",
  "devices": [
    { "name": "pcba-01", "connected": true, "sensors_total": 7, "sensors_with_data": 7 }
  ],
  "endpoints": {
    "redfish": { "enabled": true, "port": 8000 },
    "modbus":  { "enabled": true, "port": 1503 },
    "health":  { "enabled": true, "port": 8081 }
  },
  "computed_sensors": 0
}
```

用途：systemd/容器/K8s 的 `ExecStartPost`、`livenessProbe`、`readinessProbe`
可直接指向 `/healthz` 与 `/readyz`。

## 5. 快速验证

```bash
# 终端 1：模拟 PCBA
cargo run --example mock_pcba 1502
# 终端 2：网关（阶段 6 演示配置：日志目录 logs + 健康端点 8081）
cargo run -- --config config/test-p6.toml
# 终端 3：健康检查
curl -s http://127.0.0.1:8081/healthz   # {"status":"ok"}
curl -s http://127.0.0.1:8081/readyz    # 200 + 状态 JSON
ls logs/                                # telemux.YYYY-MM-DD 滚动文件
```

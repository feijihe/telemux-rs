# 部署指南（阶段 8：打包发布）

本文档说明如何构建、安装、部署 Telmux-rs 网关到生产环境。

## 1. 构建发布产物

```bash
cargo build --release
```

产物：
- `target/release/telemux`（Linux）/ `telemux.exe`（Windows）—— 网关主程序；
- `target/release/examples/mock_pcba` —— 仅开发桩（release 下打印提示后退出）。

Release 优化（见 `Cargo.toml [profile.release]`）：
- `lto = true` + `codegen-units = 1`：全程序链接优化，体积更小；
- `strip = true`：剥离符号（Windows 下 telemux.exe 约 2.7 MB）；
- `panic = "abort"`：panic 直接中止，无 unwind 开销（嵌入式/网关场景更稳）。

**Release 二进制不含任何开发功能**（已字符串级验证）：dashboard 模块、
mock PCBA、动态配置（`ConfigHandle.update/save` 编译期不存在）全部剥离。

## 2. Linux 部署

一键脚本（构建 + 安装 + systemd）：

```bash
sudo ./deploy/install.sh                      # 默认用 config/example.toml
sudo ./deploy/install.sh config/my.toml       # 指定生产配置
```

或手动：

```bash
sudo install -m 0755 target/release/telemux /usr/local/bin/
sudo mkdir -p /etc/telemux /var/log/telemux
sudo install -m 0644 config/example.toml /etc/telemux/telemux.toml
# 编辑 /etc/telemux/telemux.toml：log_dir = "/var/log/telemux"、设备清单
sudo install -m 0644 deploy/telemux.service /etc/systemd/system/
sudo systemctl daemon-reload && sudo systemctl enable --now telemux
```

验证：`systemctl status telemux`、`curl localhost:8081/healthz`。

## 3. Windows 部署

### 前台运行

```powershell
telemux.exe --config config\example.toml
```

### Windows 服务（推荐生产）

```powershell
# 管理员 PowerShell
deploy\install.bat D:\path\to\telemux.toml   # 构建 + 复制 + 交互式安装服务
# 或手动
telemux.exe --install-service --config D:\path\to\telemux.toml
sc start telemux
sc query telemux          # 状态
sc stop telemux
telemux.exe --uninstall-service
```

服务名 `telemux`，LocalSystem 账户，自启动。**必须**在配置中设置 `log_dir`
（服务会话无控制台，日志只能看滚动文件）。

## 4. 配置交付清单

| 文件 | 用途 |
|---|---|
| `config/example.toml` | 演示配置（mock 寄存器集 + 5 条管道） |
| `config/test-p5.toml` | 阶段 5 验收配置（R/W、bit、computed、三端点） |
| `config/test-p6.toml` | 阶段 6 验收配置（滚动日志 + 健康端点） |
| `deploy/telemux.service` | Linux systemd 单元 |
| `deploy/install.sh` / `install.bat` | 一键安装脚本 |
| `docs/OPERATIONS.md` | 日志/停机/守护/健康检查运维指南 |

## 5. 生产检查清单

- [ ] `[general] log_dir` 指向持久化目录（systemd 服务必须）
- [ ] 设备清单/寄存器映射与实际 PCBA 一致（address/function/value_type）
- [ ] 端口规划：Redfish 8000、Modbus 1503、健康 8081（`[endpoints]`）
- [ ] 只读传感器未设 `access = "read_write"`
- [ ] 防火墙放行所需端口；`/healthz` 与 `/readyz` 挂到监控/编排系统
- [ ] 首次启动后检查日志无 `pipeline failed` / 连接错误

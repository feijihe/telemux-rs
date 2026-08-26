# telemux（网关 crate）

Telmux-rs 的生产二进制：**Modbus 采集 → 处理管道 → Redfish/Modbus 出口**。

采集层通过 `tokio-modbus` 以 Modbus-TCP/RTU 读取远端 PCBA 传感器原始值，经可配置的处理管道（单位换算、滤波、计算、阈值判断）得到标准化指标，再通过 Redfish、Modbus 服务对外暴露。

## 目录

- `src/` — `config`（TOML 配置）、`acquisition`（采集）、`pipeline`（处理管道）、`protocol`（Redfish/Modbus 服务）、`dashboard`（dev 仪表盘）、`mock`（dev 模拟从站）等
- `config/` — 示例配置：`example.toml`、`test-p5.toml`、`test-p6.toml`、`cdu-gateway.toml`（对接模拟器）
- `docs/` — `IMPLEMENTATION.md`、`REDFISH.md`、`MODBUS_SERVER.md`、`SIMULATION.md`、`OPERATIONS.md`、`DEPLOYMENT.md`、`DEV_DASHBOARD.md`
- `tests/` — 集成测试（含与模拟器闭环的 `cdu_sim.rs`）
- `web/dist/` — 前端构建产物，由 `web/apps/dashboard` 生成，编译期经 `include_dir!` 嵌入

## 运行

```bash
cargo run -p telemux -- --config crates/telemux/config/example.toml
# 对接 CDU 模拟器
cargo run -p telemux -- --config crates/telemux/config/cdu-gateway.toml
```

默认端口：Redfish `8000`、Modbus 从站 `1503`、健康检查 `8081`。

## 构建注意

- `mock` 与 `dashboard` 模块由 `cfg(any(debug_assertions, feature = "dev-dashboard"))` 门控，release 构建默认排除；需要开发仪表盘时用 `cargo build --release --features dev-dashboard`。
- Windows 服务（install/uninstall/run）为 `#[cfg(windows)]` 专属，`windows-service` 依赖亦为目标门控。
- 处理管道**不是 `Send`**（`meval` 阶段持有 `Rc`），在主循环内联运行，勿移入 `tokio::spawn`。
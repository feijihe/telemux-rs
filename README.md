# Telmux‑rs

> PCBA 板载传感器遥测多路网关，Rust 实现。采集板卡原始传感器数据，本地运算处理，对外提供 Redfish / Modbus 多协议访问接口。
>
> 采用 **cargo workspace**（monorepo）组织，`crates/telemux`（网关）+ `crates/telemux-sim`（CDU 仿真器），单一 `Cargo.lock` 保证依赖版本一致。

## Overview

**Telmux‑rs** 是部署在 Windows / Linux 主机上的遥测网关服务：
1. 通过 Modbus‑TCP / Modbus‑RTU 与远端 PCBA 通信，读取板卡原始寄存器数据（温度、电压、电流、风扇转速等传感器原始值）；
2. 在主机本地完成单位换算、滑动滤波、数学计算、阈值判断、统计聚合等业务处理；
3. 将处理完成的标准化指标，对外同时暴露 **Redfish、Modbus** 接口；
4. 上层管理平台、SCADA、机房运维系统可任选协议读取 PCBA 的传感器状态。

> **SNMP** 因 Rust 生态评估暂缓（见 `docs/IMPLEMENTATION.md` 阶段 5 说明）。

## Repository Layout

```text
telemux-rs/
├── Cargo.toml               # workspace 根（单一 Cargo.lock）
└── crates/
    ├── telemux/             # 网关：Modbus 采集 → 处理 → Redfish/Modbus 出口
    │   ├── src/             # config/app/protocol/acquisition/pipeline/...
    │   ├── config/          # 示例配置（example/test-p5/test-p6/cdu-gateway.toml）
    │   ├── tests/           # 集成测试（全链路/断线重连/协议/仿真闭环）
    │   └── docs/            # IMPLEMENTATION/REDFISH/MODBUS_SERVER/SIMULATION/...
    └── telemux-sim/         # CDU 仿真器：物理模型 + Modbus-TCP 从站 + 网页 UI
        ├── src/             # model/registers/server/web + main
        └── config/cdu.toml  # 仿真 CDU 配置（传感器布局 + 物理因果）
```

## workspace（monorepo）

单一 `Cargo.lock` 锁定全仓依赖版本（tokio-modbus / meval 等），两个 crate 不会出现版本漂移。
网关与模拟器**彻底解耦**：网关只依赖 Modbus 协议（`transport = "tcp"`），release 二进制
经字符串级验证**不含任何仿真代码**；模拟器作为独立进程以 Modbus-TCP 从站暴露寄存器地
址，供网关读取与控制（与连接真实 CDU 完全同构）。

## Building

```bash
# 全仓（两个二进制）
cargo build --workspace
# 或构建 release
cargo build --release --workspace
```

产物：

| 二进制 | 说明 |
|---|---|
| `target/release/telemux` | 网关主程序（LTO+strip+panic=abort，≈2.7 MB） |
| `target/release/telemux-sim` | CDU 仿真器（物理模型 + Modbus-TCP 从站 + 网页 UI，≈1.8 MB） |

## Running telemux（网关）

```bash
# 前台运行（示例配置：mock PCBA 寄存器集）
cargo run -p telemux -- --config crates/telemux/config/example.toml

# 指定日志级别
cargo run -p telemux -- --config config/example.toml --log-level debug

# 阶段 6 演示（滚动文件日志 + 健康端点）
cargo run -p telemux -- --config crates/telemux/config/test-p6.toml

# 对接 CDU 仿真器（与真实 CDU 同构）
cargo run -p telemux -- --config crates/telemux/config/cdu-gateway.toml
```

网关启动的端口（见 `[endpoints]`）：
- **Redfish**：默认 `8000`
- **Modbus 服务器（从站）**：默认 `1503`
- **健康/就绪**：默认 `8081`（`/healthz`、`/readyz`）

## Running telemux-sim（CDU 仿真器）

无需真实硬件，用配置驱动的**稳态代数模型**模拟一台液冷 CDU 的全部传感器，
供开发 / 测试 / 验收使用。它以 **Modbus-TCP 从站**暴露寄存器地址，同时提供
网页 UI 观察与设定。

```bash
# 启动模拟器（Modbus-TCP 从站 1502 + 网页 UI 8082）
cargo run -p telemux-sim -- --config crates/telemux-sim/config/cdu.toml \
    --modbus-port 1502 --web-port 8082

# 打开网页控制台
#   http://127.0.0.1:8082
#   · Canvas 二维系统图（PHEX 板换 + 一次/二次回路 + 泵 + 全部传感器标点，实时值）
#   · 温度设定（一次侧冷水 / 二次侧热水）+ 泵/阀/风扇 duty，立即联动全系统温度
#   · 寄存器地图原始值表
```

模拟器与网关的完整闭环（见 `docs/SIMULATION.md`）：

```text
telemux-sim (1502) ── Modbus ──> telemux 网关 (1503/8000)
  物理模型：泵/阀/换热因果      WriteBroker 转发写 duty → 模型联动
```

## Development

```bash
# 测试（全仓，含集成测试）
cargo test --workspace

# 仅网关 / 仅模拟器
cargo test -p telemux
cargo test -p telemux-sim

# 静态检查（clippy，全仓）
cargo clippy --workspace --all-targets

# 格式化
cargo fmt --all
```

### 端到端验证（网关 ↔ 模拟器）

```bash
# 终端 1：模拟器
cargo run -p telemux-sim -- --config crates/telemux-sim/config/cdu.toml --modbus-port 1502 --web-port 8082
# 终端 2：网关（连模拟器）
cargo run -p telemux -- --config crates/telemux/config/cdu-gateway.toml
# 终端 3：查看
curl localhost:8081/readyz
curl localhost:8000/redfish/v1/Chassis/cdu-01/Sensors
# 集成测试会验证：Modbus 写 duty → 模拟器 → 流量/压差联动
cargo test -p telemux --test cdu_sim
```

## Architecture

```
[PCBA Sensors / CDU]
       ↓（Modbus 原始 raw 数据读取）
┌─────────────────────┐
│  Acquisition Layer  │ 传感器抽象采集层（模拟器/真实设备均走同一 SensorSource）
└──────────┬──────────┘
           ↓
┌─────────────────────┐
│ Processing Pipeline │ 滤波、换算、计算、统计、阈值判断
└──────────┬──────────┘
           ↓
┌─────────────────────┐
│    Metric Store     │ 处理后指标内存存储
└──────────┬──────────┘
           ↓
     ┌─────┴─────┬────────┐
     ▼           ▼        ▼
 Redfish       SNMP     Modbus
 (HTTP)       (UDP)   (TCP Server)
```

## Features

- 传感器硬件抽象层，便于接入新的板载传感器 / 仿真设备
- 可配置数据处理流水线：均值、滑动滤波、单位转换、超限判断
- 可配置的 CDU 仿真（`telemux-sim`）：物理点 + 派生量（computed）建模，无硬件驱动全链路
- 多协议并行对外服务，共享同一套处理后的传感器指标
- TOML 配置文件驱动，无需修改代码即可映射寄存器 / Redfish 资源 / 处理管道
- 滚动文件日志 + 结构化输出；优雅停机（SIGINT/SIGTERM）；Windows Service / systemd 守护
- 健康/就绪端点（`/healthz`、`/readyz`）；低资源开销，适配嵌入式、BMC 受限环境

## Documentation

- `docs/IMPLEMENTATION.md` — 分阶段实现计划与进度（阶段 0-8 ✅ + Dev Dashboard）
- `docs/REDFISH.md` — Redfish 服务与读写
- `docs/MODBUS_SERVER.md` — Modbus 服务器（四区映射 / 地址分配 / 读写）
- `docs/SIMULATION.md` — telemux-sim 建模与协议契约
- `docs/OPERATIONS.md` — 日志/停机/守护/健康检查运维
- `docs/DEPLOYMENT.md` — 构建 / 安装 / 部署
- `docs/DEV_DASHBOARD.md` — 开发调试面板（dev 构建）

# CDU 仿真（telemux-sim）指南

独立模拟器：用配置驱动的**稳态代数模型**模拟一台液冷 CDU 的全部传感器，
作为 **Modbus-TCP 从站**暴露寄存器地址，供 Telemux 网关读取与控制——
与连接真实 CDU 完全同构（**网关不依赖模拟器**，只依赖 Modbus 协议）。

workspace 结构（单一 Cargo.lock，保证 tokio-modbus/meval 等依赖版本一致）：

```text
telemux-rs/
├── Cargo.toml               # [workspace] members = ["crates/*"]
└── crates/
    ├── telemux/             # 网关（Modbus 采集 → Redfish/Modbus 出口）
    └── telemux-sim/         # 模拟器（物理模型 + Modbus-TCP 从站 + 网页 UI）
```

## 1. 快速开始（两个进程）

```bash
# 终端 1：模拟器（Modbus-TCP 从站 1502 + 网页 UI 8082）
cargo run -p telemux-sim -- --config crates/telemux-sim/config/cdu.toml \
    --modbus-port 1502 --web-port 8082

# 终端 2：网关（连模拟器，与真实 CDU 同构）
cargo run -p telemux -- --config crates/telemux/config/cdu-gateway.toml

# 网页 UI（观察所有寄存器地址/类型/原始值）
打开 http://127.0.0.1:8082

# 网关出口
curl localhost:8081/readyz
curl localhost:8000/redfish/v1/Chassis/cdu-01/Sensors
```

## 2. 模拟器配置（crates/telemux-sim/config/cdu.toml）

与网关原 `[sim]` 段一致：`[[sim.controls]]`（duty，可写）+ `[[sim.sensors]]`
（物理测量点，`formula` meval 稳态表达式 + `inputs` 显式依赖映射）。

```toml
[sim]
[[sim.controls]]
name = "pump1_duty"     # 公式引用名
initial = 50
unit = "%"
writable = true         # true → 暴露为保持寄存器（u16 0-100），可写

[[sim.sensors]]
sensor_id = "cdu.pump1.out_p"
name = "Pump1 Outlet Pressure"
kind = "pressure"
unit = "kPa"
formula = "p_in + pump1_duty^2 * 0.08"   # 离心泵扬程 ∝ duty²
[sim.sensors.inputs]
p_in = "cdu.pump1.in_p"
```

变量解析规则：`inputs` 映射 → 控制变量名 → 内置时间 `t` → 传感器短名。

## 3. Modbus 寄存器地图（模拟器 ↔ 网关契约）

| 区域 | 地址 | 内容 | 方向 |
|---|---|---|---|
| 保持寄存器 | 0x0000 起 | 控制变量（u16，0-100） | **读写**（网关写 → 驱动模型） |
| 输入寄存器 | 0x0000 起 | 传感器（f32 双字，Big 字序） | 只读 |

地址按配置顺序确定性 append。网关侧 `crates/telemux/config/cdu-gateway.toml`
把 `[[devices.registers]]` 映射到这些地址（`function="holding"` 的 duty 可写，
`function="input"` 的传感器 f32 双字）。

## 4. 网页 UI

`http://127.0.0.1:8082`：展示
- **控制变量表**（保持寄存器地址 + 值 + 可写性）；
- **传感器表**（sensor_id/类型/值/公式）；
- **寄存器地图原始值表**（区域/地址/槽位/原始 u16/解码 f32），每 1s 刷新。

API：`GET /api/state` 返回全部上述 JSON。

## 5. 物理建模原则：物理点在 sim，派生量用 computed（网关侧）

真实系统没有 dp 探头：泵压差 = 出口压力 − 进口压力，是两路压力传感器差分。
因此模拟器只产出**独立物理测量点**；**压差/温差**等派生量在网关侧用
`[[computed]]` 计算（`cdu-gateway.toml` 已含 pump1.dp/pump2.dp/delta_t），
与数据来源无关——接真实 CDU 时 computed 原样保留。

## 6. 完整闭环（已验证）

```
telemux-sim                     telemux 网关
┌─────────────────┐    Modbus    ┌──────────────────────┐
│ 物理模型         │◄───写 duty───│ 保持寄存器（read_write）│
│  泵/阀/换热因果   │              │  WriteBroker 转发      │
│ 传感器值 → 输入区 │───读──────→ │ 输入寄存器（f32）      │
└─────────────────┘              │ → 管道 → computed →    │
                                 │   Redfish/Modbus/健康   │
                                 └──────────────────────┘
```

`crates/telemux/tests/cdu_sim.rs` 集成测试验证：
- 模拟器读（duty + 传感器 f32）；
- 模拟器写 duty → 流量联动（写 pump1_duty=80 → f2 109→142 L/min）；
- **网关写 duty=90 → 模拟器 → f2=153 L/min**（完整闭环）。

## 7. 生产切换

接真实 CDU：模拟器进程停掉，网关配置改真实 CDU 的 IP/端口与寄存器映射
（保持 `[[computed]]` 派生量）。模拟器可作为**开发/测试/验收环境**，
与生产使用同一份网关配置结构。

## 8. 局限（当前实现）

- 稳态代数模型：公式直接给稳态值，无惯性/延迟；
- 换热模型定性正确（一次侧冷水流量↑ → 二次侧温度↓），系数需按真实 CDU 标定；
- 公式引用错误降级为 warn 并跳过该传感器。

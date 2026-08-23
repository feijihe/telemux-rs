# CDU 仿真（Simulation）指南

阶段 8 扩展：用配置驱动的**稳态代数模型**模拟一台液冷 CDU 的全部传感器，
无真实硬件即可驱动整个网关链路（采集 → 管道 → 存储 → Redfish/Modbus/健康端点）。

示例配置：`config/cdu.toml`（29 个仿真传感器 + 4 个控制变量 + 4 个派生量）。

## 1. 快速开始

```bash
# 启动网关（仿真 CDU，无外部硬件）
cargo run -- --config config/cdu.toml

# 健康端点
curl localhost:8081/readyz          # status: ready, sensors_total: 30
# Redfish 传感器集合
curl localhost:8000/redfish/v1/Chassis/cdu-01/Sensors
```

## 2. 配置结构

### 设备

```toml
[[devices]]
name = "cdu-01"
transport = "sim"       # 关键：仿真数据源（无需 host/port/串口/寄存器）
unit_id = 1
poll_interval_ms = 1000
```

### 控制变量（`[[sim.controls]]`）

泵/阀/风扇的占空比等，是仿真的**输入**，可被协议层写入（预留写接口）：

```toml
[[sim.controls]]
name = "pump1_duty"     # 公式中引用此名字
initial = 50            # 初始值
unit = "%"
writable = true         # true → 映射为 Modbus 保持寄存器（u16 0-100），可写
```

### 仿真传感器（`[[sim.sensors]]`）

每个传感器是一个**物理测量点**，由 `formula` 稳态表达式求值：

```toml
[[sim.sensors]]
sensor_id = "cdu.pump1.out_p"
name = "Pump1 Outlet Pressure"
kind = "pressure"       # pressure/temperature/flow/level/ph/leak/humidity
unit = "kPa"
formula = "p_in + pump1_duty^2 * 0.08"   # 离心泵扬程 ∝ duty²
[sim.sensors.inputs]    # 显式依赖映射（与 [[computed]] 一致）
p_in = "cdu.pump1.in_p"
```

**变量解析规则**（formula 中可引用）：
1. `inputs` 映射的键 → 引用的传感器 id / 控制变量名（推荐，无歧义）；
2. 控制变量名（如 `pump1_duty`）；
3. 内置时间 `t`（自启动秒数，用于 `sin(t)` 等缓慢波动）；
4. 其他传感器的短名（sensor_id 末段，如 `cdu.pri.p1` → `p1`）——仅在无歧义时可用。

> 注：meval 变量名不能含 `.`，所以跨传感器引用**必须**用 `inputs` 显式映射
> （与 `[[computed]]` 的 `inputs` 语义完全一致）。

## 3. 物理建模原则：物理点在 sim，派生量用 computed

**真实系统没有 dp 探头**：泵压差 = 出口压力 − 进口压力，是两路压力传感器
的差分计算值。因此：

- `[sim]` 只放**独立存在的物理测量点**（对应系统图上的 Pn/Tn/Fn 各自位置）；
- **压差、温差、效率**等派生量一律用现有 `[[computed]]` 表达：

```toml
[[computed]]
sensor_id = "cdu.pump1.dp"
name = "Pump1 Differential Pressure"
unit = "kPa"
expression = "p_out - p_in"
[computed.inputs]
p_out = "cdu.pump1.out_p"
p_in = "cdu.pump1.in_p"
```

好处：sim 保持"硬件清单"纯净语义；computed 复用已有派生机制（协议出口、
按 unit 分类）；**生产切换时 computed 原样保留**。

## 4. 因果链示例（config/cdu.toml）

```
pump1_duty↑  →  f2_flow↑  (10 + (pump1+pump2)*1.1)
             →  out_p↑    (in_p + duty² * 0.08)  →  dp (computed)↑
valve1_duty↑ →  f1_flow↑  (8 + duty * 0.9)
f1_flow↑     →  t7/t8↓    (t5 - f1 * 0.18，一次侧冷水降温)
fan_duty↑    →  t5/t6↓    (回水温度降低)
```

## 5. 协议映射

| 对象 | Modbus 区域 | 说明 |
|---|---|---|
| 仿真传感器 | 输入寄存器 | f32（每传感器 2 字），computed 之后追加 |
| computed 派生量 | 输入寄存器 | f32，位于 sim 传感器之前（先 computed 后 sim） |
| 可写控制变量 | 保持寄存器 | u16（0-100），`writable=true`，写保持寄存器即更新 duty |

写链路：Modbus 写保持寄存器 → WriteBroker → sim 设备轮询任务 →
`SimSource.write_holding_register` → 更新控制变量 → 下一轮采集生效。

## 6. 生产切换

同一份配置，把设备 `transport` 从 `"sim"` 改回 `"tcp"`/`"rtu"` 即接真实 CDU：
- `[[computed]]` 派生量**原样保留**（本就在指标层计算）；
- `[sim]` 段可保留（不再被引用）或删除；
- 需按真实寄存器补 `[[devices.registers]]` 映射。

## 7. 局限（当前实现）

- **稳态代数模型**：公式直接给出稳态值，无惯性/延迟（温度不会"缓变"，
  是瞬间跳到新稳态）。如需动态（一阶惯性、爬升/回落），留待后续扩展。
- 换热模型为**定性正确**（一次侧冷水流量↑ → 二次侧温度↓），系数需按
  真实 CDU 标定；精确传热方程（LMTD/UA）未实现。
- 公式引用错误（未知变量）降级为 warn 并跳过该传感器，不中断采集。

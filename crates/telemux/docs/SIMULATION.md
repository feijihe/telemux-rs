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

与网关原 `[sim]` 段一致：`[[sim.controls]]`（duty，可写）+ 传感器按回路与出入口分组。
传感器按**回路侧 → 出入口**两级组织：
- 一次侧（冷水回路）：`[[sim.pri.in]]`（入口）、`[[sim.pri.out]]`（出口）、`[[sim.pri.aux]]`（流量/液位等辅助）
- 二次侧（热水回路）：`[[sim.sec.in]]`、`[[sim.sec.out]]`、`[[sim.sec.aux]]`（流量 + 泵）
- 水箱/环境/泄漏等全局传感器用 `[[sim.sensors]]` 平铺

代码层（`SimConfig.iter_sensors()`）按 **一次侧 in/out/aux → 二次侧 in/out/aux → 未分组**
顺序扁平化。默认寄存器地址按组连续划分（保持区=控制、输入区=传感器，均从 0 起）。

### 显式地址与 u16 原始寄存器（cdu2 适配）

`config/cdu2.toml` 由外部 CDU 寄存器映射 `cdu2.yaml` 改造而来，演示两类扩展能力：

- **显式地址**：`SimControl.address`（保持寄存器）与 `SimSensor.address`（输入或
  保持寄存器）指定寄存器地址（0x0000 起，稀疏填充）。缺省仍按顺序紧凑分配
  （兼容原 cdu.toml）。
- **寄存器区**：`SimSensor.area = "input"`（默认，输入寄存器 0x04）或
  `"holding"`（保持寄存器 0x03 的**只读槽位**，对齐真实 CDU 中
  `read_holding_registers` 的测量点——Modbus 工具用功能码 03 即可读取）。
- **u16 单字存储**：`SimSensor.storage = "u16"` 使传感器占 1 个寄存器（默认
  `"f32"` 占 2 字 Big 字序）。`encode` 给出物理值 → 原始整数的编码表达式（变量
  `v`），使模拟器暴露的原始寄存器与真实 CDU 一致——网关按 cdu2.yaml 的解码公式
  （如 T = raw/10）即可还原物理值。

物理分组（cdu2.yaml → cdu2.toml）：F1/T1/P1 → `pri.in`，T2/P2 → `pri.out`，
T3/P3 → `sec.out`，T4/P4 → `sec.in`；泵/阀/环境/派生量按语义归 aux 或全局。
线圈设备（LE1/LI1/LI2）暂不模拟。

```toml
[sim]
[[sim.controls]]
name = "pump1_duty"     # 公式引用名
initial = 50
unit = "%"
writable = true         # true → 暴露为保持寄存器（u16 0-100），可写
address = 2192          # 显式保持寄存器地址（对应 cdu2.yaml Pump1.DutyCycle）

[[sim.pri.in]]
sensor_id = "cdu.pri.in.t1"
name = "Primary Inlet T1 (cold)"
kind = "temperature"
unit = "°C"
formula = "12 + 0.5 * sin(t)"   # 物理值（稳态模型）
address = 3328          # 显式寄存器地址（cdu2.yaml Temperatures.T1）
area = "holding"        # 保持寄存器区（功能码 03 可读，对齐 read_holding_registers）
storage = "u16"         # 单字原始寄存器
encode = "v * 10"       # 物理值 → 原始整数（逆解码公式：raw = 物理 × 10）
```

变量解析规则：`inputs` 映射 → 控制变量名 → 内置时间 `t` → 传感器短名。
跨回路引用（如二次侧 `cdu.sec.t7` 引用一次侧 `cdu.pri.f1`）通过 `inputs` 显式映射即可。
注意校验是**两遍**：先收集全部传感器 id，再校验 `inputs` 目标，故依赖方可以出现在被依赖方之前（分组顺序不限）。

## 3. Modbus 寄存器地图（模拟器 ↔ 网关契约）

| 区域 | 地址 | 内容 | 方向 |
|---|---|---|---|
| 保持寄存器 | 0x0000 起 | 控制变量（u16，0-100）+ `area="holding"` 的只读传感器 | **读**；控制变量可写 |
| 输入寄存器 | 0x0000 起 | 传感器（默认 `area="input"`；f32 双字 Big 字序，或 u16 单字原始值） | 只读 |

地址按配置顺序确定性 append；配置 `address` 字段后可显式指定（稀疏填充）。
网关侧 `crates/telemux/config/cdu-gateway.toml` 把 `[[devices.registers]]`
映射到这些地址（`function="holding"` 的 duty 可写，`function="input"` 的
传感器 f32 双字或 u16 单字）。

## 4. 网页 UI

`http://127.0.0.1:8082`（**React + TypeScript + shadcn/ui**，`web/apps/sim-ui`，
构建产物经 `include_dir!` 嵌入 `web_assets.rs` 服务）：
- **CDU 二维系统图（Canvas）**：PHEX 板换居中，一次侧（冷水）与二次侧（热水）
  回路按管路布局，所有传感器标点显示实时值（温度/压力/流量等按类型配色）；
  泵以并联环插在下管（只显示转速），H1 比例阀旁显示 duty；
- **温度设定面板**：一次侧冷水 `primary_cold_temp`、二次侧热水 `secondary_hot_temp`
  （另有 pump1/2_duty、valve1_duty、fan_duty），输入后点"应用"立即生效，
  整个系统的温度因果链即时联动（已验证：设冷水 12→20°C，T1/T3 与二次侧 T7 同步变化）；
- **寄存器地图原始值表**（区域/地址/槽位/原始 u16/解码 f32），WebSocket 500ms 推送。

开发模式：`pnpm --filter sim-ui dev`（vite，端口 5180，代理 `/api`→8082）。

API：
- `GET /api/state` — 全部状态 JSON；
- `POST /api/control` — 设置控制变量 `{"name","value"}`，立即驱动模型。

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

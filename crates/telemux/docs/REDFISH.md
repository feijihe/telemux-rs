# Redfish 接口文档（阶段 5）

> 状态：设计定稿，待实现。本网关实现 Redfish 的"传感器资源"最小子集
> （只读 + 对 R/W 寄存器的写操作），不追求完整规范。

## 1. 设计原则

- **配置驱动**：所有资源（Chassis / 传感器）由 `ConfigHandle.read()` 中的
  `[[devices]]` + `[[devices.registers]]` + `[[computed]]` 实时构建。
  **手动编辑配置文件新增寄存器后（重启生效），资源自动出现**——协议层不做任何固化。
- **值来自 MetricStore**：`Sensor.Reading` 取计算后的 metric（无 pipeline 取 raw），
  与 dashboard / Modbus 一致。**computed（虚拟传感器）与真实传感器同等暴露**。
- **读写双向**：GET 读全部资源；**PATCH 可写寄存器**（`access=read_write`），
  写入转发到 PCBA（见 §7）。只读资源写返回 `405`。
- 端口：`[endpoints] redfish_port`（默认 `8000`），绑定 `0.0.0.0`（对外服务）。

## 2. 配置（`[endpoints]` 段，与 Modbus 共用）

```toml
[endpoints]
redfish_enabled = true
redfish_port = 8000
modbus_enabled = true
modbus_port = 1503
modbus_unit_id = 1
```

寄存器扩展属性（与 `docs/MODBUS_SERVER.md` §3.1 一致）：

```toml
[[devices.registers]]
name = "fan1_duty"
sensor_id = "pcba-01.fan1_duty"
function = "holding"
access = "read_write"     # read（默认）| read_write —— 决定能否 PATCH
address = 10
value_type = "u16"
unit = "%"

# bit 型（function = coil / discrete_input + value_type = bool）
[[devices.registers]]
name = "leak_detect"
sensor_id = "pcba-01.leak"
function = "discrete_input"
value_type = "bool"
address = 2
unit = "leak"
```

## 3. 资源树

```
GET/PATCH /redfish/v1                              ServiceRoot
└── /redfish/v1/Chassis                            ChassisCollection
    └── /redfish/v1/Chassis/{deviceName}           Chassis（每个 DeviceConfig 一个）
        ├── /redfish/v1/Chassis/{device}/Thermal        Thermal（温度/风扇）
        ├── /redfish/v1/Chassis/{device}/Power          Power（电压）
        └── /redfish/v1/Chassis/{device}/Sensors        SensorCollection
            └── /redfish/v1/Chassis/{device}/Sensors/{sensorId}   Sensor（可 PATCH）
```

`{deviceName}` = 设备 `name`；`{sensorId}` = 寄存器/计算指标的 `sensor_id`。

## 4. 端点与响应示例

> 示例基于扩展配置（温湿度 + 露点 computed + 风扇占空比 R/W + 漏液 bit）：
> env_temp=27°C、env_humidity=60%RH、dew_point=18.6°C、fan1_speed=3250 RPM、
> fan1_duty=60%、leak=false。

### 4.1 ServiceRoot — `GET /redfish/v1`

```json
{
  "@odata.id": "/redfish/v1",
  "@odata.type": "#ServiceRoot.v1_16_0.ServiceRoot",
  "Id": "RootService",
  "Name": "Telemux Redfish Service",
  "RedfishVersion": "1.16.0",
  "UUID": "telemux-0000-0000-0000-000000000001",
  "Chassis": { "@odata.id": "/redfish/v1/Chassis" }
}
```

### 4.2 Chassis — `GET /redfish/v1/Chassis/pcba-01`

```json
{
  "@odata.id": "/redfish/v1/Chassis/pcba-01",
  "@odata.type": "#Chassis.v1_24_0.Chassis",
  "Id": "pcba-01",
  "Name": "pcba-01",
  "ChassisType": "Component",
  "Status": { "State": "Enabled", "Health": "OK" },
  "Thermal": { "@odata.id": "/redfish/v1/Chassis/pcba-01/Thermal" },
  "Power":   { "@odata.id": "/redfish/v1/Chassis/pcba-01/Power" },
  "Sensors": { "@odata.id": "/redfish/v1/Chassis/pcba-01/Sensors" }
}
```

### 4.3 Thermal — `GET /redfish/v1/Chassis/pcba-01/Thermal`

温度（°C/C）进 `Temps[]`；转速（RPM）进 `Fans[]`。

```json
{
  "@odata.id": "/redfish/v1/Chassis/pcba-01/Thermal",
  "@odata.type": "#Thermal.v1_6_2.Thermal",
  "Id": "Thermal",
  "Name": "Thermal",
  "Temps@odata.count": 2,
  "Temps": [
    {
      "@odata.id": "/redfish/v1/Chassis/pcba-01/Sensors/pcba-01.env_temp",
      "MemberId": "pcba-01.env_temp",
      "Name": "env_temp",
      "ReadingCelsius": 27.0,
      "Status": { "State": "Enabled", "Health": "OK" }
    },
    {
      "@odata.id": "/redfish/v1/Chassis/pcba-01/Sensors/pcba-01.dew_point",
      "MemberId": "pcba-01.dew_point",
      "Name": "dew_point",
      "ReadingCelsius": 18.6,
      "Status": { "State": "Enabled", "Health": "OK" }
    }
  ],
  "Fans@odata.count": 1,
  "Fans": [
    {
      "@odata.id": "/redfish/v1/Chassis/pcba-01/Sensors/pcba-01.fan1_speed",
      "MemberId": "pcba-01.fan1_speed",
      "Name": "fan1_speed",
      "Reading": 3250,
      "ReadingUnits": "RPM",
      "Status": { "State": "Enabled", "Health": "OK" }
    }
  ]
}
```

> **computed（露点）与真实传感器同列表**：分类只依赖 `unit`，不区分数据来源。

### 4.4 Power — `GET /redfish/v1/Chassis/pcba-01/Power`

电压类（V/mV）进 `Voltages[]`；**R/W 寄存器在响应中带 `Actions` 链接**（可写提示）。

```json
{
  "@odata.id": "/redfish/v1/Chassis/pcba-01/Power",
  "@odata.type": "#Power.v1_7_3.Power",
  "Id": "Power",
  "Name": "Power",
  "Voltages@odata.count": 1,
  "Voltages": [
    {
      "@odata.id": "/redfish/v1/Chassis/pcba-01/Sensors/pcba-01.pump1_voltage",
      "MemberId": "pcba-01.pump1_voltage",
      "Name": "pump1_voltage",
      "ReadingVolts": 24.1,
      "Status": { "State": "Enabled", "Health": "OK" }
    }
  ]
}
```

### 4.5 Sensor（可写示例）— `GET /redfish/v1/Chassis/pcba-01/Sensors/pcba-01.fan1_duty`

```json
{
  "@odata.id": "/redfish/v1/Chassis/pcba-01/Sensors/pcba-01.fan1_duty",
  "@odata.type": "#Sensor.v1_7_0.Sensor",
  "Id": "pcba-01.fan1_duty",
  "Name": "fan1_duty",
  "Reading": 60,
  "ReadingUnits": "%",
  "ReadingType": "Percent",
  "Status": { "State": "Enabled", "Health": "OK" },
  "Timestamp": "2026-08-21T17:05:51.000Z",
  "Actions": {
    "#Sensor.SetReading": {
      "target": "/redfish/v1/Chassis/pcba-01/Sensors/pcba-01.fan1_duty"
    }
  },
  "Oem": {
    "Telemux": {
      "Access": "read_write",
      "SensorId": "pcba-01.fan1_duty",
      "Address": 10,
      "Function": "holding",
      "ValueType": "u16",
      "Formula": "v = v × 1 → %"
    }
  }
}
```

**字段规则**：
- `Reading`：metric 值；无 pipeline 取 raw；未采集 → `State=Disabled` 省略 Reading。
- **可写传感器**（`access=read_write`）带 `Actions` 链接；只读传感器无该字段。
- **bit 传感器**（bool）：`Reading` 为 0/1，`ReadingType` 按 unit 或取 `Presence`/`Level`
  （默认 `Other`），`Oem.Telemux.ValueType = "bool"`。
- **computed**：`Oem.Telemux` 带 `Computed: true` 与公式，无 raw 字段。

### 4.6 SensorCollection — `GET /redfish/v1/Chassis/pcba-01/Sensors`

全部寄存器 + computed：

```json
{
  "@odata.id": "/redfish/v1/Chassis/pcba-01/Sensors",
  "@odata.type": "#SensorCollection.SensorCollection",
  "Name": "Sensor Collection",
  "Members@odata.count": 7,
  "Members": [
    { "@odata.id": "/redfish/v1/Chassis/pcba-01/Sensors/pcba-01.env_temp" },
    { "@odata.id": "/redfish/v1/Chassis/pcba-01/Sensors/pcba-01.env_humidity" },
    { "@odata.id": "/redfish/v1/Chassis/pcba-01/Sensors/pcba-01.dew_point" },
    { "@odata.id": "/redfish/v1/Chassis/pcba-01/Sensors/pcba-01.fan1_speed" },
    { "@odata.id": "/redfish/v1/Chassis/pcba-01/Sensors/pcba-01.fan1_duty" },
    { "@odata.id": "/redfish/v1/Chassis/pcba-01/Sensors/pcba-01.liquid_level" },
    { "@odata.id": "/redfish/v1/Chassis/pcba-01/Sensors/pcba-01.leak" }
  ]
}
```

## 5. 写操作（PATCH）

对 `access=read_write` 的传感器：

```http
PATCH /redfish/v1/Chassis/pcba-01/Sensors/pcba-01.fan1_duty
Content-Type: application/json

{ "Reading": 60 }
```

| 场景 | 响应 |
|---|---|
| 寄存器存在且 `read_write` | `200` + 更新后的 Sensor 资源 |
| 寄存器只读 | `405 Method Not Allowed` |
| 寄存器不存在 | `404 Not Found` |
| PCBA 写入失败（设备离线/异常） | `503 Service Unavailable` + Oem.Telemux.Error |

**写入语义**：Redfish 写的是**物理值**（如 60%），网关按寄存器的 `scale/offset`
（若配置了 pipeline 换算）反算为寄存器原始值后写 PCBA；未配 pipeline 则直接写。
> 与 Modbus 从站"写原始值"的语义不同（Modbus 主站通常直接操作寄存器），
> 见 `docs/MODBUS_SERVER.md` §6。

## 6. 传感器分类与状态映射

### 6.1 分类（unit 启发式，不区分真实/computed）

| unit 包含 | 归入 | ReadingType |
|---|---|---|
| `°C` / `c` / `celsius` | `Thermal.Temps` | `Temperature` |
| `rpm` | `Thermal.Fans` | `Rotational` |
| `v` / `mv` | `Power.Voltages` | `Voltage` |
| `a` / `ma` | 仅 `Sensors` | `Current` |
| `%` | 仅 `Sensors` | `Percent` |
| `ph` | 仅 `Sensors` | `PH`（自定义） |
| 其他 / 无 unit | 仅 `Sensors` | `Other` |

### 6.2 状态映射（MetricStatus → Status）

| MetricStatus | Health | State |
|---|---|---|
| Normal | `OK` | `Enabled` |
| Warning | `Warning` | `Enabled` |
| Critical | `Critical` | `Enabled` |
| Unknown | `Unknown` | `Enabled` |
| 无数据 | `NA` | `Disabled` |

## 7. 写路径（协议层 → 采集层 → PCBA）

```
PATCH { "Reading": 60 }
  → 查配置：pcba-01.fan1_duty（holding, read_write, scale 相关 pipeline）
  → 反算原始值（如有 scale：raw = 60 / scale - offset）
  → 采集客户端 write_holding_register(address=10, raw)
  → 成功 200；失败 503
```

- 底层用 tokio-modbus `Writer` trait（`write_single_register` / `write_single_coil`）。
- 写后由下一轮采集回读，`Reading` 自动反映实际值。

## 8. 配置驱动与自动兼容

- 每次请求实时从 `ConfigHandle.read()` 构建资源树。
- 新增设备 → 新 Chassis；新增寄存器 → Sensors/Thermal/Power 自动出现；
  **新增 `[[computed]]` → 自动出现为传感器**；新增 R/W 寄存器 → 自动可 PATCH。
- 值从 `MetricStore` 读取，与 dashboard / Modbus 出口一致。

## 9. 验证方法

```bash
curl http://127.0.0.1:8000/redfish/v1
curl http://127.0.0.1:8000/redfish/v1/Chassis/pcba-01/Sensors/pcba-01.dew_point   # computed
curl -X PATCH http://127.0.0.1:8000/redfish/v1/Chassis/pcba-01/Sensors/pcba-01.fan1_duty \
     -H "Content-Type: application/json" -d '{"Reading": 60}'
# 新增寄存器后（重启或 dev 面板热更新）：GET 新 sensorId 自动存在
```

## 10. 实现要点（供实现阶段参考）

- axum 路由：`/redfish/v1` 前缀 + 动态段 `{device}` / `{sensorId}`；
  `PATCH` 需显式路由（`axum::routing::patch`）
- 分类 / 状态映射 / 反算为独立纯函数（单测：unit 匹配、Health 映射、scale 反算）
- 写操作通过 `SensorSource` 新接口（`write_holding_register` / `write_single_coil`）
- 实现位置：`src/protocol/redfish.rs`；端口/开关从 `[endpoints]` 读取

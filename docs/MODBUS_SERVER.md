# Modbus Server 接口文档（阶段 5）— Registers Table

> 状态：设计定稿，待实现。本网关作为 **Modbus-TCP 从站**，把 PCBA 的传感器指标
> 暴露给上层主站（SCADA / 机房监控），并**支持把主站的写入请求转发回 PCBA**
> （如风扇/水泵/比例阀占空比等 R/W 寄存器）。RTU 从站（串口）暂不实现，先做 TCP。

## 1. 设计原则

- **配置驱动 + 自动地址分配**：映射表由 `ConfigHandle.read()` 中的 `[[devices]]` +
  `[[devices.registers]]`（以及 `[[computed]]` 虚拟传感器）**自动生成**。
  **手动编辑配置文件新增寄存器后自动分配新地址，已有地址保持不变**。
- **值来自 MetricStore**：读操作返回计算后的 metric（无 pipeline 的寄存器返回 raw），
  与 dashboard / Redfish 一致；**写操作把主站值转发到 PCBA 对应寄存器**。
- **四区模型**：完整覆盖 Modbus 的 4 个地址空间（见 §3），与 PCBA 的
  R / R/W / bit 寄存器一一对应。
- 监听：`[endpoints] modbus_port`（默认 `1503`），unit id `modbus_unit_id`（默认 1）。
  > 采集端口 1502 是**客户端**连 PCBA；1503 是**从站**端口，二者不冲突。

## 2. 配置（`[endpoints]` 段，与 Redfish 共用）

```toml
[endpoints]
redfish_enabled = true
redfish_port = 8000
modbus_enabled = true
modbus_port = 1503
modbus_unit_id = 1
```

## 3. 寄存器模型与地址自动分配

### 3.1 配置侧：寄存器属性扩展

`[[devices.registers]]` 新增两个字段，与 PCBA 能力一一对应：

| 字段 | 取值 | 说明 |
|---|---|---|
| `access` | `"read"`（默认）\| `"read_write"` | R/W 属性；`read_write` 的寄存器才允许写入 |
| `function` | `"holding"` \| `"input"` \| `"coil"` \| `"discrete_input"` | 扩展到 4 个 Modbus 区 |
| `value_type` | `u16/i16/u32/i32/f32` 之外新增 **`bool`** | bit 型（配合 coil/discrete_input，如液位/漏液） |

```toml
# 可写示例：风扇占空比（保持寄存器，R/W）
[[devices.registers]]
name = "fan1_duty"
sensor_id = "pcba-01.fan1_duty"
function = "holding"
access = "read_write"      # ← 允许主站/Redfish 写入
address = 10
value_type = "u16"
unit = "%"

# bit 示例：漏液检测（离散输入，只读）
[[devices.registers]]
name = "leak_detect"
sensor_id = "pcba-01.leak"
function = "discrete_input"   # ← bit 区
value_type = "bool"           # ← bit 类型
address = 2
unit = "leak"
```

### 3.2 地址分配算法（确定性，4 个独立空间）

```
线圈区         coil function           （0x01 读 / 0x05 写）
离散输入区     discrete_input function （0x02 只读）
保持寄存器区   holding function        （0x03 读 / 0x06、0x10 写）
输入寄存器区   input function          （0x04 只读）

每个空间各自从 0 开始；分配顺序：config.devices（按顺序）→ 各设备 registers（按顺序）
每个寄存器占用 effective_count 个字（bool=1，u16/i16=1，u32/i32/f32=2）
```

**关键性质——追加稳定**：新寄存器追加到配置末尾 → 分配在已用地址之后，
**已有地址不变**（主站点位表无需修改）。中间插入会使后续地址顺移（建议一律追加）。

**`[[computed]]` 虚拟传感器**同样参与分配（追加在普通寄存器之后），自动获得地址，
主站可像读真实传感器一样读取计算值（露点/温差/压差等）。

## 4. Registers Table（示例）

> 示例基于扩展后的 PCBA 配置（风扇×1 占空比可写、温湿度 + 露点 computed、漏液 bit）。

### 4.1 保持寄存器区（0x03 / 0x06 / 0x10）

| 地址 | 长度 | 寄存器 | sensor_id | access | 类型 | 值示例 | 说明 |
|---|---|---|---|---|---|---|---|
| 0 | 1 | fan1_duty | pcba-01.fan1_duty | **R/W** | u16 | 60 | 占空比，可写 |
| 1 | 1 | pump1_duty | pcba-01.pump1_duty | **R/W** | u16 | 50 | 占空比，可写 |
| 2 | 1 | valve1_duty | pcba-01.valve1_duty | **R/W** | u16 | 35 | 占空比，可写 |

### 4.2 输入寄存器区（0x04）

| 地址 | 长度 | 寄存器 | sensor_id | access | 类型 | 值示例 | 说明 |
|---|---|---|---|---|---|---|---|
| 0 | 1 | fan1_speed | pcba-01.fan1_speed | R | u16 | 3250 | 转速 RPM |
| 1 | 1 | fan1_current | pcba-01.fan1_current | R | u16 | 120 | 电流 |
| 2 | 1 | pump1_temp | pcba-01.pump1_temp | R | u16 | 45 | 温度 °C |
| 3 | 1 | env_temp | pcba-01.env_temp | R | u16 | 27 | 温湿度-温度 |
| 4 | 1 | env_humidity | pcba-01.env_humidity | R | u16 | 60 | 温湿度-湿度 %RH |
| 5 | 2 | dew_point | pcba-01.dew_point | R | f32 | 18.6 | **computed**（露点） |
| 7 | 2 | diff_pressure | pcba-01.diff_pressure | R | f32 | 0.8 | **computed**（压差，示例） |

### 4.3 离散输入区（0x02，bit）

| 地址 | 长度 | 寄存器 | sensor_id | access | 类型 | 值示例 | 说明 |
|---|---|---|---|---|---|---|---|
| 0 | 1 | liquid_level | pcba-01.liquid_level | R | bool | 1 | 液位（1=有液） |
| 1 | 1 | leak_detect | pcba-01.leak | R | bool | 0 | 漏液（1=漏液） |       

### 4.4 线圈区（0x01 / 0x05）

| 地址 | 长度 | 寄存器 | sensor_id | access | 类型 | 说明 |
|---|---|---|---|---|---|---|
| 0 | 1 | pump1_enable | pcba-01.pump1_enable | **R/W** | bool | 水泵启停（示例） |

> 若 PCBA 有可写的 bit（如使能位），配 `function = "coil"` + `access = "read_write"`。

### 4.5 新增寄存器的效果（自动兼容）

在配置末尾追加任意寄存器（含 computed）→ 自动分配新地址，已有点位不变。
**四个区互不影响**：追加的 `holding` 寄存器只动保持区地址。

## 5. 值编码规则

| value_type | 编码 |
|---|---|
| u16 / i16 | `metric_value.round()` 取整，16 位截断 |
| u32 / i32 | `metric_value.round()` 取整，32 位 |
| f32 | IEEE754 单精度位模式，按 `word_order` 拆分两个字 |
| bool | 0 / 1 |

- 无 metric（无 pipeline）→ 编码 raw 值；未采集到数据 → 返回 `0xFFFF`（寄存器区）
  / `0`（bit 区）。
- `[[computed]]` 的值直接作为 metric 编码（无 raw）。

## 6. 功能码支持（读写双向）

| 功能码 | 名称 | 行为 |
|---|---|---|
| 0x01 | 读线圈 | ✅ 返回线圈区值（bool） |
| 0x02 | 读离散输入 | ✅ 返回离散输入区值（bool） |
| 0x03 | 读保持寄存器 | ✅ 返回保持区值 |
| 0x04 | 读输入寄存器 | ✅ 返回输入区值 |
| 0x05 | 写单线圈 | ✅ 仅 `coil` + `access=read_write`；否则异常 |
| 0x06 | 写单保持寄存器 | ✅ 仅 `holding` + `access=read_write`；否则异常 |
| 0x10 | 写多保持寄存器 | ✅ 同 0x06 规则 |
| 其他 | — | ❌ 异常 `IllegalFunction` |

**写路径**（协议层 → 采集层 → PCBA）：

```
主站 0x06 写保持寄存器 addr=0 值=60
  → 查映射表：pcba-01.fan1_duty（holding, read_write）
  → 校验 access：read_write ✓
  → 经采集客户端 Modbus 写保持寄存器（写 PCBA 原地址 address=10）
  → 成功返回应答；失败返回异常（ServerDeviceFailure）
```

- 写入的是**寄存器原始值**（与采集侧同尺度，如占空比直接写 60）；
  物理换算由主站负责（与 Redfish 写入"物理值"的语义不同，见 REDFISH.md §8）。
- 写只读寄存器 / 不存在的地址 → 异常 `IllegalDataAddress`。

## 7. 主站验证（mbpoll）

```bash
# 读输入寄存器：dew_point(addr 5, f32 两字)
mbpoll -m tcp -p 1503 -a 1 -t 4:float -r 5 -c 2 127.0.0.1
# 读离散输入：漏液(addr 1)
mbpoll -m tcp -p 1503 -a 1 -t 0 -r 1 -c 1 127.0.0.1
# 写保持寄存器：风扇占空比 = 60（addr 0, read_write）
mbpoll -m tcp -p 1503 -a 1 -t 4:hex -r 0 -c 1 -w 60 127.0.0.1   # 以 mbpoll 写语法为准
```

（`mbpoll` 语法按主站工具实际版本为准；核心是地址/长度/类型与映射表一致。）

## 8. 实现要点（供实现阶段参考）

- 用 `tokio-modbus` 的 server 骨架（`server::tcp::Server`，已在 `src/mock.rs` 验证用法）
- 请求处理时实时构建映射表（ConfigHandle）+ 查 MetricStore 取值
- **写请求**：解析目标寄存器 → 校验 access → 调用采集层写入接口
  （`SensorSource` 新增 `write_holding_register` / `write_single_coil` 方法，
  底层为 tokio-modbus `Writer` trait，需要采集客户端持有写上下文）
- 映射算法（4 区地址分配）为独立纯函数，单测地址稳定性与追加语义
- 实现位置：`src/protocol/modbus_server.rs`；端口/开关从 `[endpoints]` 读取
- `[[computed]]` 在 consumer 中计算后写入 MetricStore（见 IMPLEMENTATION.md 阶段 3 扩展）

## 9. 与 Redfish 的一致性

- 同一 `MetricStore` 取值、同一 `ConfigHandle` 生成点位 → Redfish 资源与 Modbus 点位天然一致。
- **写一致性**：主站经 Modbus 写入的值，同样反映在 Redfish 读取（写后采集回读）。
- 后续 SNMP 沿用同一数据源与写语义（`docs/IMPLEMENTATION.md` 阶段 5）。

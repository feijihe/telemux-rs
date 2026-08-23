# Telmux-rs 分步实现计划

> PCBA 板载传感器遥测多路网关（Rust）。采集层（Modbus）→ 处理管道 → 内存指标存储 → Redfish / SNMP / Modbus 三协议出口。

本文档把 README 的目标拆解为可独立验证的里程碑，并记录当前实现进度。

## 总体架构（来自 README）

```
[PCBA Sensors]
       ↓（原始 raw 数据读取）
┌─────────────────────┐
│  Acquisition Layer  │ 传感器抽象采集层，统一封装各类硬件读取接口
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
(HTTP)       (UDP)   (TCP/RTU)
```

## 分层约定（重要设计决策）

- **采集层只负责"读原始寄存器并解码"**：寄存器解码为数值（u16/i16/u32/i32/f32、字节序处理），**不做单位换算**。
- **单位换算、滤波、计算、阈值判断全部属于处理管道（阶段 3）**，配置在 `[[pipeline]]`，与 README 架构图一致。
- 各层之间通过明确的类型与 channel 通信：采集层输出 `RawSample`，管道输出 `Metric`，存储层按 `SensorId` 索引。

---

## 阶段 0 — 工程骨架 ✅（已完成）

- Rust edition 2024，`telemux` 包（lib + bin 双目标），release 优化（LTO、codegen-units=1、strip）。
- 模块布局：`config / domain / acquisition / logging / mock`（mock 仅开发构建编译）+ `main.rs`。
- 依赖：`tokio`、`tokio-modbus`（TCP+RTU 客户端/服务端）、`tokio-serial`、`serde`+`toml`、
  `tracing`+`tracing-subscriber`、`anyhow`/`thiserror`、`clap`、`async-trait`。
- **网络说明（已解决）**：早期受限沙箱（workspace-write）下 HTTPS 被禁（schannel 报
  `SEC_E_NO_CREDENTIALS`），crates.io 不可达，曾以离线缓存 + 手写 Modbus 协议层替代。
  根因是沙箱受限令牌无法访问证书库，**并非网络或代理问题**——以完整沙箱权限
  （danger-full-access）运行 cargo 即可正常联网（已实测 `cargo add`/`fetch`/`build` 通过）。
  网络恢复后已换回正式依赖：`tokio-modbus 0.17`、`tokio-serial 5.5`、`tracing-subscriber`、
  `async-trait`；手写协议层（`src/acquisition/modbus.rs`）已删除。
- **验收**：`cargo run` 启动、结构化日志、Ctrl+C 优雅退出。

## 阶段 1 — 领域模型 + 配置驱动 ✅（已完成）

| 步骤 | 内容 |
|---|---|
| 1.1 | 核心类型：`SensorId`、`RawSample`（原始寄存器值）、`Metric`（标准化指标）、`MetricStatus` |
| 1.2 | TOML 配置 schema：`[general]`、`[[devices]]`（TCP/RTU、从站地址、轮询间隔）、`[[devices.registers]]`（功能码、地址、长度、数值类型、字序） |
| 1.3 | serde 反序列化 + 自定义校验（sensor_id 全局唯一、寄存器地址冲突、transport 必填字段、unit_id 范围） |

**验收**：坏配置报错定位到具体字段；`config/example.toml` 可加载。
**说明**：`[[pipeline]]`、`[endpoints]`、`[[alerts]]` 配置段留待阶段 3/5 加入，当前解析器忽略未知字段以保证前向兼容。

### 阶段 1 扩展（阶段 5 需求，待实现）

为支持 PCBA 的 R/W 寄存器、bit 信号与二次计算指标，配置模型扩展：

| 项 | 变更 |
|---|---|
| `RegisterConfig.access` | 新增：`"read"`（默认）\| `"read_write"` —— 决定协议层能否写入 |
| `RegisterFunction` | `holding`/`input` 之外新增 **`coil`**（0x01 读/0x05 写）与 **`discrete_input`**（0x02 只读）—— 覆盖 Modbus 4 区 |
| `ValueType` | `u16/i16/u32/i32/f32` 之外新增 **`bool`**（bit 型，配合 coil/discrete_input，如液位/漏液） |
| `[[computed]]`（新段） | 虚拟传感器：由其他寄存器/计算指标经表达式二次计算（露点、温差、压差），见阶段 3 扩展 |

## 阶段 2 — 采集层（Acquisition Layer）✅（已完成）

| 步骤 | 内容 |
|---|---|
| 2.1 | `SensorSource` trait（`#[async_trait]`）：`read_samples() -> Vec<RawSample>`，统一封装硬件读取接口 |
| 2.2 | `ModbusTcpSource` / `ModbusRtuSource`（基于 `tokio-modbus` 0.17）：读保持/输入寄存器、超时、断线重连 |
| 2.3 | RTU 串口传输基于 `tokio-serial`（`open_native_async`），另提供 `from_stream` 支持内存流/RTU-over-TCP |
| 2.4 | 寄存器解码（纯函数，可单测）：u16/i16/u32/i32/f32，字序 big/little |
| 2.5 | 轮询调度器：每设备一个 tokio 任务，`interval` 驱动，失败指数退避，输出进 `mpsc` channel |
| 2.6 | Mock PCBA 从站（`src/mock.rs` + `examples/mock_pcba.rs`，基于 `tokio-modbus` server）+ TCP/RTU 集成测试。**仅开发构建编译**（`cfg(any(debug_assertions, feature="dev-dashboard"))`），release 生产排除；相关测试/示例同样门控 |

**验收**：`cargo test` 通过（含对接模拟从站的 TCP 集成测试、内存流上的 RTU 帧测试）；
`cargo run` 持续输出采样日志。
**说明**：RTU 真实串口路径依赖硬件，自动化测试覆盖 RTU 帧协议（duplex + CRC16 校验），
串口打开路径为编译级覆盖。

### 阶段 2 扩展（阶段 5 需求，待实现）

- **读 bit**：`coil` → `read_coils`，`discrete_input` → `read_discrete_inputs`（tokio-modbus
  `Reader` 已支持）；`bool` 解码为 0.0/1.0 的 `RawSample`
- **写路径**：`SensorSource` 新增 `write_holding_register(sensor_id, value)` /
  `write_single_coil(sensor_id, value)`（底层 `Writer` trait）；写前按映射表校验 `access`

## Dev Dashboard（开发调试面板）✅（三期完成）

独立于主线阶段的功能（详见 `docs/DEV_DASHBOARD.md`）：

- 本地 Web 面板（`127.0.0.1:8080`）：表格展示寄存器配置 + 原始值 + 计算指标（状态着色），WebSocket 实时刷新（~1Hz 快照）
- **公式可视化**：每行可点击展开计算链路（`describe_stage`/`describe_pipeline` 自动生成，如 `v = v × 0.1 → °C → avg(最近 5 个值) → 状态: <5 critical / ...`）
- **动态创建寄存器**（第三期）：`POST /api/registers` 热添加 —— 服务端权威校验（sensor_id 唯一 / 地址重叠 / pipeline 合法）→ 写回 TOML → ≤1 个 poll 周期自动生效（无需重启）；前端表单 + 错误展示
- **开发/生产隔离**：`cfg(any(debug_assertions, feature = "dev-dashboard"))`
  - `ConfigHandle`（`src/config_handle.rs`）：dev 构建内部 `Arc<RwLock<Config>>` 可热更新；**release 构建内部 `Arc<Config>` 纯只读，`update()`/`save()` 编译期不存在**（已验证 release 二进制无任何 dashboard/动态配置痕迹）
  - 采集层 `SensorSource::read_samples(registers)` 每轮从 ConfigHandle 热读寄存器列表；`PipelinesCache` 按配置 revision 重建
- 依赖 axum/serde_json/futures-util 常驻（cargo 无法按 profile 启用 optional 依赖；axum 亦为阶段 5 Redfish 所需）

## 阶段 3 — 处理管道（Processing Pipeline）✅（已完成）

| 步骤 | 内容 |
|---|---|
| 3.1 | `Stage` trait（`process(&mut SampleContext)`）+ `Pipeline`（按序执行，`RawSample → Metric`）。注意：Stage 刻意不要求 `Send`（meval 表达式内部持 `Rc`），管道在主线程 consumer 中单线程运行 |
| 3.2 | 内置阶段（`src/pipeline/stages.rs`）：`scale`（线性换算+改单位）、`sliding_average`/`median`（滑动滤波）、`math`（meval 表达式，变量 `v`）、`threshold`（Normal/Warning/Critical，critical 优先）、`aggregate`（窗口 min/max/avg） |
| 3.3 | 错误降级：管道失败仅记 warn 日志并跳过该 sensor 的 metric 更新（store 保留上一次值），不影响其他传感器 |
| 3.4 | `[[pipeline]]` 配置段（`StageConfig` 枚举，`type` 标签驱动）+ 校验（sensor_id 必须存在且唯一、窗口范围、阈值边界、表达式可解析） |

**验收**：各 stage 单测 + 管道集成测试通过；`config/example.toml` 演示 5 条管道（换算/滤波/阈值/聚合）。
**已知坑**：TOML 键 `[[pipeline]]`（单数）与 Rust 字段 `pipelines` 不匹配会导致静默为空 —— 已用 `#[serde(rename = "pipeline")]` 修复并补测试。

### 阶段 3 扩展：`[[computed]]` 虚拟传感器（阶段 5 需求，待实现）

由多个读取值/计算指标二次计算出新指标（露点、温差、压差），**不直接读寄存器**：

```toml
[[computed]]
sensor_id = "pcba-01.dew_point"      # 全局唯一，自动成为协议层传感器
name = "dew_point"
unit = "°C"
# 输入变量：键为表达式变量名，值为引用的 sensor_id
inputs = { t = "pcba-01.env_temp", h = "pcba-01.env_humidity" }
# 纯数学表达式（meval 多变量，内置 ln/exp/...）：Magnus 露点公式
expression = "243.5 * (ln(h/100) + 17.67*t/(243.5+t)) / (17.67 - ln(h/100) - 17.67*t/(243.5+t))"
```

- **处理时机**：采集 consumer 每批样本后，对每个 computed 求值（输入取 MetricStore 最新值；
  输入未就绪则跳过），结果以"无 raw 的 metric"写入 MetricStore
- **自动出现**：computed 与真实传感器同等出现在 dashboard / Redfish / Modbus 点位表中
  （分类只看 unit，不区分来源）
- **校验**：sensor_id 全局唯一且不与寄存器重复；inputs 引用的 sensor_id 必须存在；
  表达式可解析且变量名 ⊆ inputs 键；表达式可引用其他 computed（需按拓扑顺序求值，防环）
- 存储扩展：`SensorState.raw` 改为 `Option<RawSample>`（computed 无 raw），
  所有读取方（dashboard/redfish/modbus）处理 None

## 阶段 4 — 指标存储（Metric Store）✅（已完成）

| 步骤 | 内容 |
|---|---|
| 4.1 | `MetricStore`（`src/store.rs`）：`RwLock<HashMap<SensorId, SensorState>>`，`SensorState` 同时保留**最近 RawSample + 最近 Metric**（dashboard 面板约束） |
| 4.2 | 快照 API（`snapshot()`/`get()`，供协议层读取）+ 变更通知（`watch` revision 通道，供告警/Trap 订阅） |
| 4.3 | 采集 consumer 重构：`update_batch_raw` → 逐 sensor 跑 pipeline → `update_metric`（主线程 select 循环，替代 tokio::spawn —— 管道非 Send） |

**验收**：并发读写单测通过；dashboard 快照展示 raw + metric 双列。
**Dev Dashboard 第二期**：`snapshot.rs` 已从 `MetricStore` 读取并填充 `metric` 字段（含状态着色数据），
前端无需改动 —— 见 `docs/DEV_DASHBOARD.md`。

## 阶段 5 — 输出协议（Redfish + Modbus Server）✅（已完成；SNMP 暂缓）

**范围调整**：SNMP 因 Rust 生态不成熟**暂不实现**（本阶段交付 Redfish + Modbus Server）。

**设计文档**：`docs/REDFISH.md`、`docs/MODBUS_SERVER.md`（实现与文档一致）。

**5.1 Redfish（axum，端口 `[endpoints] redfish_port` 默认 8000）**
- 资源树：`/redfish/v1` → `/Chassis/{device}` → `/Thermal`、`/Power`、`/Sensors/{sensorId}`，Redfish Schema 风格 JSON
- **配置驱动**：每次请求从 `ConfigHandle` 构建资源 → 新增寄存器/computed 自动出现（已验证）
- **读写双向**：`PATCH /Sensors/{id}` 写 `access=read_write` 寄存器（物理值按 scale 反算 → 写 PCBA）；只读返回 405
- `unit` 启发式分类（温度/转速/电压/电流）；`MetricStatus → Health` 映射；computed 与真实传感器同等暴露

**5.2 Modbus Server（tokio-modbus TCP 从站，端口 `modbus_port` 默认 1503）**
- **四区模型 + 自动地址分配**：线圈/离散输入/保持/输入各自从 0 起按配置顺序分配；computed 进输入区（f32）；**追加式新增地址稳定**
- **读写双向**：0x01-0x04 读（返回 metric，无 pipeline 用 raw，无数据 0xFFFF/false）；0x05/0x06/0x10 写（仅 `access=read_write` 且单字，经 WriteBroker 转发到 PCBA）；写只读/多字 → 异常
- bit 型（bool + coil/discrete_input）支持液位/漏液等状态信号

**5.3 SNMP（暂缓）**：待前两者稳定后评估 `snmp_rust_agent` / `async-snmp`，OID 映射沿用同一数据源。

**配套改造**：`RegisterConfig.access`（read/read_write）、`RegisterFunction` 增 coil/discrete_input、
`ValueType` 增 bool、`[[computed]]` 虚拟传感器（meval 多变量表达式 + 环检测）、`[endpoints]` 配置段、
采集层写接口（`SensorSource::write_holding_register`/`write_single_coil`）、`WriteBroker` 写通道
（协议层 → 设备轮询任务 → PCBA）、`SensorState.raw` 改 Option（computed 无 raw）。

**验收（已端到端验证）**：`curl` Redfish 资源树 + PATCH 写（200/405）；
Rust 客户端读 Modbus 输入/离散输入、写保持（OK）、写只读（IllegalFunction）；
dev 面板新增寄存器后 Redfish 自动出现（count+1）、Modbus 自动分配地址。

**5.3 SNMP（暂缓）**
- 待 Redfish/Modbus 稳定后评估 `snmp_rust_agent` / `async-snmp`，或自实现最小 v1/v2c agent；OID 映射沿用同一数据源。

**验收**：`curl` Redfish 资源树、`mbpoll` 读 Modbus 从站，数值与模拟 PCBA 一致；新增寄存器后两协议自动兼容。

## 阶段 6 — 可观测性与运维 ✅（已完成）

**文档**：`docs/OPERATIONS.md`（用法）、`deploy/telemux.service`（systemd unit）。

| 步骤 | 内容 |
|---|---|
| 6.1 | tracing 结构化日志 + **滚动文件日志**（`tracing-appender` 按日轮转 + `log_max_files` 保留；stdout 按 `--log-level`/`RUST_LOG` 过滤，文件层记录 TRACE 全量便于排障；退出时 guard flush） |
| 6.2 | **优雅停机**：SIGINT + SIGTERM（Unix）/ Ctrl+C（Windows）/ 服务 Stop 事件 → 主循环退出 → 通知协议+采集任务停止 → 等待任务 → flush 日志。核心逻辑重构为 `src/app.rs::run_gateway(cli, signal_rx)`，信号由外部注入 |
| 6.3 | **守护进程**：Windows Service（`windows-service` crate，`--install-service`/`--uninstall-service`/`--service`，服务名 telemux，cfg(windows)）+ Linux systemd unit（`deploy/telemux.service`，含安全加固与内存限制） |
| 6.4 | **健康/就绪端点**（`src/health.rs`，默认 8081）：`/healthz` 存活（恒 200）、`/readyz` 就绪（至少一台设备 2 个轮询间隔内有数据才 200，附设备连接状态与协议端点状态） |

**配套改造**：
- 配置：`[general] log_dir` / `log_max_files`（默认 7）；`[endpoints] health_enabled` / `health_port`（默认 8081）
- 模块：`src/cli.rs`（Cli 参数）、`src/app.rs`（run_gateway 核心）、`src/health.rs`、`src/service.rs`（Windows）
- `main.rs` 瘦身为入口：Windows 服务模式分发 + 前台模式信号转发任务
- 依赖：`tracing-appender 0.2`；`[target.'cfg(windows)'.dependencies] windows-service 0.7`；dev-deps `tower` + `http-body-util`（健康端点路由测试）

**验收（已端到端验证）**：mock + 网关（`config/test-p6.toml`）运行：
`/healthz` 200、`/readyz` 200 且 7 传感器全部有数据、日志滚动文件生成并含 debug 样本；
全部 62 测试通过（新增 health 3 + logging 文件写入 1），clippy 干净，dev/release/release+dashboard 三种构建均无警告。

## 阶段 7 — 测试与验证 ✅（已完成）

| 项 | 内容 |
|---|---|
| 单元测试 | config 解析（新增 computed 环检测/未知输入/链式引用）、管道阶段、寄存器/表映射、Redfish 处理器错误路径（404/400/503）、Modbus 服务层错误路径（越界读 → IllegalDataAddress、占位符、写只读 → IllegalFunction）、store 有界性（1 万次写入容量不变） |
| 集成测试 | **全链路一致性**（`tests/full_pipeline.rs`，进程内 mock → 采集解码 → 管道 → 存储 → 协议视图/编码互逆，数值逐层断言）；RTU 帧测试；TCP 断线重连 |
| 稳定性 | **断线重连演练**（`tests/stability.rs`：连续轮询 → 杀 mock → 换端口重启 + 热更新配置 → 一个轮询周期内自动恢复，存储有界）；内存验证（store 每传感器只保留最新值，容量恒定，贴合低资源开销目标） |
| 修复 | computed 校验两阶段化（允许任意顺序引用，环由 DFS 检测——此前前向约束使环检测不可达）；`ComputedEngine` 拓扑排序（computed→computed 链配置乱序也能一轮收敛） |

**验收**：77 个测试全绿（68 lib + 9 集成），clippy 零警告，dev/release 构建均通过；`cargo test --release` 通过。

## 阶段 8 — 打包发布 ✅（已完成）

**文档**：`docs/DEPLOYMENT.md`（部署指南）。

| 项 | 内容 |
|---|---|
| Release 构建 | `lto` + `codegen-units=1` + `strip` + `panic=abort`；Windows telemux.exe ≈ 2.7 MB；**字符串级验证** release 二进制不含 dashboard/mock/动态配置（仅 CLI 参数名含 `--dashboard-port` 等字样，属预期） |
| 安装脚本 | `deploy/install.sh`（Linux：构建→安装→配置→systemd 启用）、`deploy/install.bat`（Windows：构建→复制→交互式服务安装） |
| 平台交付 | Linux systemd（`deploy/telemux.service`）、Windows Service（`--install-service`）；双平台配置/文档齐备 |
| 冒烟测试 | **release 二进制端到端**：mock + `telemux.exe --config test-p6.toml` 运行 → healthz/readyz/Redfish 全 200、滚动日志持续写入 |

**验收**：`cargo build --release` 干净、release 二进制三端点冒烟通过、安装脚本与部署文档就绪。

## 阶段 8 扩展 — CDU 仿真（Simulation）✅（已完成）

**文档**：`docs/SIMULATION.md`、示例配置 `config/cdu.toml`。

把"假 mock"升级为**配置驱动的物理仿真数据源**，用于整机（CDU）开发/演示/验收：

- **`[sim]` 配置段**：`[[sim.controls]]`（泵/阀/风扇 duty，`writable` 可映射为 Modbus 保持寄存器）+ `[[sim.sensors]]`（物理测量点，`formula` meval 稳态表达式 + `inputs` 显式依赖映射）
- **`Transport::Sim`**：设备声明 `transport = "sim"` 即接入仿真数据源（`src/simulation.rs`，实现 `SensorSource`），复用采集/管道/协议全链路；sim 设备无需寄存器（自动产出全部 sim 传感器）
- **物理建模原则**：物理点（Pn/Tn/Fn 独立测量位置）在 `[sim]`，**压差/温差等派生量用 `[[computed]]`**（真实系统无 dp 探头，是两路压力差分）——生产切换时 computed 原样保留
- **变量解析**：`inputs` 映射（与 computed 一致）→ 控制变量名 → 内置时间 `t` → 传感器短名
- **写驱动仿真**：可写控制变量 → Modbus 保持寄存器（u16 0-100）→ WriteBroker → SimSource 更新 duty → 下一轮采集生效
- **协议映射**：仿真传感器与 computed 均进输入寄存器（f32），控制变量进保持区（u16 R/W）
- 示例 `config/cdu.toml`：29 传感器（一次/二次侧 P/T/F、泵进出口压力、水箱液位/PH、环境/泄漏）+ 4 控制变量 + 4 派生量（泵压差×2、温差×2）

**验收（已端到端验证）**：`cargo run -- --config config/cdu.toml` 无硬件运行，
Redfish Sensors 33+、readyz `sensors_total:30`；
**Modbus 写 pump1_duty=80 → f2_flow 109→142 L/min**（10+(80+40)*1.1），因果链成立；
`tests/cdu_sim.rs` 集成测试通过；85 测试全绿、clippy 干净。

---

## 关键依赖速查

| 用途 | crate |
|---|---|
| 异步运行时 | `tokio` |
| Modbus 客户端/服务端 | `tokio-modbus` 0.17（TCP + RTU，含 server 骨架用于 mock） |
| RTU 串口 | `tokio-serial` 5.5 |
| Redfish HTTP | `axum` + `serde_json`（阶段 5） |
| SNMP agent | `snmp_rust_agent` / `async-snmp`（阶段 5，备选自实现） |
| 配置 | `serde` + `toml` |
| 日志 | `tracing` + `tracing-subscriber`（env-filter + tracing-log 桥接） |
| 表达式计算 | `fasteval` / `meval`（阶段 3） |
| CLI | `clap` |

## 主要风险点

1. **SNMP 是最大不确定性**——Rust 生态 agent 库较新，阶段 5 前先做最小 spike。
2. **RTU 串口**在 Windows 有权限/兼容差异，先 TCP 后 RTU。
3. **Redfish 范围要克制**——按"只读传感器资源"最小子集实现，不追求完整规范。

## 进度

- [x] 阶段 0 工程骨架
- [x] 阶段 1 领域模型 + 配置
- [x] 阶段 2 采集层
- [x] 阶段 3 处理管道
- [x] 阶段 4 指标存储
- [x] 阶段 5 输出协议（Redfish + Modbus Server；SNMP 暂缓）
- [x] 阶段 6 可观测性与运维
- [x] 阶段 7 测试与验证
- [x] 阶段 8 打包发布
- [x] Dev Dashboard（两期均完成）

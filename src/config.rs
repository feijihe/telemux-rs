//! TOML 驱动的配置：设备、寄存器、轮询行为。
//!
//! 管道/端点/告警段在后续阶段添加；未知字段会被忽略以保证向前兼容。

use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

/// 顶层配置，从 TOML 反序列化。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub general: GeneralConfig,
    #[serde(default)]
    pub devices: Vec<DeviceConfig>,
    /// 每传感器处理管道（阶段 3）。TOML 键：`[[pipeline]]`。
    #[serde(default, rename = "pipeline")]
    pub pipelines: Vec<PipelineConfig>,
    /// 由其他传感器计算出的虚拟传感器（阶段 5）。TOML 键：`[[computed]]`。
    #[serde(default, rename = "computed")]
    pub computed: Vec<ComputedConfig>,
    /// 协议端点（阶段 5）：Redfish / Modbus 服务器。
    #[serde(default)]
    pub endpoints: EndpointsConfig,
    /// CDU 仿真建模（阶段 8扩展）：配置一台虚拟 CDU 的传感器布局与
    /// 物理耦合关系，产出模拟数据（无真实硬件）。见 `docs/SIMULATION.md`。
    #[serde(default)]
    pub sim: SimConfig,
}

/// CDU 仿真建模配置（`[sim]`）。
///
/// 用**稳态代数模型**表达传感器间的物理因果（如泵 duty→流量/压差、
/// 比例阀 duty→一次侧流量→二次侧温度），供开发/演示/验收使用。
/// 生产环境将设备 `transport` 改回 `tcp`/`rtu` 即接真实 CDU。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SimConfig {
    /// 控制变量（泵/阀/风扇 duty 等），可被协议层写入以驱动仿真。
    #[serde(default)]
    pub controls: Vec<SimControl>,
    /// 仿真传感器：每个通过 formula 表达式从控制变量与其他传感器求值。
    #[serde(default)]
    pub sensors: Vec<SimSensor>,
}

/// 仿真控制变量（如 pump1_duty，0-100）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimControl {
    /// 控制变量名，供传感器 formula 引用（如 "pump1_duty"）。
    pub name: String,
    /// 初始值。
    #[serde(default)]
    pub initial: f64,
    /// 单位（信息性）。
    #[serde(default)]
    pub unit: Option<String>,
    /// 是否可写（协议层 PATCH/Modbus 可改变 duty）。
    #[serde(default)]
    pub writable: bool,
}

/// 仿真传感器：由稳态表达式求值，作为原始样本产出。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimSensor {
    /// 全网关唯一指标键（如 "cdu.pump1.dp"）。
    pub sensor_id: String,
    /// 显示名称。
    pub name: String,
    /// 传感器类型：pressure/temperature/flow/level/ph/leak/...
    /// 用于协议层分类（Redfish ReadingType）。
    #[serde(default)]
    pub kind: String,
    /// 物理单位。
    #[serde(default)]
    pub unit: Option<String>,
    /// 稳态求值表达式（meval 语法）。
    ///
    /// 变量解析规则（优先级从高到低）：
    /// 1. `inputs` 映射的键 → 引用的仿真传感器（或控制变量）；
    /// 2. 控制变量名（如 `pump1_duty`）；
    /// 3. 内置时间 `t`（自启动秒数，用于模拟缓慢波动）；
    /// 4. 其他仿真传感器的短名（sensor_id 末段，如 `cdu.pri.p1` → `p1`）。
    ///
    /// 注：meval 变量名不能含 `.`，故跨传感器引用推荐用 `inputs` 显式映射
    /// （与 `[[computed]]` 一致），短名仅在本机无歧义时可用。
    pub formula: String,
    /// 变量名 → 引用的传感器/控制变量（可选，显式依赖映射）。
    #[serde(default)]
    pub inputs: std::collections::HashMap<String, String>,
}

/// 协议端点设置（阶段 5 + 阶段 6）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct EndpointsConfig {
    #[serde(default = "default_true")]
    pub redfish_enabled: bool,
    #[serde(default = "default_redfish_port")]
    pub redfish_port: u16,
    #[serde(default = "default_true")]
    pub modbus_enabled: bool,
    #[serde(default = "default_modbus_port")]
    pub modbus_port: u16,
    #[serde(default = "default_modbus_unit_id")]
    pub modbus_unit_id: u8,
    /// 健康/就绪 HTTP 端点（阶段 6.4）。
    #[serde(default = "default_true")]
    pub health_enabled: bool,
    #[serde(default = "default_health_port")]
    pub health_port: u16,
}

impl Default for EndpointsConfig {
    fn default() -> Self {
        Self {
            redfish_enabled: default_true(),
            redfish_port: default_redfish_port(),
            modbus_enabled: default_true(),
            modbus_port: default_modbus_port(),
            modbus_unit_id: default_modbus_unit_id(),
            health_enabled: default_true(),
            health_port: default_health_port(),
        }
    }
}

fn default_true() -> bool {
    true
}
fn default_redfish_port() -> u16 {
    8000
}
fn default_modbus_port() -> u16 {
    1503
}
fn default_modbus_unit_id() -> u8 {
    1
}
fn default_health_port() -> u16 {
    8081
}

impl Config {
    /// 加载并校验配置文件。
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("read config file `{}`", path.display()))?;
        let cfg: Config = toml::from_str(&text)
            .with_context(|| format!("parse config file `{}`", path.display()))?;
        cfg.validate()
            .with_context(|| format!("invalid config file `{}`", path.display()))?;
        Ok(cfg)
    }

    /// 超出 TOML 类型检查的语义校验。
    pub fn validate(&self) -> Result<()> {
        let mut seen_devices = std::collections::HashSet::new();
        let mut seen_sensors = std::collections::HashSet::new();
        let mut known_sensors: std::collections::HashSet<&str> = std::collections::HashSet::new();

        if self.devices.is_empty() {
            bail!("at least one [[devices]] entry is required");
        }
        if self.devices.len() > 64 {
            bail!("too many devices ({}), max 64", self.devices.len());
        }

        for device in &self.devices {
            if !seen_devices.insert(device.name.as_str()) {
                bail!("duplicate device name `{}`", device.name);
            }
            if device.unit_id > 247 {
                bail!(
                    "device `{}`: unit_id {} out of range (0..=247)",
                    device.name,
                    device.unit_id
                );
            }
            match device.transport {
                Transport::Tcp => {
                    if device.host.is_empty() {
                        bail!("device `{}`: tcp transport requires `host`", device.name);
                    }
                    if device.port == 0 {
                        bail!("device `{}`: tcp transport requires a valid `port`", device.name);
                    }
                }
                Transport::Rtu => {
                    if device.serial_port.is_none() {
                        bail!(
                            "device `{}`: rtu transport requires `serial_port` (e.g. \"COM3\")",
                            device.name
                        );
                    }
                    if device.baud_rate.is_none() {
                        bail!("device `{}`: rtu transport requires `baud_rate`", device.name);
                    }
                }
                Transport::Sim => {
                    // 仿真设备：不要求 host/port/串口，但必须有 [sim] 配置。
                    if self.sim.controls.is_empty() {
                        bail!(
                            "device `{}`: sim transport requires `[sim]` controls",
                            device.name
                        );
                    }
                }
            }
            if device.poll_interval_ms == 0 {
                bail!("device `{}`: poll_interval_ms must be > 0", device.name);
            }
            if device.timeout_ms == 0 {
                bail!("device `{}`: timeout_ms must be > 0", device.name);
            }
            if device.reconnect_initial_ms > device.reconnect_max_ms {
                bail!(
                    "device `{}`: reconnect_initial_ms ({}) must be <= reconnect_max_ms ({})",
                    device.name,
                    device.reconnect_initial_ms,
                    device.reconnect_max_ms
                );
            }
            if device.registers.is_empty() && device.transport != Transport::Sim {
                bail!("device `{}`: at least one register is required", device.name);
            }
            if device.registers.len() > 256 {
                bail!("device `{}`: too many registers ({}), max 256", device.name, device.registers.len());
            }

            // 寄存器：名称唯一、sensor_id 唯一、地址范围不重叠。
            let mut seen_reg_names = std::collections::HashSet::new();
            let mut ranges: Vec<(RegisterFunction, u16, u16)> = Vec::new();
            for reg in &device.registers {
                if !seen_reg_names.insert(reg.name.as_str()) {
                    bail!(
                        "device `{}`: duplicate register name `{}`",
                        device.name,
                        reg.name
                    );
                }
                if !seen_sensors.insert(reg.sensor_id.as_str()) {
                    bail!("duplicate sensor_id `{}` (must be unique gateway-wide)", reg.sensor_id);
                }
                known_sensors.insert(reg.sensor_id.as_str());
                // bit 区（coil/discrete_input）必须用 bool；非 bit 区不允许 bool
                // 之外的类型出现在 bit 区
                if reg.function.is_bit() && reg.value_type != ValueType::Bool {
                    bail!(
                        "device `{}` register `{}`: function {:?} requires value_type = \"bool\"",
                        device.name, reg.name, reg.function
                    );
                }
                // read_write 仅允许在可写区（holding/coil）
                if reg.access == Access::ReadWrite && !reg.function.is_writable() {
                    bail!(
                        "device `{}` register `{}`: access = \"read_write\" is only allowed for holding/coil registers",
                        device.name, reg.name
                    );
                }
                let width = reg.value_type.register_count();
                let eff_count = reg.effective_count();
                if eff_count != width {
                    bail!(
                        "device `{}` register `{}`: count {} does not match value_type {:?} (expected {})",
                        device.name, reg.name, eff_count, reg.value_type, width
                    );
                }
                let end = reg.address.checked_add(width).context("register address overflow")?;
                for &(other_func, other_addr, other_end) in &ranges {
                    if other_func == reg.function && reg.address < other_end && other_addr < end {
                        bail!(
                            "device `{}`: register `{}` range [{}, {}) overlaps `{}` range [{}, {})",
                            device.name,
                            reg.name,
                            reg.address,
                            end,
                            device.registers
                                .iter()
                                .find(|r| r.function == other_func && r.address == other_addr)
                                .map(|r| r.name.as_str())
                                .unwrap_or("?"),
                            other_addr,
                            other_end
                        );
                    }
                }
                ranges.push((reg.function, reg.address, end));
            }
        }

        // 仿真传感器也可作为管道输入（无寄存器，采集层直接产出样本）。
        for s in &self.sim.sensors {
            known_sensors.insert(s.sensor_id.as_str());
        }

        // 管道：sensor_id 必须引用已知寄存器（或仿真传感器）、必须唯一，
        // 且包含有效的阶段参数。
        let mut seen_pipelines: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for pipe in &self.pipelines {
            if !known_sensors.contains(pipe.sensor_id.as_str()) {
                bail!(
                    "pipeline sensor_id `{}` does not match any device register",
                    pipe.sensor_id
                );
            }
            if !seen_pipelines.insert(pipe.sensor_id.as_str()) {
                bail!(
                    "duplicate pipeline for sensor_id `{}` (only one pipeline per sensor)",
                    pipe.sensor_id
                );
            }
            if pipe.stages.is_empty() {
                bail!("pipeline for `{}`: at least one stage is required", pipe.sensor_id);
            }
            if pipe.stages.len() > 16 {
                bail!("pipeline for `{}`: too many stages ({}), max 16", pipe.sensor_id, pipe.stages.len());
            }
            for stage in &pipe.stages {
                crate::pipeline::validate_stage(stage)
                    .map_err(|e| anyhow::anyhow!("pipeline for `{}`: {e}", pipe.sensor_id))?;
            }
        }

        // 计算（虚拟传感器）：id 唯一、引用的输入存在、
        // 表达式可解析且其变量是输入的子集、无环。
        // 两阶段：先收集全部 computed id（允许任意顺序引用，
        // 环由下方 DFS 检测），再逐项校验输入与表达式。
        let mut computed_ids: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for c in &self.computed {
            if !seen_sensors.insert(c.sensor_id.as_str()) {
                bail!(
                    "duplicate sensor_id `{}` (computed must not collide with registers)",
                    c.sensor_id
                );
            }
            if !computed_ids.insert(c.sensor_id.as_str()) {
                bail!("duplicate computed sensor_id `{}`", c.sensor_id);
            }
            if c.name.is_empty() {
                bail!("computed `{}`: name is required", c.sensor_id);
            }
            if c.inputs.is_empty() {
                bail!("computed `{}`: at least one input is required", c.sensor_id);
            }
        }
        for c in &self.computed {
            // 输入必须引用已存在的传感器（寄存器或任意 computed）
            for (var, src) in &c.inputs {
                if !known_sensors.contains(src.as_str()) && !computed_ids.contains(src.as_str()) {
                    bail!(
                        "computed `{}`: input `{}` references unknown sensor_id `{}`",
                        c.sensor_id,
                        var,
                        src
                    );
                }
            }
            // 表达式必须可解析，且其变量必须已在 inputs 中定义
            // （bindn 本身会拒绝未知变量）
            use std::str::FromStr;
            let expr = meval::Expr::from_str(&c.expression).map_err(|e| {
                anyhow::anyhow!(
                    "computed `{}`: invalid expression `{}`: {e}",
                    c.sensor_id,
                    c.expression
                )
            })?;
            let keys: Vec<&str> = c.inputs.keys().map(String::as_str).collect();
            // bindn 校验表达式的每个变量都已绑定到某个输入。
            let _bound = expr
                .clone()
                .bindn(&keys)
                .map_err(|e| anyhow::anyhow!("computed `{}`: {e}", c.sensor_id))?;
        }
        // 对 computed -> computed 引用做环检测。
        let mut state: std::collections::HashMap<&str, u8> = std::collections::HashMap::new();
        fn visit<'a>(
            id: &'a str,
            computed: &'a [ComputedConfig],
            state: &mut std::collections::HashMap<&'a str, u8>,
        ) -> Result<(), String> {
            match state.get(id) {
                Some(&1) => return Err(format!("computed dependency cycle at `{id}`")),
                Some(&2) => return Ok(()),
                _ => {}
            }
            state.insert(id, 1);
            if let Some(c) = computed.iter().find(|c| c.sensor_id == id) {
                for src in c.inputs.values() {
                    if computed.iter().any(|c| c.sensor_id == *src) {
                        visit(src, computed, state)?;
                    }
                }
            }
            state.insert(id, 2);
            Ok(())
        }
        for c in &self.computed {
            visit(&c.sensor_id, &self.computed, &mut state)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
        }

        // 仿真（[sim]）：控制变量名唯一、sensor_id 唯一、表达式可解析。
        let mut sim_controls: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for c in &self.sim.controls {
            if !sim_controls.insert(c.name.as_str()) {
                bail!("duplicate sim control `{}`", c.name);
            }
        }
        let mut sim_ids: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for s in &self.sim.sensors {
            if !seen_sensors.insert(s.sensor_id.as_str()) {
                bail!(
                    "duplicate sensor_id `{}` (sim must not collide with registers/computed)",
                    s.sensor_id
                );
            }
            if !sim_ids.insert(s.sensor_id.as_str()) {
                bail!("duplicate sim sensor_id `{}`", s.sensor_id);
            }
            if s.name.is_empty() {
                bail!("sim sensor `{}`: name is required", s.sensor_id);
            }
            // 表达式必须可解析。变量引用正确性（∈ 控制变量 ∪ 传感器）由
            // SimSource 求值时用 bindn 校验 —— meval 无 variables()，无法在
            // 配置期枚举变量，故延迟到求值并降级为 warn。
            use std::str::FromStr;
            meval::Expr::from_str(&s.formula).map_err(|e| {
                anyhow::anyhow!(
                    "sim sensor `{}`: invalid formula `{}`: {e}",
                    s.sensor_id,
                    s.formula
                )
            })?;
            // inputs 映射的目标必须是控制变量名或其它仿真传感器 id。
            for (var, target) in &s.inputs {
                if !sim_controls.contains(target.as_str()) && !sim_ids.contains(target.as_str()) {
                    bail!(
                        "sim sensor `{}`: input `{var}` references unknown target `{target}`",
                        s.sensor_id
                    );
                }
            }
        }
        Ok(())
    }

    /// 向已有设备添加一个寄存器（运行时热添加，开发构建）。
    /// 以增量方式执行与 `validate()` 相同的规则，使错误能精确定位到
    /// 新寄存器。可选的管道可与寄存器一同添加。
    pub fn add_register(
        &mut self,
        device_name: &str,
        register: RegisterConfig,
        pipeline: Option<PipelineConfig>,
    ) -> Result<(), String> {
        // 1. sensor_id 必须在整个网关内唯一（先做不可变借用）。
        for d in &self.devices {
            for r in &d.registers {
                if r.sensor_id == register.sensor_id {
                    return Err(format!(
                        "duplicate sensor_id `{}` (must be unique gateway-wide)",
                        register.sensor_id
                    ));
                }
            }
        }

        // 2. 设备必须存在。
        let device = self
            .devices
            .iter_mut()
            .find(|d| d.name == device_name)
            .ok_or_else(|| format!("device `{device_name}` not found"))?;

        // 3. 地址范围不得与该设备上同功能区的寄存器重叠。
        let width = register.effective_count();
        let end = register
            .address
            .checked_add(width)
            .ok_or_else(|| "register address overflow".to_string())?;
        for r in &device.registers {
            if r.function == register.function {
                let other_end = r.address.saturating_add(r.effective_count());
                if register.address < other_end && r.address < end {
                    return Err(format!(
                        "register range [{}, {}) overlaps `{}` range [{}, {})",
                        register.address, end, r.name, r.address, other_end
                    ));
                }
            }
        }

        // 4. 管道（可选）必须匹配该寄存器且有效。
        if let Some(p) = &pipeline {
            if p.sensor_id != register.sensor_id {
                return Err(format!(
                    "pipeline sensor_id `{}` must match register sensor_id `{}`",
                    p.sensor_id, register.sensor_id
                ));
            }
            if p.stages.is_empty() {
                return Err("pipeline needs at least one stage".to_string());
            }
            for s in &p.stages {
                crate::pipeline::validate_stage(s).map_err(|e| format!("pipeline stage: {e}"))?;
            }
        }

        device.registers.push(register);
        if let Some(p) = pipeline {
            self.pipelines.push(p);
        }
        Ok(())
    }
}

/// 全局设置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    #[serde(default = "default_log_level")]
    pub log_level: LogLevel,
    /// 滚动文件日志目录（阶段 6）；`None` = 仅 stdout。
    #[serde(default)]
    pub log_dir: Option<String>,
    /// 滚动文件日志保留的最大文件数（默认 7）。
    #[serde(default = "default_log_max_files")]
    pub log_max_files: u32,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            log_level: default_log_level(),
            log_dir: None,
            log_max_files: default_log_max_files(),
        }
    }
}

fn default_log_level() -> LogLevel {
    LogLevel::Info
}
fn default_log_max_files() -> u32 {
    7
}

/// 日志详细程度，映射到 `logging` 中的 `tracing::Level`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    #[default]
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub fn as_tracing_level(&self) -> tracing::Level {
        match self {
            LogLevel::Trace => tracing::Level::TRACE,
            LogLevel::Debug => tracing::Level::DEBUG,
            LogLevel::Info => tracing::Level::INFO,
            LogLevel::Warn => tracing::Level::WARN,
            LogLevel::Error => tracing::Level::ERROR,
        }
    }
}

/// 一台 PCBA 设备（一个 Modbus 从站）及其寄存器映射。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceConfig {
    /// 唯一设备名（用于日志）。
    pub name: String,
    /// 传输方式："tcp"（Modbus-TCP）或 "rtu"（串口 Modbus-RTU）。
    #[serde(default)]
    pub transport: Transport,
    /// Modbus 从站/单元 id，0..=247。
    #[serde(default = "default_unit_id")]
    pub unit_id: u8,
    /// TCP：主机名或 IP。
    #[serde(default)]
    pub host: String,
    /// TCP：端口。
    #[serde(default = "default_port")]
    pub port: u16,
    /// 轮询周期。
    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u64,
    /// 每次请求的超时时间。
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    /// 轮询失败后的退避起始时间。
    #[serde(default = "default_reconnect_initial_ms")]
    pub reconnect_initial_ms: u64,
    /// 退避上限。
    #[serde(default = "default_reconnect_max_ms")]
    pub reconnect_max_ms: u64,
    /// RTU：串口名（如 "COM3"）。
    #[serde(default)]
    pub serial_port: Option<String>,
    /// RTU：波特率（如 9600）。
    #[serde(default)]
    pub baud_rate: Option<u32>,
    /// 每次轮询要读取的寄存器。
    #[serde(default)]
    pub registers: Vec<RegisterConfig>,
}

fn default_unit_id() -> u8 {
    1
}
fn default_port() -> u16 {
    502
}
fn default_poll_interval_ms() -> u64 {
    1000
}
fn default_timeout_ms() -> u64 {
    1000
}
fn default_reconnect_initial_ms() -> u64 {
    1000
}
fn default_reconnect_max_ms() -> u64 {
    30000
}

/// Modbus 传输类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    #[default]
    Tcp,
    Rtu,
    /// 仿真数据源：无硬件，从 `[sim]` 配置计算传感器值（阶段 8 扩展）。
    Sim,
}

/// 寄存器地址空间（功能码）。覆盖四个 Modbus 区域。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RegisterFunction {
    /// 保持寄存器（0x03 读 / 0x06、0x10 写）。可读写。
    #[default]
    Holding,
    /// 输入寄存器（0x04）。只读。
    Input,
    /// 线圈（0x01 读 / 0x05 写）。单 bit，可读写。
    Coil,
    /// 离散输入（0x02）。单 bit，只读。
    DiscreteInput,
}

impl RegisterFunction {
    pub fn function_code(&self) -> u8 {
        match self {
            RegisterFunction::Holding => 0x03,
            RegisterFunction::Input => 0x04,
            RegisterFunction::Coil => 0x01,
            RegisterFunction::DiscreteInput => 0x02,
        }
    }

    /// 该区域是否可被协议层写入（holding/coil）。
    pub fn is_writable(&self) -> bool {
        matches!(self, RegisterFunction::Holding | RegisterFunction::Coil)
    }

    /// 该区域是否单 bit（coil/离散输入）。
    pub fn is_bit(&self) -> bool {
        matches!(self, RegisterFunction::Coil | RegisterFunction::DiscreteInput)
    }
}

/// 寄存器访问权限：决定协议层是否可以写入。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    #[default]
    Read,
    ReadWrite,
}

/// 如何解释一组寄存器的字。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ValueType {
    #[default]
    U16,
    I16,
    U32,
    I32,
    F32,
    /// 单个 bit（0/1）；与 `coil` / `discrete_input` 搭配使用。
    Bool,
}

impl ValueType {
    /// 该值占用的 16 位寄存器数量。
    pub fn register_count(&self) -> u16 {
        match self {
            ValueType::U16 | ValueType::I16 | ValueType::Bool => 1,
            ValueType::U32 | ValueType::I32 | ValueType::F32 => 2,
        }
    }
}

/// 多寄存器值的字序。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum WordOrder {
    /// 第一个寄存器存高字（如 `[0xDEAD, 0xBEEF]` -> 0xDEADBEEF）。
    #[default]
    Big,
    /// 第一个寄存器存低字。
    Little,
}

/// 要读取的一组寄存器及其解码方式。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterConfig {
    /// 人类可读名称（仅用于日志）。
    pub name: String,
    /// 全网关唯一的指标键（如 "pcba-01.cpu_temp"）。
    pub sensor_id: String,
    /// 寄存器地址空间。
    #[serde(default = "default_function")]
    pub function: RegisterFunction,
    /// 起始地址（从 0 开始）。
    pub address: u16,
    /// 16 位寄存器数量；默认为 `value_type` 的大小
    /// （u16/i16 为 1，u32/i32/f32 为 2）。显式设置时必须匹配。
    #[serde(default)]
    pub count: Option<u16>,
    /// 寄存器组的数值解释。
    #[serde(default)]
    pub value_type: ValueType,
    /// 多寄存器值的字序。
    #[serde(default)]
    pub word_order: WordOrder,
    /// 原始单位，仅作信息展示（管道在阶段 3 换算单位）。
    #[serde(default)]
    pub unit: Option<String>,
    /// 访问权限（read | read_write）。可读写寄存器可被
    /// 协议层写入（仅限 holding/coil 区域）。
    #[serde(default)]
    pub access: Access,
}

impl RegisterConfig {
    /// 要读取的 16 位寄存器数量：显式 `count`，或省略时
    /// 由 `value_type` 推导。
    pub fn effective_count(&self) -> u16 {
        self.count.unwrap_or_else(|| self.value_type.register_count())
    }
}

fn default_function() -> RegisterFunction {
    RegisterFunction::Holding
}

/// 虚拟传感器：通过数学表达式从其他传感器的值计算出的指标
/// （露点、温差/压差等）。无需硬件读取。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComputedConfig {
    /// 唯一指标键（全网关唯一；不得与寄存器冲突）。
    pub sensor_id: String,
    /// 显示名称。
    pub name: String,
    /// 物理单位（供协议层分类使用）。
    #[serde(default)]
    pub unit: Option<String>,
    /// 变量名 -> 引用的 sensor_id（真实寄存器或另一个 computed）。
    pub inputs: std::collections::HashMap<String, String>,
    /// 基于 `inputs` 键的数学表达式（meval 语法）。
    pub expression: String,
}

/// 单个传感器的处理管道：按顺序应用的一组阶段，
/// 将原始样本转换为处理后的指标（阶段 3）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineConfig {
    /// 必须匹配某设备寄存器的 `sensor_id`（全网关唯一）。
    pub sensor_id: String,
    /// 阶段，每个样本按顺序执行。
    #[serde(default)]
    pub stages: Vec<StageConfig>,
}

/// 管道的一个阶段。标签即 TOML 的 `type` 字段。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StageConfig {
    /// 线性换算：`value = value * scale + offset`，可选更新单位。
    Scale {
        scale: f64,
        #[serde(default)]
        offset: f64,
        #[serde(default)]
        unit: Option<String>,
    },
    /// 滑动窗口平均滤波。
    SlidingAverage {
        window: usize,
    },
    /// 滑动窗口中值滤波。
    Median {
        window: usize,
    },
    /// 基于当前值的数学表达式（变量 `v`），如 `(v - 273.15) * 10`。
    Math {
        expression: String,
    },
    /// 阈值检查，设置指标状态（critical 优先于 warning）。
    Threshold {
        #[serde(default)]
        low_warning: Option<f64>,
        #[serde(default)]
        high_warning: Option<f64>,
        #[serde(default)]
        low_critical: Option<f64>,
        #[serde(default)]
        high_critical: Option<f64>,
    },
    /// 窗口统计（min / max / avg），替换当前值。
    Aggregate {
        window: usize,
        mode: AggregateMode,
    },
}

/// [`StageConfig::Aggregate`] 的聚合模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AggregateMode {
    Min,
    Max,
    Avg,
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_TOML: &str = r#"
[general]
log_level = "debug"

[[devices]]
name = "pcba-01"
transport = "tcp"
host = "127.0.0.1"
port = 1502
unit_id = 1
poll_interval_ms = 500

[[devices.registers]]
name = "cpu_temp_raw"
sensor_id = "pcba-01.cpu_temp"
function = "holding"
address = 0
value_type = "u16"
unit = "counts"

[[devices.registers]]
name = "voltage_raw"
sensor_id = "pcba-01.voltage"
function = "holding"
address = 4
count = 2
value_type = "f32"
word_order = "big"
"#;

    #[test]
    fn parses_pipeline_sections() {
        let toml = r#"
[[devices]]
name = "a"
host = "127.0.0.1"
[[devices.registers]]
name = "r1"
sensor_id = "s.1"
address = 0
value_type = "u16"

[[pipeline]]
sensor_id = "s.1"
[[pipeline.stages]]
type = "scale"
scale = 0.1
unit = "°C"
[[pipeline.stages]]
type = "threshold"
high_warning = 80
"#;
        let cfg: Config = toml::from_str(toml).unwrap();
        assert_eq!(cfg.pipelines.len(), 1);
        let pipe = &cfg.pipelines[0];
        assert_eq!(pipe.sensor_id, "s.1");
        assert_eq!(pipe.stages.len(), 2);
        cfg.validate().unwrap();
    }

    #[test]
    fn rejects_pipeline_for_unknown_sensor() {
        let toml = r#"
[[devices]]
name = "a"
host = "127.0.0.1"
[[devices.registers]]
name = "r1"
sensor_id = "s.1"
address = 0

[[pipeline]]
sensor_id = "s.nope"
[[pipeline.stages]]
type = "scale"
scale = 1.0
"#;
        let cfg: Config = toml::from_str(toml).unwrap();
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("does not match any device register"), "got: {err}");
    }

    #[test]
    fn rejects_duplicate_pipeline_for_sensor() {
        let toml = r#"
[[devices]]
name = "a"
host = "127.0.0.1"
[[devices.registers]]
name = "r1"
sensor_id = "s.1"
address = 0

[[pipeline]]
sensor_id = "s.1"
[[pipeline.stages]]
type = "scale"
scale = 1.0

[[pipeline]]
sensor_id = "s.1"
[[pipeline.stages]]
type = "scale"
scale = 2.0
"#;
        let cfg: Config = toml::from_str(toml).unwrap();
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("only one pipeline per sensor"), "got: {err}");
    }

    #[test]
    fn rejects_invalid_stage_expression() {
        let toml = r#"
[[devices]]
name = "a"
host = "127.0.0.1"
[[devices.registers]]
name = "r1"
sensor_id = "s.1"
address = 0

[[pipeline]]
sensor_id = "s.1"
[[pipeline.stages]]
type = "math"
expression = "v +* 2"
"#;
        let cfg: Config = toml::from_str(toml).unwrap();
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("invalid expression"), "got: {err}");
    }

    #[test]
    fn parses_valid_config() {
        let cfg: Config = toml::from_str(VALID_TOML).unwrap();
        assert_eq!(cfg.devices.len(), 1);
        let dev = &cfg.devices[0];
        assert_eq!(dev.host, "127.0.0.1");
        assert_eq!(dev.port, 1502);
        assert_eq!(dev.registers.len(), 2);
        // count 默认为 value_type 的大小
        assert_eq!(dev.registers[0].effective_count(), 1);
        assert_eq!(dev.registers[1].effective_count(), 2);
        cfg.validate().unwrap();
    }

    #[test]
    fn rejects_duplicate_sensor_id() {
        let toml = r#"
[[devices]]
name = "a"
host = "127.0.0.1"
[[devices.registers]]
name = "r1"
sensor_id = "s.dup"
address = 0
[[devices.registers]]
name = "r2"
sensor_id = "s.dup"
address = 10
"#;
        let cfg: Config = toml::from_str(toml).unwrap();
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("duplicate sensor_id"), "got: {err}");
    }

    #[test]
    fn rejects_overlapping_register_ranges() {
        let toml = r#"
[[devices]]
name = "a"
host = "127.0.0.1"
[[devices.registers]]
name = "r1"
sensor_id = "s1"
address = 0
value_type = "u32"
[[devices.registers]]
name = "r2"
sensor_id = "s2"
address = 1
value_type = "u16"
"#;
        let cfg: Config = toml::from_str(toml).unwrap();
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("overlaps"), "got: {err}");
    }

    #[test]
    fn rejects_rtu_without_serial_port() {
        let toml = r#"
[[devices]]
name = "a"
transport = "rtu"
[[devices.registers]]
name = "r1"
sensor_id = "s1"
address = 0
"#;
        let cfg: Config = toml::from_str(toml).unwrap();
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("serial_port"), "got: {err}");
    }

    #[test]
    fn rejects_unit_id_out_of_range() {
        let toml = r#"
[[devices]]
name = "a"
host = "127.0.0.1"
unit_id = 250
[[devices.registers]]
name = "r1"
sensor_id = "s1"
address = 0
"#;
        let cfg: Config = toml::from_str(toml).unwrap();
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("unit_id"), "got: {err}");
    }

    #[test]
    fn rejects_unknown_transport_value() {
        let toml = r#"
[[devices]]
name = "a"
transport = "udp"
[[devices.registers]]
name = "r1"
sensor_id = "s1"
address = 0
"#;
        assert!(toml::from_str::<Config>(toml).is_err());
    }

    #[test]
    fn rejects_mismatched_count() {
        let toml = r#"
[[devices]]
name = "a"
host = "127.0.0.1"
[[devices.registers]]
name = "r1"
sensor_id = "s1"
address = 0
value_type = "f32"
count = 1
"#;
        let cfg: Config = toml::from_str(toml).unwrap();
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("count"), "got: {err}");
    }

    #[test]
    fn rejects_computed_cycle() {
        let toml = r#"
[[devices]]
name = "a"
host = "127.0.0.1"
[[devices.registers]]
name = "r1"
sensor_id = "s1"
address = 0

[[computed]]
sensor_id = "c1"
name = "c1"
expression = "b + 1"
[computed.inputs]
b = "c2"

[[computed]]
sensor_id = "c2"
name = "c2"
expression = "a + 1"
[computed.inputs]
a = "c1"
"#;
        let cfg: Config = toml::from_str(toml).unwrap();
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("cycle"), "got: {err}");
    }

    #[test]
    fn rejects_computed_unknown_input() {
        let toml = r#"
[[devices]]
name = "a"
host = "127.0.0.1"
[[devices.registers]]
name = "r1"
sensor_id = "s1"
address = 0

[[computed]]
sensor_id = "c1"
name = "c1"
expression = "x * 2"
[computed.inputs]
x = "s.does_not_exist"
"#;
        let cfg: Config = toml::from_str(toml).unwrap();
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("unknown sensor_id"), "got: {err}");
    }

    #[test]
    fn accepts_chained_computed_no_cycle() {
        let toml = r#"
[[devices]]
name = "a"
host = "127.0.0.1"
[[devices.registers]]
name = "r1"
sensor_id = "s1"
address = 0

[[computed]]
sensor_id = "c1"
name = "c1"
expression = "v * 2"
[computed.inputs]
v = "s1"

[[computed]]
sensor_id = "c2"
name = "c2"
expression = "c1v + 1"
[computed.inputs]
c1v = "c1"
"#;
        let cfg: Config = toml::from_str(toml).unwrap();
        cfg.validate().unwrap();
    }

    #[test]
    fn parses_cdu_sim_config() {
        // 端到端：CDU 仿真配置（config/cdu.toml）可解析且通过校验。
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("config/cdu.toml");
        let cfg = Config::load(&path).unwrap();
        assert_eq!(cfg.devices.len(), 1);
        assert_eq!(cfg.devices[0].transport, Transport::Sim);
        assert!(cfg.devices[0].registers.is_empty(), "sim 设备无需寄存器");
        assert_eq!(cfg.sim.controls.len(), 4, "pump1/pump2/valve1/fan");
        assert!(!cfg.sim.sensors.is_empty());
        // 派生量（computed）存在。
        assert!(cfg
            .computed
            .iter()
            .any(|c| c.sensor_id == "cdu.pump1.dp"));
    }

    #[test]
    fn rejects_sim_sensor_unknown_input() {
        let toml = r#"
[[devices]]
name = "a"
transport = "sim"

[sim]
[[sim.controls]]
name = "duty"
initial = 50
writable = true
[[sim.sensors]]
sensor_id = "s.flow"
name = "Flow"
kind = "flow"
formula = "duty * x"
[sim.sensors.inputs]
x = "s.nope"
"#;
        let cfg: Config = toml::from_str(toml).unwrap();
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("unknown target"), "got: {err}");
    }
}

//! TOML-driven configuration: devices, registers, polling behavior.
//!
//! Pipeline / endpoints / alerts sections are added in later phases; unknown
//! fields are ignored for forward compatibility.

use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

/// Top-level configuration, deserialized from TOML.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub general: GeneralConfig,
    #[serde(default)]
    pub devices: Vec<DeviceConfig>,
    /// Per-sensor processing pipelines (phase 3). TOML key: `[[pipeline]]`.
    #[serde(default, rename = "pipeline")]
    pub pipelines: Vec<PipelineConfig>,
}

impl Config {
    /// Load and validate a config file.
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("read config file `{}`", path.display()))?;
        let cfg: Config = toml::from_str(&text)
            .with_context(|| format!("parse config file `{}`", path.display()))?;
        cfg.validate()
            .with_context(|| format!("invalid config file `{}`", path.display()))?;
        Ok(cfg)
    }

    /// Semantic validation beyond TOML typing.
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
            if device.registers.is_empty() {
                bail!("device `{}`: at least one register is required", device.name);
            }
            if device.registers.len() > 256 {
                bail!("device `{}`: too many registers ({}), max 256", device.name, device.registers.len());
            }

            // Registers: unique names, unique sensor ids, no overlapping ranges.
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

        // Pipelines: sensor_id must reference a known register, be unique,
        // and contain valid stage parameters.
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
        Ok(())
    }

    /// Add a register to an existing device (runtime hot-add, dev builds).
    /// Runs the same rules as `validate()` incrementally so errors point at
    /// the new register. An optional pipeline may accompany the register.
    pub fn add_register(
        &mut self,
        device_name: &str,
        register: RegisterConfig,
        pipeline: Option<PipelineConfig>,
    ) -> Result<(), String> {
        // 1. sensor_id must be unique gateway-wide (immutable borrow first).
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

        // 2. Device must exist.
        let device = self
            .devices
            .iter_mut()
            .find(|d| d.name == device_name)
            .ok_or_else(|| format!("device `{device_name}` not found"))?;

        // 3. Address range must not overlap same-function registers on this device.
        let width = register.effective_count();
        let end = register
            .address
            .checked_add(width)
            .ok_or_else(|| "register address overflow".to_string())?;
        for r in &device.registers {
            if r.function == register.function {
                let other_end = r
                    .address
                    .checked_add(r.effective_count())
                    .unwrap_or(u16::MAX);
                if register.address < other_end && r.address < end {
                    return Err(format!(
                        "register range [{}, {}) overlaps `{}` range [{}, {})",
                        register.address, end, r.name, r.address, other_end
                    ));
                }
            }
        }

        // 4. Pipeline (optional) must match the register and be valid.
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

/// Global settings.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct GeneralConfig {
    #[serde(default = "default_log_level")]
    pub log_level: LogLevel,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            log_level: default_log_level(),
        }
    }
}

fn default_log_level() -> LogLevel {
    LogLevel::Info
}

/// Log verbosity, mapped to a `tracing::Level` in `logging`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl Default for LogLevel {
    fn default() -> Self {
        Self::Info
    }
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

/// One PCBA device (a Modbus slave) and its register map.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceConfig {
    /// Unique device name (used in logs).
    pub name: String,
    /// Transport: "tcp" (Modbus-TCP) or "rtu" (Modbus-RTU over serial).
    #[serde(default)]
    pub transport: Transport,
    /// Modbus slave/unit id, 0..=247.
    #[serde(default = "default_unit_id")]
    pub unit_id: u8,
    /// TCP: host name or IP.
    #[serde(default)]
    pub host: String,
    /// TCP: port.
    #[serde(default = "default_port")]
    pub port: u16,
    /// Polling period.
    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u64,
    /// Per-request timeout.
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    /// Backoff start for failed polls.
    #[serde(default = "default_reconnect_initial_ms")]
    pub reconnect_initial_ms: u64,
    /// Backoff cap.
    #[serde(default = "default_reconnect_max_ms")]
    pub reconnect_max_ms: u64,
    /// RTU: serial port name (e.g. "COM3").
    #[serde(default)]
    pub serial_port: Option<String>,
    /// RTU: baud rate (e.g. 9600).
    #[serde(default)]
    pub baud_rate: Option<u32>,
    /// Registers to read on every poll.
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

/// Modbus transport type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    #[default]
    Tcp,
    Rtu,
}

/// Register address space (function code).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RegisterFunction {
    /// Holding registers (function 0x03).
    Holding,
    /// Input registers (function 0x04).
    Input,
}

impl RegisterFunction {
    pub fn function_code(&self) -> u8 {
        match self {
            RegisterFunction::Holding => 0x03,
            RegisterFunction::Input => 0x04,
        }
    }
}

/// How to interpret a register group's words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ValueType {
    #[default]
    U16,
    I16,
    U32,
    I32,
    F32,
}

impl ValueType {
    /// Number of 16-bit registers this value occupies.
    pub fn register_count(&self) -> u16 {
        match self {
            ValueType::U16 | ValueType::I16 => 1,
            ValueType::U32 | ValueType::I32 | ValueType::F32 => 2,
        }
    }
}

/// Word order for multi-register values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum WordOrder {
    /// First register holds the high word (e.g. `[0xDEAD, 0xBEEF]` -> 0xDEADBEEF).
    #[default]
    Big,
    /// First register holds the low word.
    Little,
}

/// One register group to read and how to decode it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterConfig {
    /// Human-readable name (logs only).
    pub name: String,
    /// Unique gateway-wide metric key (e.g. "pcba-01.cpu_temp").
    pub sensor_id: String,
    /// Register address space.
    #[serde(default = "default_function")]
    pub function: RegisterFunction,
    /// Start address (0-based).
    pub address: u16,
    /// Number of 16-bit registers; defaults to the size of `value_type`
    /// (1 for u16/i16, 2 for u32/i32/f32). Must match when set explicitly.
    #[serde(default)]
    pub count: Option<u16>,
    /// Numeric interpretation of the register group.
    #[serde(default)]
    pub value_type: ValueType,
    /// Word order for multi-register values.
    #[serde(default)]
    pub word_order: WordOrder,
    /// Raw unit, informational (pipeline converts units in phase 3).
    #[serde(default)]
    pub unit: Option<String>,
}

impl RegisterConfig {
    /// Number of 16-bit registers to read: explicit `count`, or derived from
    /// `value_type` when omitted.
    pub fn effective_count(&self) -> u16 {
        self.count.unwrap_or_else(|| self.value_type.register_count())
    }
}

fn default_function() -> RegisterFunction {
    RegisterFunction::Holding
}

/// Processing pipeline for one sensor: a chain of stages applied in order,
/// converting a raw sample into a processed metric (phase 3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineConfig {
    /// Must match the `sensor_id` of some device register (gateway-wide unique).
    pub sensor_id: String,
    /// Stages, executed in order on every sample.
    #[serde(default)]
    pub stages: Vec<StageConfig>,
}

/// One stage of a pipeline. The tag is the TOML `type` field.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StageConfig {
    /// Linear conversion: `value = value * scale + offset`, optionally updates the unit.
    Scale {
        scale: f64,
        #[serde(default)]
        offset: f64,
        #[serde(default)]
        unit: Option<String>,
    },
    /// Sliding window average filter.
    SlidingAverage {
        window: usize,
    },
    /// Sliding window median filter.
    Median {
        window: usize,
    },
    /// Math expression over the current value (variable `v`), e.g. `(v - 273.15) * 10`.
    Math {
        expression: String,
    },
    /// Threshold check, sets the metric status (critical beats warning).
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
    /// Windowed statistics (min / max / avg), replacing the value.
    Aggregate {
        window: usize,
        mode: AggregateMode,
    },
}

/// Aggregation modes for [`StageConfig::Aggregate`].
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
        // count defaults to value_type size
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
}

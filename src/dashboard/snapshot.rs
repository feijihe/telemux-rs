//! Snapshot model: register configuration + latest raw sample + computed
//! metric, serialized as JSON for the dev dashboard.

use serde::Serialize;

use crate::config::{Config, RegisterFunction, Transport};
use crate::config_handle::ConfigHandle;
use crate::domain::{Metric, MetricStatus, RawSample, SensorId};
use crate::store::MetricStore;

/// Full snapshot of all devices and their latest samples.
#[derive(Debug, Clone, Serialize)]
pub struct DashboardSnapshot {
    /// Unix millis when the snapshot was generated.
    pub generated_at_ms: u64,
    pub devices: Vec<DeviceSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceSnapshot {
    pub name: String,
    pub transport: String,
    pub host: String,
    pub port: u16,
    /// True if a raw sample was seen recently (within ~2 poll intervals).
    pub connected: bool,
    pub registers: Vec<RegisterSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RegisterSnapshot {
    pub name: String,
    pub sensor_id: String,
    pub function: &'static str,
    pub address: u16,
    pub count: u16,
    pub value_type: &'static str,
    pub word_order: &'static str,
    pub unit: Option<String>,
    /// Latest raw sample; `None` until the first successful read.
    pub raw: Option<ValueSnapshot>,
    /// Latest computed metric; `None` until the pipeline produces one.
    pub metric: Option<MetricSnapshot>,
    /// Human-readable pipeline formula (joined stages); `None` without a pipeline.
    pub formula: Option<String>,
    /// Per-stage formula texts (for the expandable detail row).
    pub stages: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ValueSnapshot {
    pub value: f64,
    pub timestamp_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct MetricSnapshot {
    pub value: f64,
    pub unit: Option<String>,
    pub status: MetricStatus,
    pub timestamp_ms: u64,
}

impl From<&RawSample> for ValueSnapshot {
    fn from(s: &RawSample) -> Self {
        Self {
            value: s.raw_value,
            timestamp_ms: s.timestamp_ms(),
        }
    }
}

impl From<&Metric> for MetricSnapshot {
    fn from(m: &Metric) -> Self {
        Self {
            value: m.value,
            unit: m.unit.clone(),
            status: m.status,
            timestamp_ms: m.timestamp_ms(),
        }
    }
}

/// 增量更新消息：只含动态数据（raw + metric），不含静态配置。
/// 完整配置表由 `GET /api/snapshot` 提供（首次加载、新增寄存器后重新拉取）。
#[derive(Debug, Clone, Serialize)]
pub struct SampleUpdate {
    pub sensor_id: String,
    pub raw: Option<ValueSnapshot>,
    pub metric: Option<MetricSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateMessage {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub generated_at_ms: u64,
    pub samples: Vec<SampleUpdate>,
}

/// 从 store 构建增量更新：所有已知传感器的最新 raw + metric。
/// 尚未被采集到的传感器不出现（前端保持占位）。
pub fn build_update(store: &MetricStore) -> UpdateMessage {
    let mut samples = Vec::new();
    for (sensor_id, state) in store.snapshot() {
        samples.push(SampleUpdate {
            sensor_id: sensor_id.as_str().to_string(),
            raw: Some(ValueSnapshot::from(&state.raw)),
            metric: state.metric.as_ref().map(MetricSnapshot::from),
        });
    }
    UpdateMessage {
        kind: "update",
        generated_at_ms: now_ms(),
        samples,
    }
}

/// Build a snapshot from the runtime configuration + the latest store state.
pub fn build_snapshot(handle: &ConfigHandle, store: &MetricStore) -> DashboardSnapshot {
    let config: Config = handle.read();
    let generated_at_ms = now_ms();
    let mut devices = Vec::with_capacity(config.devices.len());
    for device in &config.devices {
        let mut registers = Vec::with_capacity(device.registers.len());
        let mut newest: Option<u64> = None;
        for reg in &device.registers {
            let state = store.get(&SensorId(reg.sensor_id.clone()));
            let ts = state.as_ref().map(|s| s.raw.timestamp_ms());
            newest = newest.max(ts);
            let pipeline = config
                .pipelines
                .iter()
                .find(|p| p.sensor_id == reg.sensor_id);
            registers.push(RegisterSnapshot {
                name: reg.name.clone(),
                sensor_id: reg.sensor_id.clone(),
                function: function_str(reg.function),
                address: reg.address,
                count: reg.effective_count(),
                value_type: value_type_str(reg.value_type),
                word_order: word_order_str(reg.word_order),
                unit: reg.unit.clone(),
                raw: state.as_ref().map(|s| ValueSnapshot::from(&s.raw)),
                metric: state.as_ref().and_then(|s| s.metric.as_ref()).map(MetricSnapshot::from),
                formula: pipeline.map(crate::pipeline::describe_pipeline),
                stages: pipeline
                    .map(|p| p.stages.iter().map(crate::pipeline::describe_stage).collect())
                    .unwrap_or_default(),
            });
        }
        devices.push(DeviceSnapshot {
            name: device.name.clone(),
            transport: transport_str(device.transport),
            host: device.host.clone(),
            port: device.port,
            connected: is_fresh(newest, device.poll_interval_ms),
            registers,
        });
    }
    DashboardSnapshot {
        generated_at_ms,
        devices,
    }
}

/// A sample is "fresh" if it arrived within ~2 poll intervals of now.
fn is_fresh(newest: Option<u64>, poll_interval_ms: u64) -> bool {
    match newest {
        Some(ts) => now_ms().saturating_sub(ts) <= poll_interval_ms * 2,
        None => false,
    }
}

fn now_ms() -> u64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_millis() as u64,
        Err(_) => 0,
    }
}

fn transport_str(t: Transport) -> String {
    match t {
        Transport::Tcp => "tcp".to_string(),
        Transport::Rtu => "rtu".to_string(),
    }
}

fn function_str(f: RegisterFunction) -> &'static str {
    match f {
        RegisterFunction::Holding => "holding",
        RegisterFunction::Input => "input",
    }
}

fn value_type_str(v: crate::config::ValueType) -> &'static str {
    match v {
        crate::config::ValueType::U16 => "u16",
        crate::config::ValueType::I16 => "i16",
        crate::config::ValueType::U32 => "u32",
        crate::config::ValueType::I32 => "i32",
        crate::config::ValueType::F32 => "f32",
    }
}

fn word_order_str(w: crate::config::WordOrder) -> &'static str {
    match w {
        crate::config::WordOrder::Big => "big",
        crate::config::WordOrder::Little => "little",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        DeviceConfig, RegisterConfig, RegisterFunction, ValueType, WordOrder,
    };
    use crate::config_handle::ConfigHandle;

    fn sample_config() -> Config {
        Config {
            general: Default::default(),
            devices: vec![DeviceConfig {
                name: "pcba-01".into(),
                transport: Transport::Tcp,
                unit_id: 1,
                host: "127.0.0.1".into(),
                port: 1502,
                poll_interval_ms: 500,
                timeout_ms: 1000,
                reconnect_initial_ms: 100,
                reconnect_max_ms: 1000,
                serial_port: None,
                baud_rate: None,
                registers: vec![RegisterConfig {
                    name: "cpu_temp_raw".into(),
                    sensor_id: "pcba-01.cpu_temp".into(),
                    function: RegisterFunction::Holding,
                    address: 0,
                    count: Some(1),
                    value_type: ValueType::U16,
                    word_order: WordOrder::Big,
                    unit: Some("counts".into()),
                }],
            }],
            pipelines: vec![],
        }
    }

    fn sample(id: &str, raw: f64) -> RawSample {
        RawSample {
            sensor_id: SensorId(id.into()),
            name: "cpu_temp_raw".into(),
            raw_value: raw,
            unit: Some("counts".into()),
            timestamp: std::time::SystemTime::now(),
        }
    }

    #[test]
    fn snapshot_without_samples_has_empty_raw_and_metric() {
        let handle = ConfigHandle::new(sample_config(), "unused.toml".into());
        let store = MetricStore::new();
        let snap = build_snapshot(&handle, &store);
        assert_eq!(snap.devices.len(), 1);
        let reg = &snap.devices[0].registers[0];
        assert!(reg.raw.is_none());
        assert!(reg.metric.is_none());
        assert!(!snap.devices[0].connected);
    }

    #[test]
    fn snapshot_includes_raw_and_metric() {
        let handle = ConfigHandle::new(sample_config(), "unused.toml".into());
        let store = MetricStore::new();
        store.update_metric(
            sample("pcba-01.cpu_temp", 251.0),
            Metric {
                sensor_id: SensorId("pcba-01.cpu_temp".into()),
                value: 25.1,
                unit: Some("°C".into()),
                status: MetricStatus::Normal,
                timestamp: std::time::SystemTime::now(),
            },
        );
        let snap = build_snapshot(&handle, &store);
        let reg = &snap.devices[0].registers[0];
        assert_eq!(reg.raw.as_ref().unwrap().value, 251.0);
        let m = reg.metric.as_ref().unwrap();
        assert_eq!(m.value, 25.1);
        assert_eq!(m.status, MetricStatus::Normal);
        assert!(snap.devices[0].connected);
    }

    #[test]
    fn snapshot_is_valid_json() {
        let handle = ConfigHandle::new(sample_config(), "unused.toml".into());
        let store = MetricStore::new();
        let snap = build_snapshot(&handle, &store);
        let json = serde_json::to_string(&snap).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["devices"][0]["registers"][0]["name"], "cpu_temp_raw");
    }
}

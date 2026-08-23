//! 协议层（阶段 5）：Redfish 与 Modbus 服务器通过各自协议暴露指标存储。
//! 两者完全由配置驱动：传感器列表和寄存器表在每次请求时基于实时的
//! `ConfigHandle` 构建，因此配置中新增的寄存器/computed 传感器会自动出现。

pub mod modbus_server;
pub mod redfish;

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{mpsc, oneshot};

use crate::config::{Access, Config, RegisterFunction, ValueType, WordOrder};
use crate::domain::{MetricStatus, SensorId};
use crate::store::MetricStore;

/// 与协议无关的统一传感器视图（真实寄存器或 computed）。
/// 由配置 + 存储的最新状态构建。
#[derive(Debug, Clone)]
pub struct SensorView {
    pub sensor_id: String,
    pub name: String,
    pub device: String,
    pub is_computed: bool,
    pub function: RegisterFunction,
    pub access: Access,
    pub address: u16,
    pub value_type: ValueType,
    pub word_order: WordOrder,
    pub unit: Option<String>,
    pub formula: Option<String>,
    /// 当前值：优先指标，原始值作为回退。
    pub value: Option<f64>,
    pub status: MetricStatus,
    pub timestamp_ms: Option<u64>,
}

/// 从配置 + 存储构建传感器视图。各设备的真实寄存器在前
/// （按配置顺序），随后是 computed 传感器。
pub fn build_views(config: &Config, store: &MetricStore) -> Vec<SensorView> {
    let mut views = Vec::new();
    for device in &config.devices {
        for reg in &device.registers {
            let state = store.get(&SensorId(reg.sensor_id.clone()));
            let (value, status, ts) = value_of(&state.as_ref().map(|s| (&s.metric, &s.raw)));
            let formula = config
                .pipelines
                .iter()
                .find(|p| p.sensor_id == reg.sensor_id)
                .map(crate::pipeline::describe_pipeline);
            views.push(SensorView {
                sensor_id: reg.sensor_id.clone(),
                name: reg.name.clone(),
                device: device.name.clone(),
                is_computed: false,
                function: reg.function,
                access: reg.access,
                address: reg.address,
                value_type: reg.value_type,
                word_order: reg.word_order,
                unit: reg.unit.clone(),
                formula,
                value,
                status,
                timestamp_ms: ts,
            });
        }
    }
    for c in &config.computed {
        let state = store.get(&SensorId(c.sensor_id.clone()));
        let (value, status, ts) = value_of(&state.as_ref().map(|s| (&s.metric, &s.raw)));
        views.push(SensorView {
            sensor_id: c.sensor_id.clone(),
            name: c.name.clone(),
            device: String::new(), // computed 传感器不绑定到具体设备
            is_computed: true,
            function: RegisterFunction::Input,
            access: Access::Read,
            address: 0,
            value_type: ValueType::F32,
            word_order: WordOrder::Big,
            unit: c.unit.clone(),
            formula: Some(format!(
                "{}  (computed from {})",
                c.expression,
                c.inputs.values().cloned().collect::<Vec<_>>().join(", ")
            )),
            value,
            status,
            timestamp_ms: ts,
        });
    }
    views
}

/// 提取（值、状态、时间戳），优先指标而非原始值。
fn value_of(
    state: &Option<(&Option<crate::domain::Metric>, &Option<crate::domain::RawSample>)>,
) -> (Option<f64>, MetricStatus, Option<u64>) {
    match state {
        Some((Some(metric), _)) => (
            Some(metric.value),
            metric.status,
            Some(metric.timestamp_ms()),
        ),
        Some((None, Some(raw))) => (
            Some(raw.raw_value),
            MetricStatus::Unknown,
            Some(raw.timestamp_ms()),
        ),
        _ => (None, MetricStatus::Unknown, None),
    }
}

/// 从协议层转发到采集层的写请求。
#[derive(Debug)]
pub struct WriteRequest {
    pub sensor_id: String,
    pub value: WriteValue,
    pub reply: oneshot::Sender<Result<(), crate::acquisition::AcquisitionError>>,
}

#[derive(Debug, Clone, Copy)]
pub enum WriteValue {
    Holding(u16),
    Coil(bool),
}

/// 将写请求路由到持有数据源的设备轮询任务。
#[derive(Debug, Default, Clone)]
pub struct WriteBroker {
    txs: Arc<HashMap<String, mpsc::Sender<WriteRequest>>>,
}

#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    #[error("unknown device `{0}` (no poll task for it)")]
    UnknownDevice(String),
    #[error("write channel closed")]
    BrokerClosed,
    #[error("no reply from acquisition layer")]
    NoReply,
    #[error("acquisition error: {0}")]
    Acquisition(#[from] crate::acquisition::AcquisitionError),
}

impl WriteBroker {
    pub fn new(txs: HashMap<String, mpsc::Sender<WriteRequest>>) -> Self {
        Self { txs: Arc::new(txs) }
    }

    pub fn has_device(&self, device: &str) -> bool {
        self.txs.contains_key(device)
    }

    /// 向设备的轮询任务发送写请求并等待结果。
    pub async fn write(
        &self,
        device: &str,
        sensor_id: &str,
        value: WriteValue,
    ) -> Result<(), WriteError> {
        let tx = self
            .txs
            .get(device)
            .ok_or_else(|| WriteError::UnknownDevice(device.to_string()))?;
        let (reply_tx, reply_rx) = oneshot::channel();
        tx.send(WriteRequest {
            sensor_id: sensor_id.to_string(),
            value,
            reply: reply_tx,
        })
        .await
        .map_err(|_| WriteError::BrokerClosed)?;
        match reply_rx.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(WriteError::Acquisition(e)),
            Err(_) => Err(WriteError::NoReply),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, DeviceConfig, RegisterConfig, Transport};

    fn sample_config() -> Config {
        Config {
            general: Default::default(),
            devices: vec![DeviceConfig {
                name: "pcba-01".into(),
                transport: Transport::Tcp,
                unit_id: 1,
                host: "127.0.0.1".into(),
                port: 502,
                poll_interval_ms: 100,
                timeout_ms: 100,
                reconnect_initial_ms: 100,
                reconnect_max_ms: 1000,
                serial_port: None,
                baud_rate: None,
                registers: vec![RegisterConfig {
                    name: "r1".into(),
                    sensor_id: "s.r1".into(),
                    function: RegisterFunction::Input,
                    address: 0,
                    count: Some(1),
                    value_type: ValueType::U16,
                    word_order: WordOrder::Big,
                    unit: Some("°C".into()),
                    access: Access::Read,
                }],
            }],
            pipelines: vec![],
            computed: vec![crate::config::ComputedConfig {
                sensor_id: "s.dew".into(),
                name: "dew".into(),
                unit: Some("°C".into()),
                inputs: [("t".to_string(), "s.r1".to_string())].into(),
                expression: "t * 2".into(),
            }],
            endpoints: Default::default(),
        }
    }

    #[test]
    fn views_include_registers_and_computed() {
        let store = MetricStore::new();
        let views = build_views(&sample_config(), &store);
        assert_eq!(views.len(), 2);
        assert!(!views[0].is_computed);
        assert!(views[1].is_computed);
        assert!(views[1].value.is_none(), "尚无数据");
    }
}

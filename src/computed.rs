//! 虚拟传感器（计算指标）：通过数学表达式从其他传感器的值派生
//! （露点、温差/压差等）。
//!
//! 无需硬件读取：每次采集批次都会基于存储中的最新值重新求值每个
//! computed 传感器；结果以"无原始样本的指标"写入存储，因此协议层
//! （仪表盘 / Redfish / Modbus）将它们与真实传感器完全等同地暴露。

use std::str::FromStr;

use crate::config::{Config, ComputedConfig};
use crate::config_handle::ConfigHandle;
use crate::domain::{Metric, MetricStatus, SensorId};
use crate::store::MetricStore;

/// 一个绑定到其输入的计算传感器。
pub struct ComputedSensor {
    config: ComputedConfig,
    expr: meval::Expr,
    /// 变量名，与 `inputs` 对齐（构建时固定顺序）。
    vars: Vec<String>,
    /// 被引用的传感器 id，与 `vars` 对齐。
    inputs: Vec<SensorId>,
}

impl ComputedSensor {
    pub fn new(config: ComputedConfig) -> Result<Self, String> {
        let expr = meval::Expr::from_str(&config.expression)
            .map_err(|e| format!("invalid expression `{}`: {e}", config.expression))?;
        // 从单次迭代固定变量/输入顺序（HashMap 顺序在迭代间不稳定）。
        let pairs: Vec<(String, SensorId)> = config
            .inputs
            .iter()
            .map(|(var, src)| (var.clone(), SensorId(src.clone())))
            .collect();
        let vars: Vec<String> = pairs.iter().map(|(v, _)| v.clone()).collect();
        let inputs: Vec<SensorId> = pairs.iter().map(|(_, s)| s.clone()).collect();
        // bindn 校验表达式的每个变量都已绑定到某个输入。
        // （用块限定作用域，使对 `vars` 的借用先于其移入 Self 结束）
        {
            let keys: Vec<&str> = vars.iter().map(String::as_str).collect();
            let _bound = expr
                .clone()
                .bindn(&keys)
                .map_err(|e| format!("expression `{}`: {e}", config.expression))?;
        }
        Ok(Self {
            config,
            expr,
            vars,
            inputs,
        })
    }

    /// 使用存储中的最新值求值。
    /// 任一输入尚无数据时返回 `None`（本轮跳过）。
    pub fn eval(&self, store: &MetricStore) -> Option<Metric> {
        let mut values = Vec::with_capacity(self.inputs.len());
        for input in &self.inputs {
            let state = store.get(input)?;
            // 输入值：优先取指标（输入可能是其他 computed 传感器），
            // 原始值作为回退。
            let value = match &state.metric {
                Some(m) => m.value,
                None => state.raw.as_ref()?.raw_value,
            };
            values.push(value);
        }
        let keys: Vec<&str> = self.vars.iter().map(String::as_str).collect();
        let f = self.expr.clone().bindn(&keys).ok()?;
        let value = f(&values);
        Some(Metric {
            sensor_id: SensorId(self.config.sensor_id.clone()),
            value,
            unit: self.config.unit.clone(),
            status: MetricStatus::Normal,
            timestamp: std::time::SystemTime::now(),
        })
    }
}

/// 计算传感器缓存，配置版本变化时重建。
/// 刻意不实现 `Send`（meval 表达式持有 Rc），在消费者线程中运行。
pub struct ComputedEngine {
    revision: u64,
    sensors: Vec<ComputedSensor>,
}

impl ComputedEngine {
    pub fn new(config: &Config) -> Self {
        Self {
            revision: 0,
            sensors: Self::build(&config.computed),
        }
    }

    fn build(configs: &[ComputedConfig]) -> Vec<ComputedSensor> {
        configs
            .iter()
            .map(|c| ComputedSensor::new(c.clone()).expect("validated computed builds"))
            .collect()
    }

    /// 自上次刷新以来配置版本变化时重建。
    pub fn refresh(&mut self, handle: &ConfigHandle) {
        let rev = handle.revision();
        if rev != self.revision {
            let config = handle.read();
            self.sensors = Self::build(&config.computed);
            self.revision = rev;
            tracing::debug!("computed engine rebuilt ({} sensor(s))", self.sensors.len());
        }
    }

    pub fn len(&self) -> usize {
        self.sensors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sensors.is_empty()
    }

    /// 将所有 computed 传感器求值并写入存储（无原始值的指标）。
    pub fn run(&self, store: &MetricStore) {
        for sensor in &self.sensors {
            if let Some(metric) = sensor.eval(store) {
                store.update_metric(None, metric);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ComputedConfig, DeviceConfig, RegisterConfig, RegisterFunction, ValueType, WordOrder};
    use crate::domain::RawSample;

    fn store_with(registers: &[(&str, f64)]) -> MetricStore {
        let store = MetricStore::new();
        for (id, v) in registers {
            store.update_raw(RawSample {
                sensor_id: SensorId((*id).into()),
                name: (*id).into(),
                raw_value: *v,
                unit: None,
                timestamp: std::time::SystemTime::now(),
            });
        }
        store
    }

    fn computed(inputs: &[(&str, &str)], expression: &str) -> ComputedConfig {
        ComputedConfig {
            sensor_id: "s.computed".into(),
            name: "computed".into(),
            unit: Some("°C".into()),
            inputs: inputs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            expression: expression.into(),
        }
    }

    #[test]
    fn evaluates_expression_from_inputs() {
        // 温差：t1 - t2
        let store = store_with(&[("s.t1", 30.0), ("s.t2", 20.0)]);
        let sensor = ComputedSensor::new(computed(&[("t1", "s.t1"), ("t2", "s.t2")], "t1 - t2"))
            .unwrap();
        let m = sensor.eval(&store).unwrap();
        assert_eq!(m.value, 10.0);
        assert_eq!(m.unit.as_deref(), Some("°C"));
    }

    #[test]
    fn skips_when_input_missing() {
        let store = store_with(&[("s.t1", 30.0)]); // t2 缺失
        let sensor = ComputedSensor::new(computed(&[("t1", "s.t1"), ("t2", "s.t2")], "t1 - t2"))
            .unwrap();
        assert!(sensor.eval(&store).is_none());
    }

    #[test]
    fn rejects_unknown_variable() {
        let err = match ComputedSensor::new(computed(&[("t1", "s.t1")], "t1 - t2")) {
            Ok(_) => panic!("expected error"),
            Err(e) => e,
        };
        assert!(err.contains("unknown variable"), "got: {err}");
    }

    #[test]
    fn engine_writes_metrics_without_raw() {
        let config = Config {
            general: Default::default(),
            devices: vec![DeviceConfig {
                name: "d".into(),
                transport: crate::config::Transport::Tcp,
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
                    name: "t1".into(),
                    sensor_id: "s.t1".into(),
                    function: RegisterFunction::Input,
                    address: 0,
                    count: Some(1),
                    value_type: ValueType::U16,
                    word_order: WordOrder::Big,
                    unit: None,
                    access: crate::config::Access::Read,
                }],
            }],
            pipelines: vec![],
            computed: vec![computed(&[("t1", "s.t1")], "t1 * 2")],
            endpoints: Default::default(),
        };
        let store = store_with(&[("s.t1", 21.0)]);
        let engine = ComputedEngine::new(&config);
        engine.run(&store);
        let state = store.get(&SensorId("s.computed".into())).unwrap();
        assert!(state.raw.is_none());
        assert_eq!(state.metric.unwrap().value, 42.0);
    }
}

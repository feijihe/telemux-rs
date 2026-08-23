//! 指标存储：每个传感器的最近原始样本 + 处理后的指标。
//!
//! 由采集消费者写入（先写原始值，管道运行后写指标）；供开发仪表盘以及
//! 后续的协议层（Redfish / SNMP / Modbus）读取。版本 watch 通道通知订阅者
//! 任何更新（供仪表盘、告警、trap 使用）。

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use tokio::sync::watch;

use crate::domain::{Metric, RawSample, SensorId};

/// 单个传感器的最近状态。
#[derive(Debug, Clone)]
pub struct SensorState {
    /// 最近的原始样本（管道之前）。computed 传感器为 `None`
    /// （虚拟指标没有硬件读取）。
    pub raw: Option<RawSample>,
    /// 最近的处理后指标（管道之后 / computed）；在存在之前为 `None`
    /// （例如传感器没有管道，或管道失败——此时有意保留旧指标）。
    pub metric: Option<Metric>,
}

/// 每传感器最近 `(raw, metric)` 的线程安全存储。
#[derive(Debug)]
pub struct MetricStore {
    inner: RwLock<HashMap<SensorId, SensorState>>,
    /// 每次写入时递增；订阅者将其作为变更信号。
    revision: watch::Sender<u64>,
}

impl MetricStore {
    pub fn new() -> Self {
        let (revision, _) = watch::channel(0u64);
        Self {
            inner: RwLock::new(HashMap::new()),
            revision,
        }
    }

    /// 记录一个原始样本（管道输入）。保留已有的指标。
    pub fn update_raw(&self, sample: RawSample) {
        if let Ok(mut inner) = self.inner.write() {
            let entry = inner.entry(sample.sensor_id.clone()).or_insert(SensorState {
                raw: Some(sample.clone()),
                metric: None,
            });
            entry.raw = Some(sample);
            let _ = self.revision.send(inner.len() as u64);
        }
    }

    /// 记录一批原始样本。
    pub fn update_batch_raw(&self, batch: &[RawSample]) {
        for sample in batch {
            self.update_raw(sample.clone());
        }
    }

    /// 记录一个处理后的指标及其来源原始样本。
    /// computed 传感器的 `raw` 为 `None`。
    pub fn update_metric(&self, raw: Option<RawSample>, metric: Metric) {
        if let Ok(mut inner) = self.inner.write() {
            inner.insert(
                metric.sensor_id.clone(),
                SensorState {
                    raw,
                    metric: Some(metric),
                },
            );
            let _ = self.revision.send(inner.len() as u64);
        }
    }

    /// 某传感器的最新状态，若存在。
    pub fn get(&self, sensor_id: &SensorId) -> Option<SensorState> {
        self.inner.read().ok()?.get(sensor_id).cloned()
    }

    /// 所有传感器的快照（最近的原始值 + 指标）。
    pub fn snapshot(&self) -> HashMap<SensorId, SensorState> {
        self.inner.read().map(|m| m.clone()).unwrap_or_default()
    }

    /// 已跟踪的传感器数量。
    pub fn len(&self) -> usize {
        self.inner.read().map(|m| m.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 订阅版本变更（每次写入后触发；以当前版本号作为值）。
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.revision.subscribe()
    }
}

impl Default for MetricStore {
    fn default() -> Self {
        Self::new()
    }
}

/// 便捷的共享句柄。
pub type SharedStore = Arc<MetricStore>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    fn raw(id: &str, value: f64) -> RawSample {
        RawSample {
            sensor_id: SensorId(id.into()),
            name: id.into(),
            raw_value: value,
            unit: None,
            timestamp: SystemTime::now(),
        }
    }

    fn metric(id: &str, value: f64) -> Metric {
        Metric {
            sensor_id: SensorId(id.into()),
            value,
            unit: None,
            status: crate::domain::MetricStatus::Normal,
            timestamp: SystemTime::now(),
        }
    }

    #[test]
    fn update_raw_then_metric() {
        let store = MetricStore::new();
        let id = SensorId("s.1".into());
        store.update_raw(raw("s.1", 100.0));
        let state = store.get(&id).unwrap();
        assert_eq!(state.raw.as_ref().unwrap().raw_value, 100.0);
        assert!(state.metric.is_none());

        store.update_metric(Some(raw("s.1", 101.0)), metric("s.1", 10.1));
        let state = store.get(&id).unwrap();
        assert_eq!(state.raw.as_ref().unwrap().raw_value, 101.0);
        assert_eq!(state.metric.unwrap().value, 10.1);
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn update_metric_without_raw_for_computed() {
        let store = MetricStore::new();
        store.update_metric(None, metric("s.dew", 18.6));
        let state = store.get(&SensorId("s.dew".into())).unwrap();
        assert!(state.raw.is_none(), "computed sensors have no raw");
        assert_eq!(state.metric.unwrap().value, 18.6);
    }

    #[test]
    fn update_raw_keeps_previous_metric() {
        let store = MetricStore::new();
        store.update_metric(Some(raw("s.1", 1.0)), metric("s.1", 1.0));
        store.update_raw(raw("s.1", 2.0)); // 管道尚未运行
        let state = store.get(&SensorId("s.1".into())).unwrap();
        assert_eq!(state.raw.as_ref().unwrap().raw_value, 2.0);
        assert_eq!(state.metric.unwrap().value, 1.0); // 保留过期指标
    }

    #[test]
    fn snapshot_and_revision_notify() {
        let store = MetricStore::new();
        let mut rx = store.subscribe();
        store.update_raw(raw("s.1", 1.0));
        store.update_metric(Some(raw("s.2", 2.0)), metric("s.2", 2.0));
        let snap = store.snapshot();
        assert_eq!(snap.len(), 2);
        // 版本通道应至少触发两次
        assert!(rx.has_changed().unwrap());
        let _ = rx.borrow_and_update();
    }
}

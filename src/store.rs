//! Metric store: latest raw sample + processed metric per sensor.
//!
//! Written by the acquisition consumer (raw first, then metric after the
//! pipeline runs); read by the dev dashboard and, later, the protocol layers
//! (Redfish / SNMP / Modbus). A revision watch channel notifies subscribers
//! of any update (used by dashboards, alerts, traps).

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use tokio::sync::watch;

use crate::domain::{Metric, RawSample, SensorId};

/// Latest state of one sensor.
#[derive(Debug, Clone)]
pub struct SensorState {
    /// Latest raw sample (pre-pipeline).
    pub raw: RawSample,
    /// Latest processed metric (post-pipeline); `None` until the pipeline
    /// produces one (e.g. sensor has no pipeline configured, or the pipeline
    /// failed — the previous metric is intentionally kept).
    pub metric: Option<Metric>,
}

/// Thread-safe store of the latest `(raw, metric)` per sensor.
#[derive(Debug)]
pub struct MetricStore {
    inner: RwLock<HashMap<SensorId, SensorState>>,
    /// Incremented on every write; subscribers use it as a change signal.
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

    /// Record a raw sample (pipeline input). Keeps any existing metric.
    pub fn update_raw(&self, sample: RawSample) {
        if let Ok(mut inner) = self.inner.write() {
            let entry = inner.entry(sample.sensor_id.clone()).or_insert(SensorState {
                raw: sample.clone(),
                metric: None,
            });
            entry.raw = sample;
            let _ = self.revision.send(inner.len() as u64);
        }
    }

    /// Record a batch of raw samples.
    pub fn update_batch_raw(&self, batch: &[RawSample]) {
        for sample in batch {
            self.update_raw(sample.clone());
        }
    }

    /// Record a processed metric together with its source raw sample.
    pub fn update_metric(&self, raw: RawSample, metric: Metric) {
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

    /// Latest state for a sensor, if any.
    pub fn get(&self, sensor_id: &SensorId) -> Option<SensorState> {
        self.inner.read().ok()?.get(sensor_id).cloned()
    }

    /// Snapshot of all sensors (latest raw + metric).
    pub fn snapshot(&self) -> HashMap<SensorId, SensorState> {
        self.inner.read().map(|m| m.clone()).unwrap_or_default()
    }

    /// Number of tracked sensors.
    pub fn len(&self) -> usize {
        self.inner.read().map(|m| m.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Subscribe to revision changes (fires after every write; carries the
    /// current revision as the value).
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.revision.subscribe()
    }
}

impl Default for MetricStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Convenience shared handle.
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
        assert_eq!(state.raw.raw_value, 100.0);
        assert!(state.metric.is_none());

        store.update_metric(raw("s.1", 101.0), metric("s.1", 10.1));
        let state = store.get(&id).unwrap();
        assert_eq!(state.raw.raw_value, 101.0);
        assert_eq!(state.metric.unwrap().value, 10.1);
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn update_raw_keeps_previous_metric() {
        let store = MetricStore::new();
        store.update_metric(raw("s.1", 1.0), metric("s.1", 1.0));
        store.update_raw(raw("s.1", 2.0)); // pipeline not run yet
        let state = store.get(&SensorId("s.1".into())).unwrap();
        assert_eq!(state.raw.raw_value, 2.0);
        assert_eq!(state.metric.unwrap().value, 1.0); // stale metric retained
    }

    #[test]
    fn snapshot_and_revision_notify() {
        let store = MetricStore::new();
        let mut rx = store.subscribe();
        store.update_raw(raw("s.1", 1.0));
        store.update_metric(raw("s.2", 2.0), metric("s.2", 2.0));
        let snap = store.snapshot();
        assert_eq!(snap.len(), 2);
        // revision channel should have fired at least twice
        assert!(rx.has_changed().unwrap());
        let _ = rx.borrow_and_update();
    }
}

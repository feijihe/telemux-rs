//! Core domain types shared across layers.
//!
//! Layer contract (see `docs/IMPLEMENTATION.md`):
//! - acquisition produces [`RawSample`] (decoded raw register values, no unit conversion)
//! - the processing pipeline (phase 3) converts [`RawSample`] into [`Metric`]

use std::time::SystemTime;

use serde::Serialize;

/// Stable identifier of a sensor / metric, unique across the whole gateway.
/// Used as the key in the metric store and in protocol mappings (phase 4/5).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SensorId(pub String);

impl SensorId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SensorId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// One raw reading from a sensor register, as decoded by the acquisition layer.
/// `raw_value` is the decoded register value; unit conversion / filtering is
/// intentionally left to the processing pipeline.
#[derive(Debug, Clone)]
pub struct RawSample {
    /// Unique metric key configured for this register.
    pub sensor_id: SensorId,
    /// Human-readable register name (from config, informational only).
    pub name: String,
    /// Decoded raw register value (f64 covers u32/i32/f32 exactly enough).
    pub raw_value: f64,
    /// Raw unit, informational (e.g. "counts"); the pipeline converts to real units.
    pub unit: Option<String>,
    /// Wall-clock time of the read.
    pub timestamp: SystemTime,
}

impl RawSample {
    pub fn now() -> SystemTime {
        SystemTime::now()
    }

    /// Milliseconds since UNIX_EPOCH (handy for logs).
    pub fn timestamp_ms(&self) -> u64 {
        match self.timestamp.duration_since(SystemTime::UNIX_EPOCH) {
            Ok(d) => d.as_millis() as u64,
            Err(_) => 0,
        }
    }
}

/// Health status of a processed metric (produced by pipeline threshold stages).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MetricStatus {
    Normal,
    Warning,
    Critical,
    Unknown,
}

/// A processed, standardized metric. Produced by the pipeline (phase 3);
/// defined here so the store and protocol layers can be built against it.
#[derive(Debug, Clone)]
pub struct Metric {
    pub sensor_id: SensorId,
    pub value: f64,
    pub unit: Option<String>,
    pub status: MetricStatus,
    pub timestamp: SystemTime,
}

impl Metric {
    /// Milliseconds since UNIX_EPOCH (handy for logs and snapshots).
    pub fn timestamp_ms(&self) -> u64 {
        match self.timestamp.duration_since(SystemTime::UNIX_EPOCH) {
            Ok(d) => d.as_millis() as u64,
            Err(_) => 0,
        }
    }
}

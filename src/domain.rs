//! 跨层共享的核心领域类型。
//!
//! 分层契约（见 `docs/IMPLEMENTATION.md`）：
//! - 采集层产出 [`RawSample`]（解码后的原始寄存器值，不做单位换算）
//! - 处理管道（阶段 3）将 [`RawSample`] 转换为 [`Metric`]

use std::time::SystemTime;

use serde::Serialize;

/// 传感器/指标的稳定标识符，在整个网关内唯一。
/// 用作指标存储的键以及协议映射（阶段 4/5）的键。
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

/// 来自传感器寄存器的一次原始读数，由采集层解码产生。
/// `raw_value` 是解码后的寄存器值；单位换算/滤波有意留给处理管道完成。
#[derive(Debug, Clone)]
pub struct RawSample {
    /// 为该寄存器配置的唯一指标键。
    pub sensor_id: SensorId,
    /// 人类可读的寄存器名（来自配置，仅作信息展示）。
    pub name: String,
    /// 解码后的原始寄存器值（f64 足以精确覆盖 u32/i32/f32）。
    pub raw_value: f64,
    /// 原始单位，仅作信息展示（如 "counts"）；管道负责换算为真实单位。
    pub unit: Option<String>,
    /// 读数的墙钟时间。
    pub timestamp: SystemTime,
}

impl RawSample {
    pub fn now() -> SystemTime {
        SystemTime::now()
    }

    /// 自 UNIX_EPOCH 起的毫秒数（便于日志记录）。
    pub fn timestamp_ms(&self) -> u64 {
        match self.timestamp.duration_since(SystemTime::UNIX_EPOCH) {
            Ok(d) => d.as_millis() as u64,
            Err(_) => 0,
        }
    }
}

/// 处理后指标的健康状态（由管道阈值阶段产生）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MetricStatus {
    Normal,
    Warning,
    Critical,
    Unknown,
}

/// 处理后的标准化指标。由管道（阶段 3）产生；
/// 定义在此处，以便存储层和协议层可以基于它构建。
#[derive(Debug, Clone)]
pub struct Metric {
    pub sensor_id: SensorId,
    pub value: f64,
    pub unit: Option<String>,
    pub status: MetricStatus,
    pub timestamp: SystemTime,
}

impl Metric {
    /// 自 UNIX_EPOCH 起的毫秒数（便于日志和快照）。
    pub fn timestamp_ms(&self) -> u64 {
        match self.timestamp.duration_since(SystemTime::UNIX_EPOCH) {
            Ok(d) => d.as_millis() as u64,
            Err(_) => 0,
        }
    }
}

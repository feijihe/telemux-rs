//! Processing pipeline: converts raw samples into processed metrics.
//!
//! A pipeline is an ordered chain of [`Stage`]s applied to a
//! [`SampleContext`]. Stages are configured via `[[pipeline]]` TOML sections
//! (see [`StageConfig`](crate::config::StageConfig)).

pub mod stages;

use std::collections::HashMap;
use std::str::FromStr;
use std::time::SystemTime;

use crate::config::{Config, PipelineConfig, StageConfig};
use crate::config_handle::ConfigHandle;
use crate::domain::{Metric, MetricStatus, RawSample, SensorId};

/// Pipeline processing error.
#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error("stage `{stage}`: {message}")]
    Stage {
        stage: &'static str,
        message: String,
    },
    #[error("expression error: {0}")]
    Math(String),
    #[error("pipeline has no stages")]
    Empty,
}

/// Mutable sample context flowing through the pipeline stages.
pub struct SampleContext {
    pub sensor_id: SensorId,
    pub name: String,
    pub value: f64,
    pub unit: Option<String>,
    pub status: MetricStatus,
    pub timestamp: SystemTime,
}

impl SampleContext {
    pub fn from_raw(sample: RawSample) -> Self {
        Self {
            sensor_id: sample.sensor_id,
            name: sample.name,
            value: sample.raw_value,
            unit: sample.unit,
            // Default to Normal: without a threshold stage there is no reason
            // to flag the metric. Threshold stages override this.
            status: MetricStatus::Normal,
            timestamp: sample.timestamp,
        }
    }

    pub fn into_metric(self) -> Metric {
        Metric {
            sensor_id: self.sensor_id,
            value: self.value,
            unit: self.unit,
            status: self.status,
            timestamp: self.timestamp,
        }
    }
}

/// A processing stage: transforms the sample context in place.
/// May hold state (filters, aggregates); `&mut self` allows that.
///
/// Deliberately **not** `Send`: pipelines run single-threaded inside the
/// acquisition consumer (a current-thread runtime), and some stages (e.g.
/// meval math expressions) are not thread-safe.
pub trait Stage {
    fn process(&mut self, ctx: &mut SampleContext) -> Result<(), PipelineError>;
}

/// A configured pipeline: one per sensor.
pub struct Pipeline {
    pub sensor_id: SensorId,
    stages: Vec<Box<dyn Stage>>,
}

impl Pipeline {
    pub fn from_config(config: &PipelineConfig) -> Result<Self, String> {
        let mut stages = Vec::with_capacity(config.stages.len());
        for sc in &config.stages {
            stages.push(build_stage(sc)?);
        }
        Ok(Self {
            sensor_id: SensorId(config.sensor_id.clone()),
            stages,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.stages.is_empty()
    }

    /// Run the chain on one raw sample, producing a metric.
    pub fn process(&mut self, sample: RawSample) -> Result<Metric, PipelineError> {
        if self.stages.is_empty() {
            return Err(PipelineError::Empty);
        }
        let mut ctx = SampleContext::from_raw(sample);
        for stage in &mut self.stages {
            stage.process(&mut ctx)?;
        }
        Ok(ctx.into_metric())
    }
}

/// Validate a stage configuration (used by config validation and building).
pub fn validate_stage(config: &StageConfig) -> Result<(), String> {
    match config {
        StageConfig::Scale { scale, .. } => {
            if !scale.is_finite() {
                return Err("scale must be finite".to_string());
            }
        }
        StageConfig::SlidingAverage { window }
        | StageConfig::Median { window }
        | StageConfig::Aggregate { window, .. } => {
            if *window == 0 {
                return Err("window must be >= 1".to_string());
            }
            if *window > 1024 {
                return Err("window too large (max 1024)".to_string());
            }
        }
        StageConfig::Math { expression } => {
            meval::Expr::from_str(expression)
                .map_err(|e| format!("invalid expression `{expression}`: {e}"))?;
        }
        StageConfig::Threshold {
            low_warning,
            high_warning,
            low_critical,
            high_critical,
        } => {
            if low_warning.is_none()
                && high_warning.is_none()
                && low_critical.is_none()
                && high_critical.is_none()
            {
                return Err("threshold needs at least one bound".to_string());
            }
            if let (Some(lo), Some(hi)) = (low_critical, high_critical) {
                if lo > hi {
                    return Err("low_critical must be <= high_critical".to_string());
                }
            }
            if let (Some(lo), Some(hi)) = (low_warning, high_warning) {
                if lo > hi {
                    return Err("low_warning must be <= high_warning".to_string());
                }
            }
        }
    }
    Ok(())
}

/// Build one stage from its configuration.
pub fn build_stage(config: &StageConfig) -> Result<Box<dyn Stage>, String> {
    validate_stage(config)?;
    Ok(match config {
        StageConfig::Scale {
            scale,
            offset,
            unit,
        } => Box::new(stages::ScaleStage::new(*scale, *offset, unit.clone())),
        StageConfig::SlidingAverage { window } => {
            Box::new(stages::SlidingAverageStage::new(*window))
        }
        StageConfig::Median { window } => Box::new(stages::MedianStage::new(*window)),
        StageConfig::Math { expression } => Box::new(stages::MathStage::new(expression)?),
        StageConfig::Threshold {
            low_warning,
            high_warning,
            low_critical,
            high_critical,
        } => Box::new(stages::ThresholdStage::new(
            *low_warning,
            *high_warning,
            *low_critical,
            *high_critical,
        )),
        StageConfig::Aggregate { window, mode } => {
            Box::new(stages::AggregateStage::new(*window, *mode))
        }
    })
}

/// Build all pipelines from configuration. Config must already be validated,
/// so a failing build here is a programming error.
pub fn build_pipelines(configs: &[PipelineConfig]) -> HashMap<SensorId, Pipeline> {
    configs
        .iter()
        .map(|c| {
            let pipeline = Pipeline::from_config(c).expect("validated pipeline builds");
            (pipeline.sensor_id.clone(), pipeline)
        })
        .collect()
}

/// Cache of built pipelines, rebuilt when the runtime config revision changes
/// (registers/pipelines added via the dev dashboard take effect immediately).
pub struct PipelinesCache {
    revision: u64,
    pipelines: HashMap<SensorId, Pipeline>,
}

impl PipelinesCache {
    pub fn new(config: &Config) -> Self {
        Self {
            revision: 0,
            pipelines: build_pipelines(&config.pipelines),
        }
    }

    /// Rebuild the pipelines if the config revision changed since the last
    /// refresh. Cheap no-op otherwise (no per-sample parsing).
    pub fn refresh(&mut self, handle: &ConfigHandle) {
        let rev = handle.revision();
        if rev != self.revision {
            let config = handle.read();
            self.pipelines = build_pipelines(&config.pipelines);
            self.revision = rev;
            tracing::debug!(
                "pipeline cache rebuilt ({} pipeline(s))",
                self.pipelines.len()
            );
        }
    }

    pub fn get_mut(&mut self, sensor_id: &SensorId) -> Option<&mut Pipeline> {
        self.pipelines.get_mut(sensor_id)
    }

    pub fn len(&self) -> usize {
        self.pipelines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pipelines.is_empty()
    }
}

/// Human-readable formula for one stage (generated from config, not handwritten).
pub fn describe_stage(config: &StageConfig) -> String {
    match config {
        StageConfig::Scale {
            scale,
            offset,
            unit,
        } => {
            let offset_part = if *offset != 0.0 {
                format!(" + {offset}")
            } else {
                String::new()
            };
            let unit_part = unit
                .as_ref()
                .map(|u| format!(" → {u}"))
                .unwrap_or_default();
            format!("v = v × {scale}{offset_part}{unit_part}")
        }
        StageConfig::SlidingAverage { window } => {
            format!("v = avg(最近 {window} 个值)")
        }
        StageConfig::Median { window } => {
            format!("v = median(最近 {window} 个值)")
        }
        StageConfig::Math { expression } => {
            format!("v = {expression}  (v 为当前值)")
        }
        StageConfig::Threshold {
            low_warning,
            high_warning,
            low_critical,
            high_critical,
        } => {
            let mut parts = Vec::new();
            if let Some(b) = low_critical {
                parts.push(format!("<{b} critical"));
            }
            if let Some(b) = low_warning {
                parts.push(format!("<{b} warning"));
            }
            if let Some(b) = high_warning {
                parts.push(format!(">{b} warning"));
            }
            if let Some(b) = high_critical {
                parts.push(format!(">{b} critical"));
            }
            format!("状态: {}", parts.join(" / "))
        }
        StageConfig::Aggregate { window, mode } => {
            let m = match mode {
                crate::config::AggregateMode::Min => "min",
                crate::config::AggregateMode::Max => "max",
                crate::config::AggregateMode::Avg => "avg",
            };
            format!("v = {m}(窗口 {window})")
        }
    }
}

/// Human-readable formula for a whole pipeline (stages joined by ` → `).
pub fn describe_pipeline(config: &PipelineConfig) -> String {
    config
        .stages
        .iter()
        .map(describe_stage)
        .collect::<Vec<_>>()
        .join(" → ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AggregateMode, StageConfig};

    fn ctx(value: f64) -> SampleContext {
        SampleContext {
            sensor_id: SensorId("test.sensor".into()),
            name: "sensor".into(),
            value,
            unit: Some("counts".into()),
            status: MetricStatus::Unknown,
            timestamp: SystemTime::now(),
        }
    }

    #[test]
    fn scale_stage() {
        let mut s = stages::ScaleStage::new(0.1, 5.0, Some("°C".into()));
        let mut c = ctx(100.0);
        s.process(&mut c).unwrap();
        assert_eq!(c.value, 15.0);
        assert_eq!(c.unit.as_deref(), Some("°C"));
    }

    #[test]
    fn sliding_average_stage() {
        let mut s = stages::SlidingAverageStage::new(3);
        for (input, expected) in [(1.0, 1.0), (2.0, 1.5), (3.0, 2.0), (10.0, 5.0)] {
            let mut c = ctx(input);
            s.process(&mut c).unwrap();
            assert!((c.value - expected).abs() < 1e-9, "got {}", c.value);
        }
    }

    #[test]
    fn median_stage_odd_and_even() {
        let mut s = stages::MedianStage::new(3);
        for (input, expected) in [(3.0, 3.0), (1.0, 2.0), (2.0, 2.0)] {
            let mut c = ctx(input);
            s.process(&mut c).unwrap();
            assert_eq!(c.value, expected);
        }
        // even window: average of two middles
        let mut s = stages::MedianStage::new(2);
        let mut c = ctx(1.0);
        s.process(&mut c).unwrap();
        let mut c = ctx(3.0);
        s.process(&mut c).unwrap();
        assert_eq!(c.value, 2.0);
    }

    #[test]
    fn math_stage() {
        let mut s = stages::MathStage::new("v * 2 + 1").unwrap();
        let mut c = ctx(10.0);
        s.process(&mut c).unwrap();
        assert_eq!(c.value, 21.0);
    }

    #[test]
    fn math_stage_rejects_bad_expression() {
        assert!(stages::MathStage::new("v +* 2").is_err());
    }

    #[test]
    fn threshold_stage_critical_beats_warning() {
        let mut s = stages::ThresholdStage::new(Some(20.0), Some(80.0), Some(10.0), Some(90.0));
        let cases = [
            (5.0, MetricStatus::Critical),
            (15.0, MetricStatus::Warning),
            (50.0, MetricStatus::Normal),
            (85.0, MetricStatus::Warning),
            (95.0, MetricStatus::Critical),
        ];
        for (value, expected) in cases {
            let mut c = ctx(value);
            s.process(&mut c).unwrap();
            assert_eq!(c.status, expected, "value {value}");
        }
    }

    #[test]
    fn threshold_stage_single_bound() {
        let mut s = stages::ThresholdStage::new(None, Some(80.0), None, None);
        let mut c = ctx(90.0);
        s.process(&mut c).unwrap();
        assert_eq!(c.status, MetricStatus::Warning);
        let mut c = ctx(50.0);
        s.process(&mut c).unwrap();
        assert_eq!(c.status, MetricStatus::Normal);
    }

    #[test]
    fn aggregate_stage_modes() {
        let mut s = stages::AggregateStage::new(3, AggregateMode::Max);
        for v in [1.0, 5.0, 3.0, 2.0] {
            let mut c = ctx(v);
            s.process(&mut c).unwrap();
        }
        // After four samples the window holds [5, 3, 2].
        let mut c = ctx(0.0);
        s.process(&mut c).unwrap();
        assert_eq!(c.value, 3.0); // max of [3, 2, 0]

        let mut s = stages::AggregateStage::new(2, AggregateMode::Avg);
        let mut c = ctx(10.0);
        s.process(&mut c).unwrap();
        assert_eq!(c.value, 10.0);
        let mut c = ctx(20.0);
        s.process(&mut c).unwrap();
        assert_eq!(c.value, 15.0);
    }

    #[test]
    fn pipeline_integration_raw_to_metric() {
        let cfg: PipelineConfig = toml::from_str(
            r#"
sensor_id = "s.cpu"
[[stages]]
type = "scale"
scale = 0.1
unit = "°C"
[[stages]]
type = "threshold"
high_warning = 25.0
high_critical = 30.0
"#,
        )
        .unwrap();
        let mut pipe = Pipeline::from_config(&cfg).unwrap();
        let metric = pipe
            .process(RawSample {
                sensor_id: SensorId("s.cpu".into()),
                name: "cpu".into(),
                raw_value: 260.0,
                unit: Some("counts".into()),
                timestamp: SystemTime::now(),
            })
            .unwrap();
        assert_eq!(metric.value, 26.0);
        assert_eq!(metric.unit.as_deref(), Some("°C"));
        assert_eq!(metric.status, MetricStatus::Warning);
    }

    #[test]
    fn validate_stage_rules() {
        assert!(validate_stage(&StageConfig::SlidingAverage { window: 0 }).is_err());
        assert!(validate_stage(&StageConfig::Median { window: 1025 }).is_err());
        assert!(validate_stage(&StageConfig::Math {
            expression: "v/".into()
        })
        .is_err());
        assert!(validate_stage(&StageConfig::Threshold {
            low_warning: None,
            high_warning: None,
            low_critical: None,
            high_critical: None
        })
        .is_err());
        assert!(validate_stage(&StageConfig::Threshold {
            low_warning: Some(90.0),
            high_warning: Some(10.0),
            low_critical: None,
            high_critical: None
        })
        .is_err());
        assert!(validate_stage(&StageConfig::Scale {
            scale: 1.0,
            offset: 0.0,
            unit: None
        })
        .is_ok());
    }

    #[test]
    fn describe_stage_formulas() {
        assert_eq!(
            describe_stage(&StageConfig::Scale {
                scale: 0.1,
                offset: 0.0,
                unit: Some("°C".into()),
            }),
            "v = v × 0.1 → °C"
        );
        assert_eq!(
            describe_stage(&StageConfig::Scale {
                scale: 0.001,
                offset: -2.0,
                unit: None,
            }),
            "v = v × 0.001 + -2"
        );
        assert_eq!(
            describe_stage(&StageConfig::SlidingAverage { window: 5 }),
            "v = avg(最近 5 个值)"
        );
        assert_eq!(
            describe_stage(&StageConfig::Math {
                expression: "(v - 273.15) * 10".into()
            }),
            "v = (v - 273.15) * 10  (v 为当前值)"
        );
        assert_eq!(
            describe_stage(&StageConfig::Threshold {
                low_warning: Some(10.0),
                high_warning: Some(30.0),
                low_critical: Some(5.0),
                high_critical: Some(35.0),
            }),
            "状态: <5 critical / <10 warning / >30 warning / >35 critical"
        );
        assert_eq!(
            describe_stage(&StageConfig::Aggregate {
                window: 4,
                mode: AggregateMode::Avg,
            }),
            "v = avg(窗口 4)"
        );
    }

    #[test]
    fn describe_pipeline_joins_stages() {
        let cfg: PipelineConfig = toml::from_str(
            r#"
sensor_id = "s.cpu"
[[stages]]
type = "scale"
scale = 0.1
unit = "°C"
[[stages]]
type = "threshold"
high_warning = 30
"#,
        )
        .unwrap();
        assert_eq!(
            describe_pipeline(&cfg),
            "v = v × 0.1 → °C → 状态: >30 warning"
        );
    }
}

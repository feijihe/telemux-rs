//! Built-in pipeline stages.

use std::collections::VecDeque;
use std::str::FromStr;

use crate::config::AggregateMode;
use crate::domain::MetricStatus;

use super::{PipelineError, SampleContext, Stage};

/// Linear conversion: `value = value * scale + offset`, optionally updating the unit.
pub struct ScaleStage {
    scale: f64,
    offset: f64,
    unit: Option<String>,
}

impl ScaleStage {
    pub fn new(scale: f64, offset: f64, unit: Option<String>) -> Self {
        Self { scale, offset, unit }
    }
}

impl Stage for ScaleStage {
    fn process(&mut self, ctx: &mut SampleContext) -> Result<(), PipelineError> {
        ctx.value = ctx.value * self.scale + self.offset;
        if let Some(unit) = &self.unit {
            ctx.unit = Some(unit.clone());
        }
        Ok(())
    }
}

/// Sliding window average filter.
pub struct SlidingAverageStage {
    window: usize,
    buffer: VecDeque<f64>,
}

impl SlidingAverageStage {
    pub fn new(window: usize) -> Self {
        Self {
            window,
            buffer: VecDeque::with_capacity(window),
        }
    }
}

impl Stage for SlidingAverageStage {
    fn process(&mut self, ctx: &mut SampleContext) -> Result<(), PipelineError> {
        self.buffer.push_back(ctx.value);
        if self.buffer.len() > self.window {
            self.buffer.pop_front();
        }
        ctx.value = self.buffer.iter().sum::<f64>() / self.buffer.len() as f64;
        Ok(())
    }
}

/// Sliding window median filter (average of the two middles for even windows).
pub struct MedianStage {
    window: usize,
    buffer: VecDeque<f64>,
}

impl MedianStage {
    pub fn new(window: usize) -> Self {
        Self {
            window,
            buffer: VecDeque::with_capacity(window),
        }
    }
}

impl Stage for MedianStage {
    fn process(&mut self, ctx: &mut SampleContext) -> Result<(), PipelineError> {
        self.buffer.push_back(ctx.value);
        if self.buffer.len() > self.window {
            self.buffer.pop_front();
        }
        let mut sorted: Vec<f64> = self.buffer.iter().copied().collect();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let mid = sorted.len() / 2;
        ctx.value = if sorted.len() % 2 == 0 {
            (sorted[mid - 1] + sorted[mid]) / 2.0
        } else {
            sorted[mid]
        };
        Ok(())
    }
}

/// Math expression over the current value; the variable is `v`
/// (e.g. `"(v - 273.15) * 10"`).
///
/// Stores the parsed [`meval::Expr`] and re-binds on every sample: meval's
/// bound closures are not `Send` (they hold an `Rc` function table), which is
/// fine since pipelines run single-threaded.
pub struct MathStage {
    expr: meval::Expr,
}

impl MathStage {
    pub fn new(expression: &str) -> Result<Self, String> {
        let expr = meval::Expr::from_str(expression)
            .map_err(|e| format!("invalid expression `{expression}`: {e}"))?;
        Ok(Self { expr })
    }
}

impl Stage for MathStage {
    fn process(&mut self, ctx: &mut SampleContext) -> Result<(), PipelineError> {
        let f = self
            .expr
            .clone()
            .bind("v")
            .map_err(|e| PipelineError::Math(e.to_string()))?;
        ctx.value = f(ctx.value);
        Ok(())
    }
}

/// Threshold check: sets the metric status. Critical bounds beat warning bounds.
pub struct ThresholdStage {
    low_warning: Option<f64>,
    high_warning: Option<f64>,
    low_critical: Option<f64>,
    high_critical: Option<f64>,
}

impl ThresholdStage {
    pub fn new(
        low_warning: Option<f64>,
        high_warning: Option<f64>,
        low_critical: Option<f64>,
        high_critical: Option<f64>,
    ) -> Self {
        Self {
            low_warning,
            high_warning,
            low_critical,
            high_critical,
        }
    }
}

impl Stage for ThresholdStage {
    fn process(&mut self, ctx: &mut SampleContext) -> Result<(), PipelineError> {
        let v = ctx.value;
        let mut status = MetricStatus::Normal;
        if let Some(bound) = self.low_critical {
            if v < bound {
                status = MetricStatus::Critical;
            }
        }
        if let Some(bound) = self.high_critical {
            if v > bound {
                status = MetricStatus::Critical;
            }
        }
        if status != MetricStatus::Critical {
            if let Some(bound) = self.low_warning {
                if v < bound {
                    status = MetricStatus::Warning;
                }
            }
            if let Some(bound) = self.high_warning {
                if v > bound {
                    status = MetricStatus::Warning;
                }
            }
        }
        ctx.status = status;
        Ok(())
    }
}

/// Windowed statistics (min / max / avg), replacing the value with the window stat.
pub struct AggregateStage {
    window: usize,
    mode: AggregateMode,
    buffer: VecDeque<f64>,
}

impl AggregateStage {
    pub fn new(window: usize, mode: AggregateMode) -> Self {
        Self {
            window,
            mode,
            buffer: VecDeque::with_capacity(window),
        }
    }
}

impl Stage for AggregateStage {
    fn process(&mut self, ctx: &mut SampleContext) -> Result<(), PipelineError> {
        self.buffer.push_back(ctx.value);
        if self.buffer.len() > self.window {
            self.buffer.pop_front();
        }
        ctx.value = match self.mode {
            AggregateMode::Min => self
                .buffer
                .iter()
                .copied()
                .fold(f64::INFINITY, f64::min),
            AggregateMode::Max => self
                .buffer
                .iter()
                .copied()
                .fold(f64::NEG_INFINITY, f64::max),
            AggregateMode::Avg => self.buffer.iter().sum::<f64>() / self.buffer.len() as f64,
        };
        Ok(())
    }
}

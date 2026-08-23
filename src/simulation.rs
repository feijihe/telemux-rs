//! CDU 仿真数据源（阶段 8 扩展）：从 `[sim]` 配置用**稳态代数模型**计算
//! 传感器值，无需真实硬件。复用现有采集/管道/协议链路。
//!
//! ## 模型
//!
//! - **控制变量**（`[[sim.controls]]`）：泵/阀/风扇 duty 等，可被协议层
//!   写入以驱动仿真（预留写接口）。
//! - **仿真传感器**（`[[sim.sensors]]`）：每个由 `formula` 表达式求值，
//!   可引用控制变量名与其他仿真传感器 id（按依赖递归求值，一轮收敛）。
//!
//! 表达式求值用 meval（同 computed/管道）。公式变量 = 控制变量名 ∪ 全部
//! 仿真传感器 id；引用未知名字时 `bindn` 报错 → 降级为 `None` 跳过该传感器。

use std::collections::HashMap;
use std::str::FromStr;

use async_trait::async_trait;
use tracing::warn;

use crate::acquisition::{AcquisitionError, SensorSource};
use crate::config::{RegisterConfig, SimConfig};
use crate::domain::RawSample;

/// 永不出错的 meval 求值闭包所需的最小变量全集；见 [`eval_vars`]。
/// meval 没有 `variables()`，这里用简单分词提取公式里的标识符，
/// 过滤掉内置函数名，得到引用集合。
fn formula_refs(expr: &str) -> Vec<&str> {
    const BUILTIN: &[&str] = &[
        "ln", "log", "exp", "sqrt", "abs", "sin", "cos", "tan", "asin", "acos", "atan", "sinh",
        "cosh", "tanh", "floor", "ceil", "round", "min", "max", "pow", "pi", "e", "atan2", "sign",
        "fmod",
    ];
    let bytes = expr.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < bytes.len()
                && ((bytes[i] as char).is_ascii_alphanumeric() || bytes[i] == b'_')
            {
                i += 1;
            }
            let name = &expr[start..i];
            if !BUILTIN.contains(&name) {
                out.push(name);
            }
        } else {
            i += 1;
        }
    }
    out
}

/// 仿真数据源：从 `[sim]` 计算每个寄存器的值。
pub struct SimSource {
    sim: SimConfig,
    /// 当前控制变量值（可被写入更新，预留写接口）。
    controls: HashMap<String, f64>,
    /// 预编译的传感器表达式，按 sensor_id 索引。
    exprs: HashMap<String, meval::Expr>,
    /// 启动时刻（内置变量 `t` 的零点）。
    start: std::time::Instant,
}

impl SimSource {
    /// 从配置构建仿真数据源。
    pub fn new(sim: SimConfig) -> Self {
        let controls = sim
            .controls
            .iter()
            .map(|c| (c.name.clone(), c.initial))
            .collect();
        let exprs = sim
            .sensors
            .iter()
            .filter_map(|s| {
                meval::Expr::from_str(&s.formula)
                    .ok()
                    .map(|e| (s.sensor_id.clone(), e))
            })
            .collect();
        Self {
            sim,
            controls,
            exprs,
            start: std::time::Instant::now(),
        }
    }

    /// 求值一个传感器（递归解析其依赖）。`memo` 缓存本轮已算值，
    /// `visiting` 用于环检测（配置已防环，这里防御）。返回 `None` 表示
    /// 求值失败（未知变量 / 引用缺失 / 传递环）。
    fn eval(&self, sensor_id: &str, memo: &mut HashMap<String, f64>, visiting: &mut Vec<String>) -> Option<f64> {
        if let Some(v) = memo.get(sensor_id) {
            return Some(*v);
        }
        if visiting.iter().any(|s| s == sensor_id) {
            warn!("sim: cyclic dependency at `{sensor_id}`");
            return None;
        }
        let sensor = self.sim.sensors.iter().find(|s| s.sensor_id == sensor_id)?;
        let expr = self.exprs.get(sensor_id)?;

        visiting.push(sensor_id.to_string());
        // 递归求值依赖（表达式中引用到的名字）。变量解析优先级：
        // inputs 映射 → 控制变量 → 内置时间 t → 其它传感器短名。
        let mut vars: Vec<&str> = Vec::new();
        let mut values: Vec<f64> = Vec::new();
        for name in formula_refs(&sensor.formula) {
            // 1) inputs 映射：变量名 → 传感器 id 或控制变量名。
            if let Some(target) = sensor.inputs.get(name) {
                let v = self.value_of(target, memo, visiting)?;
                if !vars.contains(&name) {
                    vars.push(name);
                    values.push(v);
                }
                continue;
            }
            // 2) 控制变量。
            if let Some(c) = self.sim.controls.iter().find(|c| c.name == name) {
                let v = self.controls.get(&c.name).copied().unwrap_or(c.initial);
                if !vars.contains(&c.name.as_str()) {
                    vars.push(c.name.as_str());
                    values.push(v);
                }
                continue;
            }
            // 3) 内置时间 t（自启动秒数）。
            if name == "t" {
                let v = self.elapsed_secs();
                if !vars.contains(&"t") {
                    vars.push("t");
                    values.push(v);
                }
                continue;
            }
            // 4) 其它传感器短名（末段）。
            if let Some(ref_id) = self.resolve_sensor(name) {
                let v = self.eval(&ref_id, memo, visiting)?;
                if !vars.contains(&name) {
                    vars.push(name);
                    values.push(v);
                }
            }
            // 其它名字（meval 内置函数/未知）：忽略，bindn 会兜底报错。
        }
        visiting.pop();

        let f = expr.clone().bindn(&vars).ok()?;
        let v = f(&values);
        if v.is_finite() {
            memo.insert(sensor_id.to_string(), v);
            Some(v)
        } else {
            None
        }
    }

    /// 解析 `inputs` 目标或控制变量名对应的值；引用传感器则递归求值。
    fn value_of(&self, target: &str, memo: &mut HashMap<String, f64>, visiting: &mut Vec<String>) -> Option<f64> {
        if let Some(c) = self.sim.controls.iter().find(|c| c.name == target) {
            return Some(self.controls.get(&c.name).copied().unwrap_or(c.initial));
        }
        if target == "t" {
            return Some(self.elapsed_secs());
        }
        if self.sim.sensors.iter().any(|s| s.sensor_id == target) {
            return self.eval(target, memo, visiting);
        }
        warn!("sim: inputs references unknown target `{target}`");
        None
    }

    /// 把公式里引用的名字解析为仿真传感器短名（sensor_id 末段）。
    fn resolve_sensor(&self, name: &str) -> Option<String> {
        self.sim
            .sensors
            .iter()
            .find(|s| short_name(&s.sensor_id) == name)
            .map(|s| s.sensor_id.clone())
    }

    /// 自启动以来的秒数（内置变量 `t`）。
    fn elapsed_secs(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }
}

/// 取 sensor_id 的末段（`cdu.f2_flow` -> `f2_flow`），供公式短名引用。
fn short_name(sensor_id: &str) -> &str {
    sensor_id.rsplit('.').next().unwrap_or(sensor_id)
}

#[async_trait]
impl SensorSource for SimSource {
    async fn read_samples(
        &mut self,
        registers: &[RegisterConfig],
    ) -> Result<Vec<RawSample>, AcquisitionError> {
        let mut samples = Vec::with_capacity(registers.len());
        let mut memo: HashMap<String, f64> = HashMap::new();
        let mut visiting: Vec<String> = Vec::new();
        // 未显式配置寄存器时，自动产出全部仿真传感器（设备无需重复声明）。
        let sensors: Vec<&crate::config::SimSensor> = if registers.is_empty() {
            self.sim.sensors.iter().collect()
        } else {
            registers
                .iter()
                .filter_map(|r| self.sim.sensors.iter().find(|s| s.sensor_id == r.sensor_id))
                .collect()
        };
        for sensor in sensors {
            let value = match self.eval(&sensor.sensor_id, &mut memo, &mut visiting) {
                Some(v) => v,
                None => {
                    warn!("sim: no value for sensor `{}`", sensor.sensor_id);
                    continue;
                }
            };
            samples.push(RawSample {
                sensor_id: crate::domain::SensorId(sensor.sensor_id.clone()),
                name: sensor.name.clone(),
                raw_value: value,
                unit: sensor.unit.clone(),
                timestamp: RawSample::now(),
            });
        }
        Ok(samples)
    }

    /// 写接口（预留）：若 sensor_id 匹配可写控制变量，则更新其值。
    async fn write_holding_register(
        &mut self,
        sensor_id: &str,
        value: u16,
    ) -> Result<(), AcquisitionError> {
        // 约定：写保持寄存器 = 更新同名控制变量（duty 用整数 0-100）。
        if let Some(c) = self.sim.controls.iter().find(|c| c.name == sensor_id) {
            if !c.writable {
                return Err(AcquisitionError::Config {
                    device: "sim".to_string(),
                    message: format!("control `{sensor_id}` is read-only"),
                });
            }
            self.controls.insert(sensor_id.to_string(), value as f64);
            Ok(())
        } else {
            Err(AcquisitionError::Config {
                device: "sim".to_string(),
                message: format!("unknown writable control `{sensor_id}`"),
            })
        }
    }

    /// 写接口（预留）：coil 写 —— sim 无 coil，按不可写处理。
    async fn write_single_coil(
        &mut self,
        sensor_id: &str,
        _value: bool,
    ) -> Result<(), AcquisitionError> {
        Err(AcquisitionError::Config {
            device: "sim".to_string(),
            message: format!("sim has no writable coil `{sensor_id}`"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{SimControl, SimSensor};

    fn sim_config() -> SimConfig {
        SimConfig {
            controls: vec![
                SimControl {
                    name: "pump1_duty".into(),
                    initial: 50.0,
                    unit: Some("%".into()),
                    writable: true,
                },
                SimControl {
                    name: "valve1_duty".into(),
                    initial: 40.0,
                    unit: Some("%".into()),
                    writable: true,
                },
            ],
            sensors: vec![
                SimSensor {
                    sensor_id: "cdu.f2_flow".into(),
                    name: "F2 Flow".into(),
                    kind: "flow".into(),
                    unit: Some("L/min".into()),
                    formula: "pump1_duty * 2".into(),
                    inputs: Default::default(),
                },
                SimSensor {
                    sensor_id: "cdu.f1_flow".into(),
                    name: "F1 Flow".into(),
                    kind: "flow".into(),
                    unit: Some("L/min".into()),
                    formula: "valve1_duty * 1.5".into(),
                    inputs: Default::default(),
                },
                // 链式：T2 依赖 F1（一次侧冷水→二次侧降温）
                SimSensor {
                    sensor_id: "cdu.t2_out".into(),
                    name: "T2 Out".into(),
                    kind: "temperature".into(),
                    unit: Some("°C".into()),
                    formula: "45 - f1_flow * 0.1".into(),
                    inputs: [("f1_flow".to_string(), "cdu.f1_flow".to_string())].into(),
                },
            ],
        }
    }

    fn reg(id: &str) -> RegisterConfig {
        RegisterConfig {
            name: id.into(),
            sensor_id: id.into(),
            function: crate::config::RegisterFunction::Input,
            address: 0,
            count: Some(1),
            value_type: crate::config::ValueType::U16,
            word_order: crate::config::WordOrder::Big,
            unit: None,
            access: crate::config::Access::Read,
        }
    }

    #[tokio::test]
    async fn evaluates_all_sensors_with_control_dependencies() {
        let mut source = SimSource::new(sim_config());
        let regs = vec![
            reg("cdu.f2_flow"),
            reg("cdu.f1_flow"),
            reg("cdu.t2_out"),
        ];
        let samples = source.read_samples(&regs).await.unwrap();
        assert_eq!(samples.len(), 3);
        let by_id: HashMap<_, _> = samples
            .iter()
            .map(|s| (s.sensor_id.as_str(), s.raw_value))
            .collect();
        assert_eq!(by_id["cdu.f2_flow"], 100.0); // 50 * 2
        assert_eq!(by_id["cdu.f1_flow"], 60.0); // 40 * 1.5
        assert_eq!(by_id["cdu.t2_out"], 39.0); // 45 - 60*0.1
    }

    #[tokio::test]
    async fn write_updates_a_control_variable() {
        let mut source = SimSource::new(sim_config());
        // 写入 pump1_duty=80 -> f2_flow = 160。
        source.write_holding_register("pump1_duty", 80).await.unwrap();
        let samples = source
            .read_samples(&[reg("cdu.f2_flow")])
            .await
            .unwrap();
        assert_eq!(samples[0].raw_value, 160.0);
    }

    #[tokio::test]
    async fn unknown_control_write_is_error() {
        let mut source = SimSource::new(sim_config());
        let err = source.write_holding_register("nope", 1).await.unwrap_err();
        assert!(err.to_string().contains("unknown writable control"));
    }

    #[tokio::test]
    async fn formula_with_unknown_var_skips_sensor() {
        let mut sim = sim_config();
        sim.sensors[0].formula = "pump1_duty + gibberish".into();
        let mut source = SimSource::new(sim);
        let samples = source.read_samples(&[reg("cdu.f2_flow")]).await.unwrap();
        assert!(samples.is_empty(), "unknown var -> bindn error -> skipped");
    }
}

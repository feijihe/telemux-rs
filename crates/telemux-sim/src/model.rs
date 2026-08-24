//! CDU 物理模型：`SimConfig` 定义 + 稳态代数求值。
//!
//! 从 Telmux-rs 网关的 `simulation.rs` 迁移而来（workspace 拆分后独立运行）。
//! 模型本身与协议无关：输入控制变量，输出每个传感器的稳态值。

use std::collections::HashMap;
use std::path::Path;
use std::str::FromStr;

use anyhow::Context;
use serde::{Deserialize, Serialize};

/// CDU 仿真建模配置（`[sim]`）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SimConfig {
    /// 控制变量（泵/阀/风扇 duty 等），可被 Modbus 写入以驱动仿真。
    #[serde(default)]
    pub controls: Vec<SimControl>,
    /// 未归组传感器（水箱/环境/泄漏等，即 `[[sim.sensors]]`）。
    #[serde(default)]
    pub sensors: Vec<SimSensor>,
    /// 一次侧（冷水回路）传感器组（`[sim.pri]`，含 in/out/aux）。
    #[serde(default)]
    pub pri: Option<Side>,
    /// 二次侧（热水回路）传感器组（`[sim.sec]`，含 in/out/aux）。
    #[serde(default)]
    pub sec: Option<Side>,
}

/// 回路侧（一次/二次）传感器组：按出入口 + 辅助三组划分。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Side {
    /// 入口传感器（`[[sim.pri.in]]` 等）。
    #[serde(default, rename = "in")]
    pub input: Vec<SimSensor>,
    /// 出口传感器（`[[sim.pri.out]]` 等）。
    #[serde(default, rename = "out")]
    pub output: Vec<SimSensor>,
    /// 辅助传感器——非入口也非出口（`[[sim.pri.aux]]` 等）。
    #[serde(default, rename = "aux")]
    pub auxiliary: Vec<SimSensor>,
}

/// 仿真控制变量（如 pump1_duty，0-100）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimControl {
    /// 控制变量名，供传感器 formula 引用（如 "pump1_duty"）。
    pub name: String,
    /// 初始值。
    #[serde(default)]
    pub initial: f64,
    /// 单位（信息性）。
    #[serde(default)]
    pub unit: Option<String>,
    /// 是否可写（Modbus 写保持寄存器可改变 duty）。
    #[serde(default)]
    pub writable: bool,
}

/// 仿真传感器：由稳态表达式求值，作为测量值产出。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimSensor {
    /// 唯一标识（如 "cdu.sec.pump1.speed"）。
    pub sensor_id: String,
    /// 显示名称。
    pub name: String,
    /// 传感器类型：pressure/temperature/flow/level/ph/leak/...
    #[serde(default)]
    pub kind: String,
    /// 物理单位。
    #[serde(default)]
    pub unit: Option<String>,
    /// 稳态求值表达式（meval 语法）。
    ///
    /// 变量解析规则（优先级从高到低）：
    /// 1. `inputs` 映射的键 → 引用的仿真传感器（或控制变量）；
    /// 2. 控制变量名（如 `pump1_duty`）；
    /// 3. 内置时间 `t`（自启动秒数，用于模拟缓慢波动）；
    /// 4. 其他仿真传感器的短名（sensor_id 末段，如 `cdu.pri.p1` → `p1`）。
    ///
    /// 注：meval 变量名不能含 `.`，故跨传感器引用推荐用 `inputs` 显式映射。
    pub formula: String,
    /// 变量名 → 引用的传感器/控制变量（可选，显式依赖映射）。
    #[serde(default)]
    pub inputs: std::collections::HashMap<String, String>,
}

impl SimConfig {
    /// 惰性迭代全部传感器（一次侧 in/out/aux → 二次侧 in/out/aux → 未分组）。
    ///
    /// 该顺序与配置逻辑一致，从而寄存器地址按组连续划分。
    pub fn iter_sensors(&self) -> impl Iterator<Item = &SimSensor> {
        fn side_sensors(side: &Side) -> impl Iterator<Item = &SimSensor> {
            side.input
                .iter()
                .chain(side.output.iter())
                .chain(side.auxiliary.iter())
        }
        self.pri
            .iter()
            .flat_map(side_sensors)
            .chain(self.sec.iter().flat_map(side_sensors))
            .chain(self.sensors.iter())
    }

    /// 从 TOML 文件加载并校验。文件顶层为 `[sim]` 表（与网关原配置一致）。
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("read sim config `{}`", path.display()))?;
        #[derive(Deserialize)]
        struct File {
            #[serde(default)]
            sim: SimConfig,
        }
        let file: File = toml::from_str(&text)
            .with_context(|| format!("parse sim config `{}`", path.display()))?;
        let cfg = file.sim;
        cfg.validate()?;
        Ok(cfg)
    }

    /// 校验：控制变量名唯一、传感器 id 唯一、表达式可解析、inputs 目标存在。
    pub fn validate(&self) -> anyhow::Result<()> {
        let mut controls: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for c in &self.controls {
            if !controls.insert(c.name.as_str()) {
                anyhow::bail!("duplicate sim control `{}`", c.name);
            }
        }
        let mut ids: std::collections::HashSet<&str> = std::collections::HashSet::new();
        // 第一遍：收集全部传感器 id（分组顺序可能使依赖方先于被依赖方出现）。
        for s in self.iter_sensors() {
            if !ids.insert(s.sensor_id.as_str()) {
                anyhow::bail!("duplicate sim sensor_id `{}`", s.sensor_id);
            }
        }
        // 第二遍：校验 name/formula/inputs 目标。
        for s in self.iter_sensors() {
            if s.name.is_empty() {
                anyhow::bail!("sim sensor `{}`: name is required", s.sensor_id);
            }
            meval::Expr::from_str(&s.formula).map_err(|e| {
                anyhow::anyhow!(
                    "sim sensor `{}`: invalid formula `{}`: {e}",
                    s.sensor_id,
                    s.formula
                )
            })?;
            for (var, target) in &s.inputs {
                if !controls.contains(target.as_str()) && !ids.contains(target.as_str()) {
                    anyhow::bail!(
                        "sim sensor `{}`: input `{var}` references unknown target `{target}`",
                        s.sensor_id
                    );
                }
            }
        }
        Ok(())
    }
}

/// 仿真状态：控制变量当前值 + 求值引擎。
pub struct SimEngine {
    sim: SimConfig,
    /// 当前控制变量值。
    controls: HashMap<String, f64>,
    /// 预编译表达式，按 sensor_id 索引。
    exprs: HashMap<String, meval::Expr>,
    /// 启动时刻（内置变量 `t` 的零点）。
    start: std::time::Instant,
}

impl SimEngine {
    pub fn new(sim: SimConfig) -> Self {
        let controls = sim
            .controls
            .iter()
            .map(|c| (c.name.clone(), c.initial))
            .collect();
        let exprs = sim
            .iter_sensors()
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

    pub fn config(&self) -> &SimConfig {
        &self.sim
    }

    /// 读取控制变量当前值（如无则用 initial）。
    pub fn control(&self, name: &str) -> Option<f64> {
        let c = self.sim.controls.iter().find(|c| c.name == name)?;
        Some(self.controls.get(&c.name).copied().unwrap_or(c.initial))
    }

    /// 设置控制变量（Modbus 写入口）。仅允许 `writable` 的控制变量。
    pub fn set_control(&mut self, name: &str, value: f64) -> anyhow::Result<()> {
        let c = self
            .sim
            .controls
            .iter()
            .find(|c| c.name == name)
            .ok_or_else(|| anyhow::anyhow!("unknown control `{name}`"))?;
        if !c.writable {
            anyhow::bail!("control `{name}` is read-only");
        }
        self.controls.insert(name.to_string(), value);
        Ok(())
    }

    /// 求值一个传感器（递归解析依赖）。`memo` 缓存本轮已算值，
    /// `visiting` 用于环检测（配置已防环，防御）。返回 `None` 表示失败。
    fn eval(
        &self,
        sensor_id: &str,
        memo: &mut HashMap<String, f64>,
        visiting: &mut Vec<String>,
    ) -> Option<f64> {
        if let Some(v) = memo.get(sensor_id) {
            return Some(*v);
        }
        if visiting.iter().any(|s| s == sensor_id) {
            tracing::warn!("sim: cyclic dependency at `{sensor_id}`");
            return None;
        }
        let sensor = self.sim.iter_sensors().find(|s| s.sensor_id == sensor_id)?;
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
                let v = self.start.elapsed().as_secs_f64();
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
    fn value_of(
        &self,
        target: &str,
        memo: &mut HashMap<String, f64>,
        visiting: &mut Vec<String>,
    ) -> Option<f64> {
        if let Some(c) = self.sim.controls.iter().find(|c| c.name == target) {
            return Some(self.controls.get(&c.name).copied().unwrap_or(c.initial));
        }
        if target == "t" {
            return Some(self.start.elapsed().as_secs_f64());
        }
        if self.sim.iter_sensors().any(|s| s.sensor_id == target) {
            return self.eval(target, memo, visiting);
        }
        tracing::warn!("sim: inputs references unknown target `{target}`");
        None
    }

    /// 把公式里引用的名字解析为仿真传感器短名（sensor_id 末段）。
    fn resolve_sensor(&self, name: &str) -> Option<String> {
        self.sim
            .iter_sensors()
            .find(|s| short_name(&s.sensor_id) == name)
            .map(|s| s.sensor_id.clone())
    }

    /// 求值全部传感器，返回 (sensor_id, value)。
    pub fn eval_all(&self) -> Vec<(String, f64)> {
        let mut memo: HashMap<String, f64> = HashMap::new();
        let mut visiting: Vec<String> = Vec::new();
        self.sim
            .iter_sensors()
            .filter_map(|s| {
                let v = self.eval(&s.sensor_id, &mut memo, &mut visiting)?;
                Some((s.sensor_id.clone(), v))
            })
            .collect()
    }
}

/// 取 sensor_id 的末段（`cdu.f2_flow` -> `f2_flow`），供公式短名引用。
fn short_name(sensor_id: &str) -> &str {
    sensor_id.rsplit('.').next().unwrap_or(sensor_id)
}

/// 从公式文本提取标识符（变量名），过滤 meval 内置函数名。
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

#[cfg(test)]
mod tests {
    use super::*;

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
                SimSensor {
                    sensor_id: "cdu.t2_out".into(),
                    name: "T2 Out".into(),
                    kind: "temperature".into(),
                    unit: Some("°C".into()),
                    formula: "45 - f1_flow * 0.1".into(),
                    inputs: [("f1_flow".to_string(), "cdu.f1_flow".to_string())].into(),
                },
            ],
            pri: None,
            sec: None,
        }
    }

    #[test]
    fn evaluates_all_sensors() {
        let engine = SimEngine::new(sim_config());
        let all = engine.eval_all();
        let by_id: HashMap<_, _> = all.into_iter().collect();
        assert_eq!(by_id["cdu.f2_flow"], 100.0); // 50*2
        assert_eq!(by_id["cdu.f1_flow"], 60.0); // 40*1.5
        assert_eq!(by_id["cdu.t2_out"], 39.0); // 45 - 60*0.1
    }

    #[test]
    fn set_control_changes_values() {
        let mut engine = SimEngine::new(sim_config());
        engine.set_control("pump1_duty", 80.0).unwrap();
        let all = engine.eval_all();
        let by_id: HashMap<_, _> = all.into_iter().collect();
        assert_eq!(by_id["cdu.f2_flow"], 160.0);
    }

    #[test]
    fn read_only_control_rejects_write() {
        let mut sim = sim_config();
        sim.controls[0].writable = false;
        let mut engine = SimEngine::new(sim);
        assert!(engine.set_control("pump1_duty", 80.0).is_err());
    }

    #[test]
    fn validate_rejects_unknown_input() {
        let mut sim = sim_config();
        sim.sensors[0].inputs = [("x".to_string(), "s.nope".to_string())].into();
        assert!(sim.validate().is_err());
    }

    #[test]
    fn loads_grouped_cdu_config() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("config/cdu.toml");
        let cfg = SimConfig::load(&path).expect("load cdu.toml");
        let pri = cfg.pri.as_ref().expect("pri present");
        let sec = cfg.sec.as_ref().expect("sec present");
        // in/out/aux 三组均有传感器。
        assert!(!pri.input.is_empty() && !pri.output.is_empty() && !pri.auxiliary.is_empty());
        assert!(!sec.input.is_empty() && !sec.output.is_empty() && !sec.auxiliary.is_empty());
        assert!(!cfg.sensors.is_empty()); // 水箱/环境/泄漏等全局未分组
        // 全量求值无环无错。
        let engine = SimEngine::new(cfg);
        let all = engine.eval_all();
        assert_eq!(all.len(), engine.config().iter_sensors().count());
    }
}

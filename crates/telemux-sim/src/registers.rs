//! Modbus 寄存器地图：把仿真模型映射为从站暴露的地址空间。
//!
//! 契约（`docs/SIMULATION.md`）：
//! - 保持寄存器 `0x0000` 起：可写控制变量（u16，0-100）+ 配置为 `area="holding"`
//!   的只读传感器槽位（对齐真实 CDU 中 `read_holding_registers` 的测量点）；
//! - 输入寄存器 `0x0000` 起：传感器测量值（默认 `area="input"`），只读。
//!
//! 地址分配：默认**确定性 append**（按配置顺序，与网关侧 Modbus Server 的
//! 分配风格一致）；配置了显式 `address` 的项按指定地址放置（稀疏填充）。
//! 传感器存储格式二选一：
//! - `storage = "f32"`（默认）：双字 Big 字序，物理值直存；
//! - `storage = "u16"`：单字，物理值经 `encode` 编码为原始整数（模拟真实
//!   CDU 的原始寄存器 + 网关侧解码公式，如 T = raw/10）。

use std::collections::HashMap;

use crate::model::{Area, SimConfig, SimEngine, Storage};

/// 保持寄存器槽位：控制变量或只读传感器。
#[derive(Debug, Clone)]
pub enum HoldingSlot {
    /// 可写控制变量。
    Control {
        /// 控制变量名。
        control: String,
        /// 是否可写。
        writable: bool,
    },
    /// 只读传感器（对齐真实 CDU 保持区的测量点）。
    Sensor(InputSlot),
}

/// 输入寄存器槽位：传感器。
#[derive(Debug, Clone)]
pub struct InputSlot {
    /// 传感器 id。
    pub sensor_id: String,
    /// 存储格式（f32 双字 / u16 单字）。
    pub storage: Storage,
    /// u16 模式下物理值 → 原始整数的编码表达式（变量 `v`）。
    pub encode: Option<String>,
}

/// 寄存器地图：地址 = Vec 下标（显式地址时稀疏填充 None）。
#[derive(Debug, Clone, Default)]
pub struct RegisterMap {
    /// 保持区（控制变量 + holding 传感器）。
    pub holding: Vec<Option<HoldingSlot>>,
    /// 输入区（传感器）。
    pub inputs: Vec<Option<InputSlot>>,
    /// 线圈区（布尔量传感器，功能码 0x01）。
    pub coils: Vec<Option<InputSlot>>,
}

impl RegisterMap {
    /// 从配置构建地图。控制变量按显式地址或配置顺序进保持区；
    /// 传感器按 `area` 进保持区（只读）、输入区或线圈区，支持显式地址
    /// （f32 占 2 字，u16/线圈占 1 字）。
    pub fn build(sim: &SimConfig) -> Self {
        let mut map = RegisterMap::default();
        // 保持区：先放显式地址（按配置顺序，稀疏填充），再紧凑追加无地址项。
        let mut next_holding = 0usize;
        for c in &sim.controls {
            let addr = c.address.map(|a| a as usize).unwrap_or(next_holding);
            while map.holding.len() <= addr {
                map.holding.push(None);
            }
            map.holding[addr] = Some(HoldingSlot::Control {
                control: c.name.clone(),
                writable: c.writable,
            });
            next_holding = addr + 1;
        }
        // 传感器：按 area 分到保持区、输入区或线圈区。
        let mut next_input = 0usize;
        let mut next_coils = 0usize;
        for s in sim.iter_sensors() {
            let width = match s.storage {
                Storage::F32 => 2usize,
                Storage::U16 => 1usize,
            };
            let (vec, next): (&mut Vec<Option<HoldingSlot>>, &mut usize) = match s.area {
                Area::Holding => (&mut map.holding, &mut next_holding),
                _ => continue, // 输入区/线圈区在下方单独处理
            };
            let addr = s.address.map(|a| a as usize).unwrap_or(*next);
            let end = addr + width;
            while vec.len() < end {
                vec.push(None);
            }
            let slot = InputSlot {
                sensor_id: s.sensor_id.clone(),
                storage: s.storage,
                encode: s.encode.clone(),
            };
            vec[addr] = Some(HoldingSlot::Sensor(slot.clone()));
            if width == 2 {
                vec[addr + 1] = Some(HoldingSlot::Sensor(slot));
            }
            *next = addr + width;
        }
        for s in sim.iter_sensors() {
            if s.area != Area::Input {
                continue;
            }
            let width = match s.storage {
                Storage::F32 => 2usize,
                Storage::U16 => 1usize,
            };
            let addr = s.address.map(|a| a as usize).unwrap_or(next_input);
            let end = addr + width;
            while map.inputs.len() < end {
                map.inputs.push(None);
            }
            let slot = InputSlot {
                sensor_id: s.sensor_id.clone(),
                storage: s.storage,
                encode: s.encode.clone(),
            };
            map.inputs[addr] = Some(slot.clone());
            if width == 2 {
                map.inputs[addr + 1] = Some(slot);
            }
            next_input = addr + width;
        }
        for s in sim.iter_sensors() {
            if s.area != Area::Coils {
                continue;
            }
            let addr = s.address.map(|a| a as usize).unwrap_or(next_coils);
            while map.coils.len() <= addr {
                map.coils.push(None);
            }
            map.coils[addr] = Some(InputSlot {
                sensor_id: s.sensor_id.clone(),
                storage: s.storage,
                encode: s.encode.clone(),
            });
            next_coils = addr + 1;
        }
        map
    }

    /// 读取保持寄存器：控制变量取整 u16；holding 传感器按 storage 编码。
    pub fn read_holding(&self, engine: &SimEngine, addr: usize) -> u16 {
        match self.holding.get(addr).and_then(|s| s.as_ref()) {
            Some(HoldingSlot::Control { control, .. }) => {
                let v = engine.control(control).unwrap_or(0.0);
                // duty 0-100：u16 直存；超出范围截断。
                v.round().clamp(0.0, u16::MAX as f64) as u16
            }
            Some(HoldingSlot::Sensor(slot)) => read_sensor(engine, slot, addr),
            None => 0,
        }
    }

    /// 读取输入寄存器：按 storage 编码。
    pub fn read_input(&self, engine: &SimEngine, addr: usize) -> u16 {
        match self.inputs.get(addr).and_then(|s| s.as_ref()) {
            Some(slot) => read_sensor(engine, slot, addr),
            None => 0,
        }
    }

    /// 读取线圈（布尔量）：物理值非零为 ON。
    pub fn read_coil(&self, engine: &SimEngine, addr: usize) -> bool {
        let slot = match self.coils.get(addr).and_then(|s| s.as_ref()) {
            Some(s) => s,
            None => return false,
        };
        let values: HashMap<String, f64> = engine.eval_all().into_iter().collect();
        let v = values.get(&slot.sensor_id).copied().unwrap_or(0.0);
        v.abs() > 0.5
    }
}

/// 读取传感器槽位。f32 双字：按物理值位拆高/低字；u16 单字：编码原始整数。
fn read_sensor(engine: &SimEngine, slot: &InputSlot, addr: usize) -> u16 {
    match slot.storage {
        Storage::F32 => {
            let values: HashMap<String, f64> = engine.eval_all().into_iter().collect();
            let v = values.get(&slot.sensor_id).copied().unwrap_or(f64::NAN);
            let bits = (v as f32).to_bits();
            if addr.is_multiple_of(2) {
                (bits >> 16) as u16 // 高字
            } else {
                (bits & 0xFFFF) as u16 // 低字
            }
        }
        Storage::U16 => {
            let values: HashMap<String, f64> = engine.eval_all().into_iter().collect();
            let v = values.get(&slot.sensor_id).copied().unwrap_or(f64::NAN);
            let raw = encode_raw(&slot.encode, v);
            (raw.round().clamp(0.0, u16::MAX as f64)) as u16
        }
    }
}

/// 用编码表达式把物理值转为原始整数（变量 `v`）。无表达式则恒等取整。
fn encode_raw(encode: &Option<String>, v: f64) -> f64 {
    match encode {
        Some(expr) => {
            if let Ok(f) = expr.parse::<meval::Expr>().and_then(|e| e.bind("v")) {
                return f(v);
            }
            v
        }
        None => v,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{SimControl, SimSensor};

    fn sim_config() -> SimConfig {
        SimConfig {
            controls: vec![SimControl {
                name: "pump1_duty".into(),
                initial: 50.0,
                unit: Some("%".into()),
                writable: true,
                address: None,
            }],
            sensors: vec![SimSensor {
                sensor_id: "cdu.pri.p1".into(),
                name: "P1".into(),
                kind: "pressure".into(),
                unit: Some("kPa".into()),
                formula: "300 + pump1_duty * 1.2".into(), // 360
                inputs: Default::default(),
                address: None,
                area: Area::Input,
                storage: Storage::F32,
                encode: None,
            }],
            pri: None,
            sec: None,
        }
    }

    #[test]
    fn map_lays_out_addresses() {
        let map = RegisterMap::build(&sim_config());
        assert_eq!(map.holding.len(), 1);
        assert!(matches!(
            &map.holding[0],
            Some(HoldingSlot::Control { control, .. }) if control == "pump1_duty"
        ));
        // f32 占 2 字
        assert_eq!(map.inputs.len(), 2);
    }

    #[test]
    fn reads_holding_and_input() {
        let map = RegisterMap::build(&sim_config());
        let engine = SimEngine::new(sim_config());
        assert_eq!(map.read_holding(&engine, 0), 50);
        // p1 = 300 + 50*1.2 = 360.0 = 0x43B40000 -> 高字 0x43B4
        let hi = map.read_input(&engine, 0);
        let lo = map.read_input(&engine, 1);
        let v = f32::from_bits(((hi as u32) << 16) | lo as u32);
        assert_eq!(v, 360.0);
    }

    /// 显式地址 + u16 单字编码：T1@3328（物理 25.5°C，encode=v*10 → 255）。
    #[test]
    fn explicit_address_u16_encode() {
        let cfg = SimConfig {
            controls: vec![],
            sensors: vec![SimSensor {
                sensor_id: "cdu.pri.in.t1".into(),
                name: "T1".into(),
                kind: "temperature".into(),
                unit: Some("°C".into()),
                formula: "25.5".into(),
                inputs: Default::default(),
                address: Some(3328),
                area: Area::Input,
                storage: Storage::U16,
                encode: Some("v * 10".into()),
            }],
            pri: None,
            sec: None,
        };
        cfg.validate().expect("validate ok");
        let map = RegisterMap::build(&cfg);
        assert_eq!(map.inputs.len(), 3329);
        assert_eq!(map.read_input(&SimEngine::new(cfg.clone()), 3328), 255);
        // 空槽位返回 0
        assert_eq!(map.read_input(&SimEngine::new(cfg), 0), 0);
    }

    /// 相邻 u16 单字地址（T1@3328、T2@3329）不重叠。
    #[test]
    fn adjacent_u16_addresses() {
        let cfg = SimConfig {
            controls: vec![],
            sensors: vec![
                SimSensor {
                    sensor_id: "cdu.pri.in.t1".into(),
                    name: "T1".into(),
                    kind: "temperature".into(),
                    unit: Some("°C".into()),
                    formula: "20".into(),
                    inputs: Default::default(),
                    address: Some(3328),
                    area: Area::Input,
                    storage: Storage::U16,
                    encode: Some("v * 10".into()),
                },
                SimSensor {
                    sensor_id: "cdu.pri.out.t2".into(),
                    name: "T2".into(),
                    kind: "temperature".into(),
                    unit: Some("°C".into()),
                    formula: "22".into(),
                    inputs: Default::default(),
                    address: Some(3329),
                    area: Area::Input,
                    storage: Storage::U16,
                    encode: Some("v * 10".into()),
                },
            ],
            pri: None,
            sec: None,
        };
        cfg.validate().expect("validate ok");
        let map = RegisterMap::build(&cfg);
        let engine = SimEngine::new(cfg);
        assert_eq!(map.read_input(&engine, 3328), 200);
        assert_eq!(map.read_input(&engine, 3329), 220);
    }

    /// 地址重叠被 validate 拒绝。
    #[test]
    fn overlapping_address_rejected() {
        let cfg = SimConfig {
            controls: vec![],
            sensors: vec![
                SimSensor {
                    sensor_id: "a".into(),
                    name: "A".into(),
                    kind: "x".into(),
                    unit: None,
                    formula: "1".into(),
                    inputs: Default::default(),
                    address: Some(100),
                    area: Area::Input,
                    storage: Storage::F32, // 占 100-101
                    encode: None,
                },
                SimSensor {
                    sensor_id: "b".into(),
                    name: "B".into(),
                    kind: "x".into(),
                    unit: None,
                    formula: "1".into(),
                    inputs: Default::default(),
                    address: Some(101),
                    area: Area::Input,
                    storage: Storage::U16,
                    encode: None,
                },
            ],
            pri: None,
            sec: None,
        };
        assert!(cfg.validate().is_err());
    }

    /// holding 区传感器：cdu2 的真实 CDU 传感器用 read_holding_registers。
    /// T1@3328（area=holding）应能通过保持寄存器（功能码 03）读到。
    #[test]
    fn holding_area_sensor_readable() {
        let cfg = SimConfig {
            controls: vec![SimControl {
                name: "pump1_duty".into(),
                initial: 50.0,
                unit: Some("%".into()),
                writable: true,
                address: Some(2192),
            }],
            sensors: vec![SimSensor {
                sensor_id: "cdu.pri.in.t1".into(),
                name: "T1".into(),
                kind: "temperature".into(),
                unit: Some("°C".into()),
                formula: "25.5".into(),
                inputs: Default::default(),
                address: Some(3328),
                area: Area::Holding,
                storage: Storage::U16,
                encode: Some("v * 10".into()),
            }],
            pri: None,
            sec: None,
        };
        cfg.validate().expect("validate ok");
        let map = RegisterMap::build(&cfg);
        let engine = SimEngine::new(cfg);
        // 保持寄存器 03 读到传感器值（原来读到 0 的 bug 修复）。
        assert_eq!(map.read_holding(&engine, 3328), 255);
        // 控制变量仍可读。
        assert_eq!(map.read_holding(&engine, 2192), 50);
    }

    /// holding 区控制与传感器地址冲突被拒绝。
    #[test]
    fn holding_area_conflict_rejected() {
        let cfg = SimConfig {
            controls: vec![SimControl {
                name: "pump1_duty".into(),
                initial: 50.0,
                unit: Some("%".into()),
                writable: true,
                address: Some(3328),
            }],
            sensors: vec![SimSensor {
                sensor_id: "cdu.pri.in.t1".into(),
                name: "T1".into(),
                kind: "temperature".into(),
                unit: Some("°C".into()),
                formula: "25.5".into(),
                inputs: Default::default(),
                address: Some(3328),
                area: Area::Holding,
                storage: Storage::U16,
                encode: None,
            }],
            pri: None,
            sec: None,
        };
        assert!(cfg.validate().is_err());
    }

    /// 线圈区：布尔量传感器（LI1@0、LI2@1、LE1@24/25），功能码 01 读取。
    #[test]
    fn coils_area_readable() {
        let cfg = SimConfig {
            controls: vec![],
            sensors: vec![
                SimSensor {
                    sensor_id: "cdu.liquid.li1".into(),
                    name: "LI1".into(),
                    kind: "level".into(),
                    unit: Some("level".into()),
                    formula: "1".into(),
                    inputs: Default::default(),
                    address: Some(0),
                    area: Area::Coils,
                    storage: Storage::U16,
                    encode: None,
                },
                SimSensor {
                    sensor_id: "cdu.liquid.li2".into(),
                    name: "LI2".into(),
                    kind: "level".into(),
                    unit: Some("level".into()),
                    formula: "1".into(),
                    inputs: Default::default(),
                    address: Some(1),
                    area: Area::Coils,
                    storage: Storage::U16,
                    encode: None,
                },
                SimSensor {
                    sensor_id: "cdu.leak.le1".into(),
                    name: "LE1".into(),
                    kind: "leak".into(),
                    unit: Some("leak".into()),
                    formula: "0".into(), // 无泄漏
                    inputs: Default::default(),
                    address: Some(24),
                    area: Area::Coils,
                    storage: Storage::U16,
                    encode: None,
                },
                SimSensor {
                    sensor_id: "cdu.leak.le1.break".into(),
                    name: "LE1 Break".into(),
                    kind: "leak".into(),
                    unit: Some("leak".into()),
                    formula: "1".into(), // 泄漏断路
                    inputs: Default::default(),
                    address: Some(25),
                    area: Area::Coils,
                    storage: Storage::U16,
                    encode: None,
                },
            ],
            pri: None,
            sec: None,
        };
        cfg.validate().expect("validate ok");
        let map = RegisterMap::build(&cfg);
        let engine = SimEngine::new(cfg);
        assert_eq!(map.coils.len(), 26); // 0..=25
        assert!(map.read_coil(&engine, 0)); // LI1 ON
        assert!(map.read_coil(&engine, 1)); // LI2 ON
        assert!(!map.read_coil(&engine, 24)); // LE1 无泄漏 OFF
        assert!(map.read_coil(&engine, 25)); // LE1 break ON
        assert!(!map.read_coil(&engine, 10)); // 空槽位 OFF
    }

    /// 线圈区地址冲突被拒绝。
    #[test]
    fn coils_area_conflict_rejected() {
        let cfg = SimConfig {
            controls: vec![],
            sensors: vec![
                SimSensor {
                    sensor_id: "a".into(),
                    name: "A".into(),
                    kind: "leak".into(),
                    unit: None,
                    formula: "1".into(),
                    inputs: Default::default(),
                    address: Some(5),
                    area: Area::Coils,
                    storage: Storage::U16,
                    encode: None,
                },
                SimSensor {
                    sensor_id: "b".into(),
                    name: "B".into(),
                    kind: "leak".into(),
                    unit: None,
                    formula: "1".into(),
                    inputs: Default::default(),
                    address: Some(5),
                    area: Area::Coils,
                    storage: Storage::U16,
                    encode: None,
                },
            ],
            pri: None,
            sec: None,
        };
        assert!(cfg.validate().is_err());
    }
}

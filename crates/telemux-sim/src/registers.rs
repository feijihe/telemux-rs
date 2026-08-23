//! Modbus 寄存器地图：把仿真模型映射为从站暴露的地址空间。
//!
//! 契约（`docs/SIMULATION.md`）：
//! - 保持寄存器 `0x0000` 起：可写控制变量（u16，0-100），Modbus 写 → 驱动模型；
//! - 输入寄存器 `0x0000` 起：传感器测量值（f32 双字，Big 字序），只读。
//!
//! 地址分配是**确定性 append**（按配置顺序），与网关侧 Modbus Server 的
//! 分配风格一致，便于对接（网关把 `[[devices.registers]]` 映射到这里）。

use crate::model::{SimConfig, SimEngine};

/// 保持寄存器槽位：控制变量名。
#[derive(Debug, Clone)]
pub struct HoldingSlot {
    /// 控制变量名。
    pub control: String,
    /// 是否为可写控制变量。
    pub writable: bool,
}

/// 输入寄存器槽位：传感器（f32 占 2 字）。
#[derive(Debug, Clone)]
pub struct InputSlot {
    /// 传感器 id。
    pub sensor_id: String,
}

/// 寄存器地图：由配置构建，地址 = Vec 下标。
#[derive(Debug, Clone, Default)]
pub struct RegisterMap {
    /// 保持区（控制变量，u16）。
    pub holding: Vec<Option<HoldingSlot>>,
    /// 输入区（传感器，f32 双字）。
    pub inputs: Vec<Option<InputSlot>>,
}

impl RegisterMap {
    /// 从配置构建地图。控制变量按配置顺序进保持区；传感器按配置顺序
    /// 进输入区（每个 f32 占 2 个字）。
    pub fn build(sim: &SimConfig) -> Self {
        let mut map = RegisterMap::default();
        for c in &sim.controls {
            map.holding.push(Some(HoldingSlot {
                control: c.name.clone(),
                writable: c.writable,
            }));
        }
        for s in &sim.sensors {
            // f32 双字：高字在前（Big）。
            map.inputs.push(Some(InputSlot {
                sensor_id: s.sensor_id.clone(),
            }));
            map.inputs.push(Some(InputSlot {
                sensor_id: s.sensor_id.clone(),
            }));
        }
        map
    }

    /// 读取保持寄存器（控制变量当前值，取整为 u16）。
    pub fn read_holding(&self, engine: &SimEngine, addr: usize) -> u16 {
        match self.holding.get(addr).and_then(|s| s.as_ref()) {
            Some(slot) => {
                let v = engine.control(&slot.control).unwrap_or(0.0);
                // duty 0-100：u16 直存；超出范围截断。
                v.round().clamp(0.0, u16::MAX as f64) as u16
            }
            None => 0,
        }
    }

    /// 读取输入寄存器（传感器值 f32 的半个字）。`word_index` 0=高字 1=低字。
    pub fn read_input(&self, engine: &SimEngine, addr: usize) -> u16 {
        let slot = match self.inputs.get(addr).and_then(|s| s.as_ref()) {
            Some(s) => s,
            None => return 0,
        };
        let values: std::collections::HashMap<String, f64> =
            engine.eval_all().into_iter().collect();
        let v = values.get(&slot.sensor_id).copied().unwrap_or(f32::NAN as f64);
        let bits = (v as f32).to_bits();
        if addr.is_multiple_of(2) {
            (bits >> 16) as u16 // 高字
        } else {
            (bits & 0xFFFF) as u16 // 低字
        }
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
            }],
            sensors: vec![SimSensor {
                sensor_id: "cdu.pri.p1".into(),
                name: "P1".into(),
                kind: "pressure".into(),
                unit: Some("kPa".into()),
                formula: "300 + pump1_duty * 1.2".into(), // 360
                inputs: Default::default(),
            }],
        }
    }

    #[test]
    fn map_lays_out_addresses() {
        let map = RegisterMap::build(&sim_config());
        assert_eq!(map.holding.len(), 1);
        assert_eq!(map.holding[0].as_ref().unwrap().control, "pump1_duty");
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
}

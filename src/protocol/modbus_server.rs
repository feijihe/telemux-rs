//! Modbus-TCP 服务器（阶段 5）：通过四个 Modbus 区域暴露传感器指标，
//! 并将写入转发到可读写寄存器。
//!
//! 地址分配完全由配置驱动且确定（见 `docs/MODBUS_SERVER.md`）：
//! 每个区域从 0 开始，按配置顺序追加寄存器；多字值每个字占用一个地址。
//! 配置中新增的寄存器/computed 传感器会自动出现；仅追加的变更保持
//! 既有地址稳定。

use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;

use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio_modbus::server::tcp::{accept_tcp_connection, Server as TcpModbusServer};
use tokio_modbus::server::Service;
use tokio_modbus::{Address, ExceptionCode, Quantity, Request, Response};
use tracing::{info, warn};

use crate::config::{Access, Config, RegisterFunction, Transport, ValueType, WordOrder};
use crate::config_handle::ConfigHandle;
use crate::domain::SensorId;
use crate::protocol::{WriteBroker, WriteValue};
use crate::store::MetricStore;

// ---------- 寄存器表 ----------

/// 一个 16 位字槽位：属于哪个传感器，以及它持有值编码中的哪个字。
#[derive(Debug, Clone)]
pub struct SlotWord {
    pub sensor_id: String,
    pub device: String,
    pub value_type: ValueType,
    pub word_order: WordOrder,
    pub word_index: usize,
    pub access: Access,
}

/// 一个 bit 槽位（线圈 / 离散输入区域）。
#[derive(Debug, Clone)]
pub struct SlotBit {
    pub sensor_id: String,
    pub device: String,
    pub access: Access,
}

/// 四个 Modbus 区域。索引 = 地址。
#[derive(Debug, Default)]
pub struct ModbusTable {
    pub coils: Vec<Option<SlotBit>>,
    pub discrete_inputs: Vec<Option<SlotBit>>,
    pub holding: Vec<Option<SlotWord>>,
    pub inputs: Vec<Option<SlotWord>>,
}

/// 从配置构建表。computed 传感器进入输入寄存器区域
/// （只读，编码为 f32）。
pub fn build_table(config: &Config) -> ModbusTable {
    let mut table = ModbusTable::default();
    for device in &config.devices {
        for reg in &device.registers {
            match reg.function {
                RegisterFunction::Coil => table
                    .coils
                    .push(Some(SlotBit {
                        sensor_id: reg.sensor_id.clone(),
                        device: device.name.clone(),
                        access: reg.access,
                    })),
                RegisterFunction::DiscreteInput => table
                    .discrete_inputs
                    .push(Some(SlotBit {
                        sensor_id: reg.sensor_id.clone(),
                        device: device.name.clone(),
                        access: reg.access,
                    })),
                RegisterFunction::Holding => {
                    append_words(&mut table.holding, &reg.sensor_id, &device.name, reg.value_type, reg.word_order, reg.access)
                }
                RegisterFunction::Input => {
                    append_words(&mut table.inputs, &reg.sensor_id, &device.name, reg.value_type, reg.word_order, reg.access)
                }
            }
        }
    }
    // Computed 传感器：只读、f32、位于输入寄存器区域。
    for c in &config.computed {
        append_words(
            &mut table.inputs,
            &c.sensor_id,
            "",
            ValueType::F32,
            WordOrder::Big,
            Access::Read,
        );
    }
    // 仿真传感器（阶段 8）：只读、f32、位于输入寄存器区域（与 computed 同级）。
    for s in &config.sim.sensors {
        let device = config
            .devices
            .iter()
            .find(|d| d.transport == Transport::Sim)
            .map(|d| d.name.clone())
            .unwrap_or_default();
        append_words(
            &mut table.inputs,
            &s.sensor_id,
            &device,
            ValueType::F32,
            WordOrder::Big,
            Access::Read,
        );
    }
    // 仿真控制变量（阶段 8）：可写控制变量映射为保持寄存器（u16，
    // 值域 0-100），写保持寄存器 → 更新控制变量 → 驱动仿真。
    for c in &config.sim.controls {
        if c.writable {
            let device = config
                .devices
                .iter()
                .find(|d| d.transport == Transport::Sim)
                .map(|d| d.name.clone())
                .unwrap_or_default();
            append_words(
                &mut table.holding,
                &c.name,
                &device,
                ValueType::U16,
                WordOrder::Big,
                Access::ReadWrite,
            );
        }
    }
    table
}

fn append_words(
    area: &mut Vec<Option<SlotWord>>,
    sensor_id: &str,
    device: &str,
    value_type: ValueType,
    word_order: WordOrder,
    access: Access,
) {
    let count = value_type.register_count() as usize;
    for word_index in 0..count {
        area.push(Some(SlotWord {
            sensor_id: sensor_id.to_string(),
            device: device.to_string(),
            value_type,
            word_order,
            word_index,
            access,
        }));
    }
}

/// 将值编码为 16 位字（采集解码器的逆操作）。
pub fn encode_value(value: f64, value_type: ValueType, word_order: WordOrder) -> Vec<u16> {
    match value_type {
        ValueType::U16 => vec![value.round() as u16],
        ValueType::I16 => vec![(value.round() as i16) as u16],
        ValueType::U32 => split32(value.round() as u32, word_order),
        ValueType::I32 => split32((value.round() as i32) as u32, word_order),
        ValueType::F32 => split32((value as f32).to_bits(), word_order),
        ValueType::Bool => vec![if value != 0.0 { 1 } else { 0 }],
    }
}

fn split32(v: u32, order: WordOrder) -> Vec<u16> {
    match order {
        WordOrder::Big => vec![(v >> 16) as u16, (v & 0xFFFF) as u16],
        WordOrder::Little => vec![(v & 0xFFFF) as u16, (v >> 16) as u16],
    }
}

// ---------- 服务 ----------

#[derive(Clone)]
pub struct ModbusState {
    config: ConfigHandle,
    store: Arc<MetricStore>,
    broker: Arc<WriteBroker>,
}

pub struct ModbusService {
    state: Arc<ModbusState>,
}

impl Service for ModbusService {
    type Request = Request<'static>;
    type Response = Response;
    type Exception = ExceptionCode;
    type Future = Pin<Box<dyn Future<Output = Result<Response, ExceptionCode>> + Send>>;

    fn call(&self, req: Self::Request) -> Self::Future {
        let state = self.state.clone();
        Box::pin(async move { state.handle(req).await })
    }
}

impl ModbusState {
    async fn handle(&self, req: Request<'static>) -> Result<Response, ExceptionCode> {
        let config = self.config.read();
        let table = build_table(&config);
        match req {
            Request::ReadCoils(addr, cnt) => {
                Ok(Response::ReadCoils(self.read_bits(&table.coils, addr, cnt)))
            }
            Request::ReadDiscreteInputs(addr, cnt) => Ok(Response::ReadDiscreteInputs(
                self.read_bits(&table.discrete_inputs, addr, cnt),
            )),
            Request::ReadHoldingRegisters(addr, cnt) => Ok(Response::ReadHoldingRegisters(
                self.read_words(&table.holding, addr, cnt)?,
            )),
            Request::ReadInputRegisters(addr, cnt) => Ok(Response::ReadInputRegisters(
                self.read_words(&table.inputs, addr, cnt)?,
            )),
            Request::WriteSingleCoil(addr, coil) => {
                self.write_bit(&table.coils, addr, coil).await?;
                Ok(Response::WriteSingleCoil(addr, coil))
            }
            Request::WriteSingleRegister(addr, word) => {
                self.write_word(&table.holding, addr, word).await?;
                Ok(Response::WriteSingleRegister(addr, word))
            }
            Request::WriteMultipleRegisters(addr, words) => {
                self.write_words(&table.holding, addr, &words).await?;
                Ok(Response::WriteMultipleRegisters(addr, words.len() as Quantity))
            }
            _ => Err(ExceptionCode::IllegalFunction),
        }
    }

    fn read_bits(
        &self,
        area: &[Option<SlotBit>],
        addr: Address,
        cnt: Quantity,
    ) -> Vec<bool> {
        let end = (addr as usize) + (cnt as usize);
        let mut out = Vec::with_capacity(cnt as usize);
        for i in addr as usize..end {
            match area.get(i) {
                Some(Some(slot)) => {
                    let v = self.value_of(&slot.sensor_id).unwrap_or(0.0);
                    out.push(v != 0.0);
                }
                _ => out.push(false),
            }
        }
        out
    }

    fn read_words(
        &self,
        area: &[Option<SlotWord>],
        addr: Address,
        cnt: Quantity,
    ) -> Result<Vec<u16>, ExceptionCode> {
        let end = (addr as usize) + (cnt as usize);
        if end > area.len() {
            return Err(ExceptionCode::IllegalDataAddress);
        }
        let mut out = Vec::with_capacity(cnt as usize);
        for slot in &area[addr as usize..end] {
            match slot {
                Some(slot) => {
                    let value = self.value_of(&slot.sensor_id).unwrap_or(0xFFFF as f64);
                    let words = encode_value(value, slot.value_type, slot.word_order);
                    out.push(words.get(slot.word_index).copied().unwrap_or(0xFFFF));
                }
                None => out.push(0xFFFF),
            }
        }
        Ok(out)
    }

    async fn write_bit(
        &self,
        area: &[Option<SlotBit>],
        addr: Address,
        coil: bool,
    ) -> Result<(), ExceptionCode> {
        let slot = area
            .get(addr as usize)
            .and_then(|s| s.as_ref())
            .ok_or(ExceptionCode::IllegalDataAddress)?;
        if slot.access != Access::ReadWrite {
            return Err(ExceptionCode::IllegalFunction);
        }
        self.broker
            .write(&slot.device, &slot.sensor_id, WriteValue::Coil(coil))
            .await
            .map_err(|_| ExceptionCode::ServerDeviceFailure)
    }

    async fn write_word(
        &self,
        area: &[Option<SlotWord>],
        addr: Address,
        word: u16,
    ) -> Result<(), ExceptionCode> {
        let slot = area
            .get(addr as usize)
            .and_then(|s| s.as_ref())
            .ok_or(ExceptionCode::IllegalDataAddress)?;
        // 只有单字寄存器可通过 0x06 写入（值语义）。
        if slot.access != Access::ReadWrite || slot.value_type.register_count() != 1 {
            return Err(ExceptionCode::IllegalFunction);
        }
        self.broker
            .write(&slot.device, &slot.sensor_id, WriteValue::Holding(word))
            .await
            .map_err(|_| ExceptionCode::ServerDeviceFailure)
    }

    async fn write_words(
        &self,
        area: &[Option<SlotWord>],
        addr: Address,
        words: &[u16],
    ) -> Result<(), ExceptionCode> {
        let slot = area
            .get(addr as usize)
            .and_then(|s| s.as_ref())
            .ok_or(ExceptionCode::IllegalDataAddress)?;
        if slot.access != Access::ReadWrite || slot.value_type.register_count() != 1 {
            return Err(ExceptionCode::IllegalFunction);
        }
        let word = words.first().copied().unwrap_or(0);
        self.broker
            .write(&slot.device, &slot.sensor_id, WriteValue::Holding(word))
            .await
            .map_err(|_| ExceptionCode::ServerDeviceFailure)
    }

    fn value_of(&self, sensor_id: &str) -> Option<f64> {
        let state = self.store.get(&SensorId(sensor_id.to_string()))?;
        if let Some(m) = &state.metric {
            Some(m.value)
        } else {
            state.raw.as_ref().map(|r| r.raw_value)
        }
    }
}

// ---------- 服务器 ----------

/// 启动 Modbus-TCP 服务器；`shutdown` 触发时返回。
pub async fn run(
    config: ConfigHandle,
    store: Arc<MetricStore>,
    broker: Arc<WriteBroker>,
    port: u16,
    unit_id: u8,
    shutdown: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let addr: SocketAddr = format!("0.0.0.0:{port}").parse()?;
    let listener = TcpListener::bind(addr).await?;
    let server = TcpModbusServer::new(listener);
    let state = Arc::new(ModbusState {
        config,
        store,
        broker,
    });
    info!("modbus server listening on {addr} (unit id {unit_id})");

    let on_connected = {
        let state = state.clone();
        move |stream: TcpStream, socket_addr: SocketAddr| {
            let state = state.clone();
            async move {
                accept_tcp_connection(stream, socket_addr, move |_| {
                    Ok(Some(ModbusService { state: state.clone() }))
                })
            }
        }
    };
    let abort = async move {
        let mut shutdown = shutdown;
        let _ = shutdown.changed().await;
    };
    let _terminated = server.serve_until(&on_connected, |e| warn!("modbus server: {e}"), abort).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ComputedConfig, DeviceConfig, RegisterConfig, Transport};

    fn sample_config() -> Config {
        Config {
            general: Default::default(),
            devices: vec![DeviceConfig {
                name: "pcba-01".into(),
                transport: Transport::Tcp,
                unit_id: 1,
                host: "127.0.0.1".into(),
                port: 502,
                poll_interval_ms: 100,
                timeout_ms: 100,
                reconnect_initial_ms: 100,
                reconnect_max_ms: 1000,
                serial_port: None,
                baud_rate: None,
                registers: vec![
                    RegisterConfig {
                        name: "fan1_duty".into(),
                        sensor_id: "s.fan_duty".into(),
                        function: RegisterFunction::Holding,
                        access: Access::ReadWrite,
                        address: 10,
                        count: Some(1),
                        value_type: ValueType::U16,
                        word_order: WordOrder::Big,
                        unit: Some("%".into()),
                    },
                    RegisterConfig {
                        name: "vcore".into(),
                        sensor_id: "s.vcore".into(),
                        function: RegisterFunction::Input,
                        access: Access::Read,
                        address: 4,
                        count: Some(2),
                        value_type: ValueType::F32,
                        word_order: WordOrder::Big,
                        unit: Some("V".into()),
                    },
                    RegisterConfig {
                        name: "leak".into(),
                        sensor_id: "s.leak".into(),
                        function: RegisterFunction::DiscreteInput,
                        access: Access::Read,
                        address: 0,
                        count: Some(1),
                        value_type: ValueType::Bool,
                        word_order: WordOrder::Big,
                        unit: Some("leak".into()),
                    },
                ],
            }],
            pipelines: vec![],
            computed: vec![ComputedConfig {
                sensor_id: "s.dew".into(),
                name: "dew".into(),
                unit: Some("°C".into()),
                inputs: [("t".to_string(), "s.vcore".to_string())].into(),
                expression: "t".into(),
            }],
            endpoints: Default::default(),
            sim: Default::default(),
        }
    }

    #[test]
    fn table_lays_out_four_areas() {
        let table = build_table(&sample_config());
        // holding：fan1_duty u16 -> 地址 0 处 1 个字
        assert_eq!(table.holding.len(), 1);
        assert_eq!(table.holding[0].as_ref().unwrap().sensor_id, "s.fan_duty");
        assert_eq!(table.holding[0].as_ref().unwrap().word_index, 0);
        // input：vcore f32（2 字）+ computed dew f32（2 字）
        assert_eq!(table.inputs.len(), 4);
        assert_eq!(table.inputs[0].as_ref().unwrap().sensor_id, "s.vcore");
        assert_eq!(table.inputs[1].as_ref().unwrap().word_index, 1);
        assert_eq!(table.inputs[2].as_ref().unwrap().sensor_id, "s.dew");
        // 离散输入：leak
        assert_eq!(table.discrete_inputs.len(), 1);
        assert_eq!(table.coils.len(), 0);
    }

    #[test]
    fn encodes_values_per_type() {
        assert_eq!(encode_value(27.4, ValueType::U16, WordOrder::Big), vec![27]);
        assert_eq!(
            encode_value(-2.0, ValueType::I16, WordOrder::Big),
            vec![0xFFFE]
        );
        assert_eq!(
            encode_value(0xDEAD_BEEFu32 as f64, ValueType::U32, WordOrder::Big),
            vec![0xDEAD, 0xBEEF]
        );
        assert_eq!(
            encode_value(12.5, ValueType::F32, WordOrder::Big),
            vec![0x4148, 0x0000]
        );
        assert_eq!(
            encode_value(12.5, ValueType::F32, WordOrder::Little),
            vec![0x0000, 0x4148]
        );
        assert_eq!(encode_value(1.0, ValueType::Bool, WordOrder::Big), vec![1]);
    }

    // ---- 服务层错误路径（阶段 7）----

    fn state() -> ModbusState {
        ModbusState {
            config: ConfigHandle::new(sample_config(), "unused.toml".into()),
            store: Arc::new(MetricStore::new()),
            broker: Arc::new(WriteBroker::default()),
        }
    }

    #[tokio::test]
    async fn read_out_of_range_is_illegal_data_address() {
        let s = state();
        let table = build_table(&sample_config());
        // holding 区只有 1 个字；读 2 个 -> 越界。
        let err = s.read_words(&table.holding, 0, 2).unwrap_err();
        assert_eq!(err, ExceptionCode::IllegalDataAddress);
        // 起始地址超出表长。
        let err = s.read_words(&table.holding, 5, 1).unwrap_err();
        assert_eq!(err, ExceptionCode::IllegalDataAddress);
    }

    #[tokio::test]
    async fn read_missing_slot_returns_placeholder() {
        let s = state();
        let table = build_table(&sample_config());
        // coils 区为空 -> 读任何地址返回 false。
        let bits = s.read_bits(&table.coils, 0, 3);
        assert_eq!(bits, vec![false, false, false]);
        // holding 区无数据（store 空）-> 0xFFFF 占位。
        let words = s.read_words(&table.holding, 0, 1).unwrap();
        assert_eq!(words, vec![0xFFFF]);
    }

    #[tokio::test]
    async fn write_readonly_register_is_illegal_function() {
        let s = state();
        let table = build_table(&sample_config());
        // discrete_inputs[0] = leak（read）-> 写必须被拒。
        let err = s.write_bit(&table.discrete_inputs, 0, true).await.unwrap_err();
        assert_eq!(err, ExceptionCode::IllegalFunction);
        // holding 区 fan_duty 是 read_write 且单字 -> 允许（broker 空设备则
        // 转发失败，但那是 ServerDeviceFailure，不是 IllegalFunction）。
    }
}

//! 传感器采集层：从 PCBA 设备读取原始寄存器值，
//! 为处理管道（阶段 3）产出 [`RawSample`]。
//!
//! 传输由 `tokio-modbus` 提供（TCP + RTU 客户端）。

pub mod decoder;
pub mod manager;
pub mod rtu;
pub mod tcp;

pub use manager::AcquisitionManager;

use std::time::Duration;

use async_trait::async_trait;
use tokio_modbus::client::{Context, Reader, Writer};

use crate::config::{Access, DeviceConfig, RegisterConfig, RegisterFunction, Transport};
use crate::domain::RawSample;

/// 采集层错误。
#[derive(Debug, thiserror::Error)]
pub enum AcquisitionError {
    #[error("modbus transport error: {0}")]
    Modbus(#[from] tokio_modbus::Error),
    #[error("modbus exception from slave: {0:?}")]
    Exception(#[from] tokio_modbus::ExceptionCode),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serial port error: {0}")]
    Serial(#[from] tokio_serial::Error),
    #[error("request timed out after {0:?}")]
    Timeout(Duration),
    #[error("device `{device}`: {message}")]
    Config { device: String, message: String },
    #[error("device `{device}` register `{name}`: {message}")]
    Register {
        device: String,
        name: String,
        message: String,
    },
}

/// 硬件传感器源：为每个配置的寄存器产出一个原始样本。
///
/// 寄存器列表每次调用时传入，以便运行时配置可被热读取
/// （开发仪表盘添加寄存器无需重启网关）。
/// 实现内部按需重连；错误返回给调用方，由调度器应用退避。
#[async_trait]
pub trait SensorSource: Send {
    async fn read_samples(
        &mut self,
        registers: &[RegisterConfig],
    ) -> Result<Vec<RawSample>, AcquisitionError>;

    /// 写入一个可读写保持寄存器的值（按 sensor_id）。
    async fn write_holding_register(
        &mut self,
        sensor_id: &str,
        value: u16,
    ) -> Result<(), AcquisitionError>;

    /// 写入一个线圈（按 sensor_id；仅 `coil` + `access=read_write`）。
    async fn write_single_coil(
        &mut self,
        sensor_id: &str,
        value: bool,
    ) -> Result<(), AcquisitionError>;
}

/// 为设备创建数据源（按传输方式）。
///
/// `sim` 仅在 `Transport::Sim` 时使用（仿真数据源的物理模型配置）。
pub async fn create_source(
    device: &DeviceConfig,
    sim: &crate::config::SimConfig,
) -> Result<Box<dyn SensorSource>, AcquisitionError> {
    match device.transport {
        Transport::Tcp => {
            let source = tcp::ModbusTcpSource::connect(device).await?;
            Ok(Box::new(source) as Box<dyn SensorSource>)
        }
        Transport::Rtu => {
            let source = rtu::ModbusRtuSource::connect(device).await?;
            Ok(Box::new(source) as Box<dyn SensorSource>)
        }
        Transport::Sim => {
            // 仿真数据源：从全局 `[sim]` 配置计算所有传感器的值（无硬件）。
            let source = crate::simulation::SimSource::new(sim.clone());
            Ok(Box::new(source) as Box<dyn SensorSource>)
        }
    }
}

/// 通过 tokio-modbus 客户端上下文读取给定寄存器，将每个寄存器组
/// 解码为 [`RawSample`]。TCP 与 RTU 数据源共用；每次寄存器读取受
/// `timeout` 保护。
pub(crate) async fn read_registers_from_context(
    client: &mut Context,
    device_name: &str,
    registers: &[RegisterConfig],
    timeout: Duration,
) -> Result<Vec<RawSample>, AcquisitionError> {
    let mut samples = Vec::with_capacity(registers.len());

    // 注意：每个寄存器组一个请求。后续阶段优化：
    // 将同功能区的连续寄存器组合并成一次读取，以减少往返次数。
    for reg in registers {
        if reg.function.is_bit() {
            // 单 bit 区域（coil / 离散输入）。
            let function = match reg.function {
                RegisterFunction::Coil => client.read_coils(reg.address, reg.effective_count()),
                RegisterFunction::DiscreteInput => {
                    client.read_discrete_inputs(reg.address, reg.effective_count())
                }
                _ => unreachable!("bit function check"),
            };
            let result = tokio::time::timeout(timeout, function)
                .await
                .map_err(|_| AcquisitionError::Timeout(timeout))?;
            let coils = match result {
                Ok(Ok(coils)) => coils,
                Ok(Err(code)) => return Err(AcquisitionError::Exception(code)),
                Err(e) => return Err(AcquisitionError::Modbus(e)),
            };
            let value = coils.first().copied().unwrap_or(false);
            samples.push(decoder::raw_sample_bool(device_name, reg, value)?);
        } else {
            let function = match reg.function {
                RegisterFunction::Holding => {
                    client.read_holding_registers(reg.address, reg.effective_count())
                }
                RegisterFunction::Input => {
                    client.read_input_registers(reg.address, reg.effective_count())
                }
                _ => unreachable!("word function check"),
            };
            let result = tokio::time::timeout(timeout, function)
                .await
                .map_err(|_| AcquisitionError::Timeout(timeout))?;
            let words = match result {
                Ok(Ok(words)) => words,
                Ok(Err(code)) => return Err(AcquisitionError::Exception(code)),
                Err(e) => return Err(AcquisitionError::Modbus(e)),
            };
            samples.push(decoder::raw_sample(device_name, reg, &words)?);
        }
    }
    Ok(samples)
}

/// 按 sensor_id 定位寄存器并校验其可写（holding + read_write）。
fn writable_register<'a>(
    device: &'a DeviceConfig,
    sensor_id: &str,
) -> Result<&'a RegisterConfig, AcquisitionError> {
    let reg = device
        .registers
        .iter()
        .find(|r| r.sensor_id == sensor_id)
        .ok_or_else(|| AcquisitionError::Config {
            device: device.name.clone(),
            message: format!("unknown sensor_id `{sensor_id}` for write"),
        })?;
    if reg.function != RegisterFunction::Holding {
        return Err(AcquisitionError::Config {
            device: device.name.clone(),
            message: format!(
                "sensor `{sensor_id}` is not a holding register (cannot write)"
            ),
        });
    }
    if reg.access != Access::ReadWrite {
        return Err(AcquisitionError::Config {
            device: device.name.clone(),
            message: format!("sensor `{sensor_id}` is read-only (access=read)"),
        });
    }
    Ok(reg)
}

/// 通过 tokio-modbus 客户端上下文写入一个保持寄存器。
pub(crate) async fn write_holding_register_from_context(
    client: &mut Context,
    device: &DeviceConfig,
    sensor_id: &str,
    value: u16,
    timeout: Duration,
) -> Result<(), AcquisitionError> {
    let reg = writable_register(device, sensor_id)?;
    let result = tokio::time::timeout(timeout, async {
        client.write_single_register(reg.address, value).await
    })
    .await
    .map_err(|_| AcquisitionError::Timeout(timeout))?;
    match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(code)) => Err(AcquisitionError::Exception(code)),
        Err(e) => Err(AcquisitionError::Modbus(e)),
    }
}

/// 通过 tokio-modbus 客户端上下文写入一个线圈。
pub(crate) async fn write_single_coil_from_context(
    client: &mut Context,
    device: &DeviceConfig,
    sensor_id: &str,
    value: bool,
    timeout: Duration,
) -> Result<(), AcquisitionError> {
    let reg = device
        .registers
        .iter()
        .find(|r| r.sensor_id == sensor_id)
        .ok_or_else(|| AcquisitionError::Config {
            device: device.name.clone(),
            message: format!("unknown sensor_id `{sensor_id}` for write"),
        })?;
    if reg.function != RegisterFunction::Coil {
        return Err(AcquisitionError::Config {
            device: device.name.clone(),
            message: format!("sensor `{sensor_id}` is not a coil (cannot write)"),
        });
    }
    if reg.access != Access::ReadWrite {
        return Err(AcquisitionError::Config {
            device: device.name.clone(),
            message: format!("sensor `{sensor_id}` is read-only (access=read)"),
        });
    }
    let result = tokio::time::timeout(timeout, async {
        client.write_single_coil(reg.address, value).await
    })
    .await
    .map_err(|_| AcquisitionError::Timeout(timeout))?;
    match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(code)) => Err(AcquisitionError::Exception(code)),
        Err(e) => Err(AcquisitionError::Modbus(e)),
    }
}

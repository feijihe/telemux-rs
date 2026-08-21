//! Sensor acquisition layer: reads raw register values from PCBA devices
//! and produces [`RawSample`]s for the processing pipeline (phase 3).
//!
//! Transport is provided by `tokio-modbus` (TCP + RTU clients).

pub mod decoder;
pub mod manager;
pub mod rtu;
pub mod tcp;

pub use manager::AcquisitionManager;

use std::time::Duration;

use async_trait::async_trait;
use tokio_modbus::client::{Context, Reader};

use crate::config::{DeviceConfig, RegisterConfig, RegisterFunction, Transport};
use crate::domain::RawSample;

/// Acquisition-layer errors.
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

/// A hardware sensor source: produces one raw sample per configured register.
///
/// The register list is passed per call so the runtime configuration can be
/// hot-read (dev dashboard adds registers without restarting the gateway).
/// Implementations reconnect internally as needed; errors are returned to the
/// caller so the scheduler can apply backoff.
#[async_trait]
pub trait SensorSource: Send {
    async fn read_samples(
        &mut self,
        registers: &[RegisterConfig],
    ) -> Result<Vec<RawSample>, AcquisitionError>;
}

/// Create the source for a device (transport-specific).
pub async fn create_source(
    device: &DeviceConfig,
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
    }
}

/// Read the given registers through a tokio-modbus client context, decoding
/// each register group into a [`RawSample`]. Shared by the TCP and RTU
/// sources; each register read is guarded by `timeout`.
pub(crate) async fn read_registers_from_context(
    client: &mut Context,
    device_name: &str,
    registers: &[RegisterConfig],
    timeout: Duration,
) -> Result<Vec<RawSample>, AcquisitionError> {
    let mut samples = Vec::with_capacity(registers.len());

    // NOTE: one request per register group. Optimization for later phases:
    // merge contiguous register groups of the same function into a single
    // read to cut round trips.
    for reg in registers {
        let function = match reg.function {
            RegisterFunction::Holding => {
                client.read_holding_registers(reg.address, reg.effective_count())
            }
            RegisterFunction::Input => {
                client.read_input_registers(reg.address, reg.effective_count())
            }
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
    Ok(samples)
}

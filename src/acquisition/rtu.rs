//! Modbus-RTU sensor source, based on `tokio-modbus` + `tokio-serial`.
//!
//! [`ModbusRtuSource::connect`] opens a real serial port. For tests or
//! RTU-over-TCP (serial-to-Ethernet converters), [`ModbusRtuSource::from_stream`]
//! attaches the RTU client to any async byte stream.

use std::time::Duration;

use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_modbus::client::Context;
use tokio_modbus::slave::Slave;
use tokio_serial::SerialPortBuilderExt;

use crate::acquisition::{read_registers_from_context, AcquisitionError, SensorSource};
use crate::config::{DeviceConfig, RegisterConfig};
use crate::domain::RawSample;

/// RTU sensor source over a serial port (or any async stream via [`from_stream`]).
pub struct ModbusRtuSource {
    device: DeviceConfig,
    client: Option<Context>,
    timeout: Duration,
}

impl ModbusRtuSource {
    /// Open the configured serial port and attach a Modbus-RTU client.
    pub async fn connect(device: &DeviceConfig) -> Result<Self, AcquisitionError> {
        let port = device.serial_port.as_deref().ok_or_else(|| AcquisitionError::Config {
            device: device.name.clone(),
            message: "rtu transport requires `serial_port`".to_string(),
        })?;
        let baud = device.baud_rate.ok_or_else(|| AcquisitionError::Config {
            device: device.name.clone(),
            message: "rtu transport requires `baud_rate`".to_string(),
        })?;
        let builder = tokio_serial::new(port, baud);
        let stream = builder.open_native_async()?;
        tracing::debug!(
            "device {}: opened serial port {port} @ {baud} baud",
            device.name
        );
        let timeout = Duration::from_millis(device.timeout_ms);
        let client = tokio_modbus::client::rtu::attach_slave(stream, Slave(device.unit_id));
        Ok(Self {
            device: device.clone(),
            client: Some(client),
            timeout,
        })
    }

    /// Attach the RTU client to a caller-provided stream (tests, RTU-over-TCP).
    /// Reconnect-after-error is not supported for such streams: recreate the
    /// source instead.
    pub fn from_stream<S>(stream: S, device: &DeviceConfig) -> Self
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let timeout = Duration::from_millis(device.timeout_ms);
        let client = tokio_modbus::client::rtu::attach_slave(stream, Slave(device.unit_id));
        Self {
            device: device.clone(),
            client: Some(client),
            timeout,
        }
    }

    async fn read_samples_impl(
        &mut self,
        registers: &[RegisterConfig],
    ) -> Result<Vec<RawSample>, AcquisitionError> {
        // Reconnect by reopening the serial port if the previous client died.
        if self.client.is_none() {
            let rebuilt = Self::connect(&self.device).await?;
            *self = rebuilt;
        }
        let result = read_registers_from_context(
            self.client.as_mut().expect("client is present"),
            &self.device.name,
            registers,
            self.timeout,
        )
        .await;
        if result.is_err() {
            self.client = None;
        }
        result
    }
}

#[async_trait]
impl SensorSource for ModbusRtuSource {
    async fn read_samples(
        &mut self,
        registers: &[RegisterConfig],
    ) -> Result<Vec<RawSample>, AcquisitionError> {
        self.read_samples_impl(registers).await
    }
}

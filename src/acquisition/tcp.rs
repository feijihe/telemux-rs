//! Modbus-TCP sensor source, based on `tokio-modbus`.

use std::net::SocketAddr;
use std::time::Duration;

use async_trait::async_trait;
use tokio_modbus::client::Context;
use tokio_modbus::slave::Slave;

use crate::acquisition::{read_registers_from_context, AcquisitionError, SensorSource};
use crate::config::{DeviceConfig, RegisterConfig};
use crate::domain::RawSample;

/// Reads all registers of one device over Modbus-TCP.
///
/// Reconnect strategy: the connection is (re)established lazily on each poll
/// that finds no live client; on any read error the client is dropped so the
/// next poll reconnects. Timing/backoff is handled by the scheduler.
pub struct ModbusTcpSource {
    device: DeviceConfig,
    client: Option<Context>,
    timeout: Duration,
}

impl ModbusTcpSource {
    pub async fn connect(device: &DeviceConfig) -> Result<Self, AcquisitionError> {
        let addr = socket_addr(device)?;
        let timeout = Duration::from_millis(device.timeout_ms);
        let client = tokio_modbus::client::tcp::connect_slave(addr, Slave(device.unit_id)).await?;
        tracing::debug!("device {}: connected to {addr}", device.name);
        Ok(Self {
            device: device.clone(),
            client: Some(client),
            timeout,
        })
    }

    async fn read_samples_impl(
        &mut self,
        registers: &[RegisterConfig],
    ) -> Result<Vec<RawSample>, AcquisitionError> {
        if self.client.is_none() {
            let addr = socket_addr(&self.device)?;
            let client =
                tokio_modbus::client::tcp::connect_slave(addr, Slave(self.device.unit_id)).await?;
            tracing::debug!("device {}: reconnected to {addr}", self.device.name);
            self.client = Some(client);
        }
        let result = read_registers_from_context(
            self.client.as_mut().expect("client is present"),
            &self.device.name,
            registers,
            self.timeout,
        )
        .await;
        if result.is_err() {
            // Drop the dead connection; the next poll reconnects.
            self.client = None;
        }
        result
    }
}

fn socket_addr(device: &DeviceConfig) -> Result<SocketAddr, AcquisitionError> {
    format!("{}:{}", device.host, device.port)
        .parse()
        .map_err(|e| AcquisitionError::Config {
            device: device.name.clone(),
            message: format!("invalid address `{}:{}`: {e}", device.host, device.port),
        })
}

#[async_trait]
impl SensorSource for ModbusTcpSource {
    async fn read_samples(
        &mut self,
        registers: &[RegisterConfig],
    ) -> Result<Vec<RawSample>, AcquisitionError> {
        self.read_samples_impl(registers).await
    }
}

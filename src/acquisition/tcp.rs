//! 基于 `tokio-modbus` 的 Modbus-TCP 传感器数据源。

use std::net::SocketAddr;
use std::time::Duration;

use async_trait::async_trait;
use tokio_modbus::client::Context;
use tokio_modbus::slave::Slave;

use crate::acquisition::{
    read_registers_from_context, write_holding_register_from_context,
    write_single_coil_from_context, AcquisitionError, SensorSource,
};
use crate::config::{DeviceConfig, RegisterConfig};
use crate::domain::RawSample;

/// 通过 Modbus-TCP 读取一台设备的所有寄存器。
///
/// 重连策略：连接在每次轮询发现无存活客户端时惰性（重）建立；
/// 任何读取错误都会丢弃客户端，使下一次轮询重连。计时/退避由调度器处理。
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

    /// 确保存在存活客户端（惰性连接/重连）。
    async fn ensure_connected(&mut self) -> Result<(), AcquisitionError> {
        if self.client.is_none() {
            let addr = socket_addr(&self.device)?;
            let client =
                tokio_modbus::client::tcp::connect_slave(addr, Slave(self.device.unit_id)).await?;
            tracing::debug!("device {}: reconnected to {addr}", self.device.name);
            self.client = Some(client);
        }
        Ok(())
    }

    async fn read_samples_impl(
        &mut self,
        registers: &[RegisterConfig],
    ) -> Result<Vec<RawSample>, AcquisitionError> {
        self.ensure_connected().await?;
        let result = read_registers_from_context(
            self.client.as_mut().expect("client is present"),
            &self.device.name,
            registers,
            self.timeout,
        )
        .await;
        if result.is_err() {
            // 丢弃失效连接；下一次轮询重连。
            self.client = None;
        }
        result
    }

    async fn write_holding_register_impl(
        &mut self,
        sensor_id: &str,
        value: u16,
    ) -> Result<(), AcquisitionError> {
        self.ensure_connected().await?;
        let result = write_holding_register_from_context(
            self.client.as_mut().expect("client is present"),
            &self.device,
            sensor_id,
            value,
            self.timeout,
        )
        .await;
        if result.is_err() {
            self.client = None;
        }
        result
    }

    async fn write_single_coil_impl(
        &mut self,
        sensor_id: &str,
        value: bool,
    ) -> Result<(), AcquisitionError> {
        self.ensure_connected().await?;
        let result = write_single_coil_from_context(
            self.client.as_mut().expect("client is present"),
            &self.device,
            sensor_id,
            value,
            self.timeout,
        )
        .await;
        if result.is_err() {
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

    async fn write_holding_register(
        &mut self,
        sensor_id: &str,
        value: u16,
    ) -> Result<(), AcquisitionError> {
        self.write_holding_register_impl(sensor_id, value).await
    }

    async fn write_single_coil(
        &mut self,
        sensor_id: &str,
        value: bool,
    ) -> Result<(), AcquisitionError> {
        self.write_single_coil_impl(sensor_id, value).await
    }
}

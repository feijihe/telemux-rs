//! 基于 `tokio-modbus` + `tokio-serial` 的 Modbus-RTU 传感器数据源。
//!
//! [`ModbusRtuSource::connect`] 打开真实串口。对于测试或
//! RTU-over-TCP（串口转以太网转换器），[`ModbusRtuSource::from_stream`]
//! 可将 RTU 客户端挂到任意异步字节流上。

use std::time::Duration;

use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_modbus::client::Context;
use tokio_modbus::slave::Slave;
use tokio_serial::SerialPortBuilderExt;

use crate::acquisition::{
    read_registers_from_context, write_holding_register_from_context,
    write_single_coil_from_context, AcquisitionError, SensorSource,
};
use crate::config::{DeviceConfig, RegisterConfig};
use crate::domain::RawSample;

/// 基于串口（或通过 [`from_stream`] 任意异步流）的 RTU 传感器数据源。
pub struct ModbusRtuSource {
    device: DeviceConfig,
    client: Option<Context>,
    timeout: Duration,
}

impl ModbusRtuSource {
    /// 打开配置的串口并挂上 Modbus-RTU 客户端。
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

    /// 将 RTU 客户端挂到调用方提供的流上（测试、RTU-over-TCP）。
    /// 此类流不支持出错后重连：请改为重建数据源。
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
        // 若前一客户端已死，重开串口重连。
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

    async fn ensure_client(&mut self) -> Result<(), AcquisitionError> {
        if self.client.is_none() {
            let rebuilt = Self::connect(&self.device).await?;
            *self = rebuilt;
        }
        Ok(())
    }

    async fn write_holding_register_impl(
        &mut self,
        sensor_id: &str,
        value: u16,
    ) -> Result<(), AcquisitionError> {
        self.ensure_client().await?;
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
        self.ensure_client().await?;
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

#[async_trait]
impl SensorSource for ModbusRtuSource {
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

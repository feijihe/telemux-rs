//! 轮询调度器：每台设备一个后台任务，按间隔驱动，
//! 连接/轮询失败时指数退避。
//!
//! 设备配置（尤其是寄存器）每次轮询时从共享的 [`ConfigHandle`] 热读取，
//! 因此运行时新增的寄存器（开发仪表盘）在一个轮询间隔内即可生效，
//! 无需重启。每个轮询任务还负责服务写请求（协议层 → PCBA）。

use std::collections::HashMap;
use std::time::Duration;

use tokio::sync::{mpsc, watch};
use tracing::{debug, info, warn};

use crate::acquisition::{create_source, AcquisitionError, SensorSource};
use crate::config::DeviceConfig;
use crate::config_handle::ConfigHandle;
use crate::domain::RawSample;
use crate::protocol::{WriteBroker, WriteRequest, WriteValue};

/// 为每台配置的设备派生一个轮询任务。
pub struct AcquisitionManager {
    device_names: Vec<String>,
}

/// [`AcquisitionManager::spawn`] 的结果：轮询任务 + 供协议层
/// 向设备写寄存器的写代理。
pub struct Spawned {
    pub tasks: Vec<tokio::task::JoinHandle<()>>,
    pub broker: WriteBroker,
}

impl AcquisitionManager {
    pub fn new(handle: &ConfigHandle) -> Self {
        let device_names = handle
            .read()
            .devices
            .iter()
            .map(|d| d.name.clone())
            .collect();
        Self { device_names }
    }

    pub fn device_count(&self) -> usize {
        self.device_names.len()
    }

    /// 派生设备轮询任务。每个任务在每次成功轮询时向 `tx` 发送
    /// [`RawSample`] 批次，并通过返回的代理服务写请求。向 `shutdown`
    /// 发送 `true` 可停止所有任务。
    pub fn spawn(
        &self,
        handle: ConfigHandle,
        tx: mpsc::Sender<Vec<RawSample>>,
        shutdown: watch::Receiver<bool>,
    ) -> Spawned {
        let mut tasks = Vec::new();
        let mut write_txs = HashMap::new();
        for name in &self.device_names {
            let (write_tx, write_rx) = mpsc::channel::<WriteRequest>(16);
            write_txs.insert(name.clone(), write_tx);
            tasks.push(tokio::spawn(device_loop(
                handle.clone(),
                name.clone(),
                tx.clone(),
                shutdown.clone(),
                write_rx,
            )));
        }
        Spawned {
            tasks,
            broker: WriteBroker::new(write_txs),
        }
    }
}

async fn device_loop(
    handle: ConfigHandle,
    device_name: String,
    tx: mpsc::Sender<Vec<RawSample>>,
    mut shutdown: watch::Receiver<bool>,
    mut write_rx: mpsc::Receiver<WriteRequest>,
) {
    let mut source: Option<Box<dyn SensorSource>> = None;
    let mut failures: u64 = 0;
    let mut interval = tokio::time::interval(Duration::from_millis(1000));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut interval_ms: Option<u64> = None;

    loop {
        // 热读取当前设备配置（连接参数 + 寄存器）。
        let device: Option<DeviceConfig> = handle
            .read()
            .devices
            .into_iter()
            .find(|d| d.name == device_name);
        let Some(device) = device else {
            warn!("device {device_name}: removed from config, stopping poll task");
            return;
        };
        // 仅在轮询周期变化时重建 interval（每次迭代都重建会重置定时器，
        // 造成忙循环）。
        if interval_ms != Some(device.poll_interval_ms) {
            interval = tokio::time::interval(Duration::from_millis(device.poll_interval_ms));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            interval_ms = Some(device.poll_interval_ms);
        }
        let registers = device.registers.clone();

        // 等待下一个轮询 tick、写请求或关闭信号。
        tokio::select! {
            _ = interval.tick() => {
                match poll(&device, &mut source, &mut failures, &registers, &tx).await {
                    Ok(true) => return,          // 消费者已退出
                    Ok(false) => {}
                    Err(()) => {
                        if wait_backoff(&device, failures, &mut shutdown).await { return; }
                    }
                }
            }
            req = write_rx.recv() => {
                match req {
                    Some(req) => {
                        let result = handle_write(&device, &mut source, &req).await;
                        let _ = req.reply.send(result);
                    }
                    None => {
                        debug!("device {}: write channel closed, exiting", device.name);
                        return;
                    }
                }
            }
            _ = shutdown.changed() => {
                info!("device {}: shutting down", device.name);
                return;
            }
        }
    }
}

/// 一次轮询：确保数据源存活、读取所有寄存器、转发样本。
/// 返回 `Ok(true)` 表示循环应停止（消费者已退出）。
async fn poll(
    device: &DeviceConfig,
    source: &mut Option<Box<dyn SensorSource>>,
    failures: &mut u64,
    registers: &[crate::config::RegisterConfig],
    tx: &mpsc::Sender<Vec<RawSample>>,
) -> Result<bool, ()> {
    if source.is_none() {
        match create_source(device).await {
            Ok(s) => {
                *source = Some(s);
                *failures = 0;
                info!("device {}: connected", device.name);
            }
            Err(e) => {
                *failures += 1;
                warn!(
                    "device {}: connect failed: {e} (consecutive failures: {failures})",
                    device.name
                );
                return Err(());
            }
        }
    }

    match source
        .as_mut()
        .expect("source present")
        .read_samples(registers)
        .await
    {
        Ok(samples) => {
            *failures = 0;
            debug!("device {}: {} sample(s) read", device.name, samples.len());
            if tx.send(samples).await.is_err() {
                debug!("device {}: consumer gone, exiting", device.name);
                Ok(true)
            } else {
                Ok(false)
            }
        }
        Err(e) => {
            *failures += 1;
            warn!(
                "device {}: poll failed: {e} (consecutive failures: {failures})",
                device.name
            );
            *source = None;
            Err(())
        }
    }
}

/// 服务来自协议层的写请求。
async fn handle_write(
    device: &DeviceConfig,
    source: &mut Option<Box<dyn SensorSource>>,
    req: &WriteRequest,
) -> Result<(), AcquisitionError> {
    let Some(source) = source.as_mut() else {
        return Err(AcquisitionError::Config {
            device: device.name.clone(),
            message: "device not connected (write refused)".to_string(),
        });
    };
    match req.value {
        WriteValue::Holding(v) => source.write_holding_register(&req.sensor_id, v).await,
        WriteValue::Coil(v) => source.write_single_coil(&req.sensor_id, v).await,
    }
}

/// 指数退避休眠；返回 `true` 表示已请求关闭。
async fn wait_backoff(
    device: &DeviceConfig,
    failures: u64,
    shutdown: &mut watch::Receiver<bool>,
) -> bool {
    let base = device.reconnect_initial_ms.max(1);
    let max = device.reconnect_max_ms.max(base);
    let exponent = failures.min(10); // 限制指数防止溢出
    let delay = Duration::from_millis(base.saturating_mul(1u64 << exponent).min(max));
    tokio::select! {
        _ = tokio::time::sleep(delay) => false,
        _ = shutdown.changed() => {
            info!("device {}: shutting down during backoff", device.name);
            true
        }
    }
}

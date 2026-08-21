//! Polling scheduler: one background task per device, interval-driven,
//! with exponential backoff on connect/poll failures.
//!
//! Device config (registers in particular) is hot-read from the shared
//! [`ConfigHandle`] on every poll, so registers added at runtime (dev
//! dashboard) take effect within one poll interval without a restart.

use std::time::Duration;

use tokio::sync::{mpsc, watch};
use tracing::{debug, info, warn};

use crate::acquisition::{create_source, SensorSource};
use crate::config::DeviceConfig;
use crate::config_handle::ConfigHandle;
use crate::domain::RawSample;

/// Spawns one polling task per configured device.
pub struct AcquisitionManager {
    device_names: Vec<String>,
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

    /// Spawn device poll tasks. Each task sends batches of [`RawSample`] to
    /// `tx` on every successful poll. Sending `true` on `shutdown` stops all
    /// tasks gracefully.
    pub fn spawn(
        &self,
        handle: ConfigHandle,
        tx: mpsc::Sender<Vec<RawSample>>,
        shutdown: watch::Receiver<bool>,
    ) -> Vec<tokio::task::JoinHandle<()>> {
        self.device_names
            .iter()
            .map(|name| {
                tokio::spawn(device_loop(
                    handle.clone(),
                    name.clone(),
                    tx.clone(),
                    shutdown.clone(),
                ))
            })
            .collect()
    }
}

async fn device_loop(
    handle: ConfigHandle,
    device_name: String,
    tx: mpsc::Sender<Vec<RawSample>>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut source: Option<Box<dyn SensorSource>> = None;
    let mut failures: u64 = 0;
    let mut interval = tokio::time::interval(Duration::from_millis(1000));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut interval_ms: Option<u64> = None;

    loop {
        // Hot-read the current device config (connection params + registers).
        let device: Option<DeviceConfig> = handle
            .read()
            .devices
            .into_iter()
            .find(|d| d.name == device_name);
        let Some(device) = device else {
            warn!("device {device_name}: removed from config, stopping poll task");
            return;
        };
        // Rebuild the interval only when the poll period changes (recreating
        // it every iteration would reset the timer and busy-loop).
        if interval_ms != Some(device.poll_interval_ms) {
            interval = tokio::time::interval(Duration::from_millis(device.poll_interval_ms));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            interval_ms = Some(device.poll_interval_ms);
        }
        let registers = device.registers.clone();

        // Wait for the next poll tick or a shutdown signal.
        tokio::select! {
            _ = interval.tick() => {}
            _ = shutdown.changed() => {
                info!("device {}: shutting down", device.name);
                return;
            }
        }

        // Ensure a live source exists (connects lazily, including after errors).
        if source.is_none() {
            match create_source(&device).await {
                Ok(s) => {
                    source = Some(s);
                    failures = 0;
                    info!("device {}: connected", device.name);
                }
                Err(e) => {
                    failures += 1;
                    warn!(
                        "device {}: connect failed: {e} (consecutive failures: {failures})",
                        device.name
                    );
                    if wait_backoff(&device, failures, &mut shutdown).await {
                        return;
                    }
                    continue;
                }
            }
        }

        match source
            .as_mut()
            .expect("source present")
            .read_samples(&registers)
            .await
        {
            Ok(samples) => {
                failures = 0;
                debug!("device {}: {} sample(s) read", device.name, samples.len());
                if tx.send(samples).await.is_err() {
                    debug!("device {}: consumer gone, exiting", device.name);
                    return;
                }
            }
            Err(e) => {
                failures += 1;
                warn!(
                    "device {}: poll failed: {e} (consecutive failures: {failures})",
                    device.name
                );
                source = None;
                if wait_backoff(&device, failures, &mut shutdown).await {
                    return;
                }
            }
        }
    }
}

/// Sleep with exponential backoff; returns `true` if shutdown was requested.
async fn wait_backoff(
    device: &DeviceConfig,
    failures: u64,
    shutdown: &mut watch::Receiver<bool>,
) -> bool {
    let base = device.reconnect_initial_ms.max(1);
    let max = device.reconnect_max_ms.max(base);
    let exponent = failures.min(10); // cap the exponent to avoid overflow
    let delay = Duration::from_millis(base.saturating_mul(1u64 << exponent).min(max));
    tokio::select! {
        _ = tokio::time::sleep(delay) => false,
        _ = shutdown.changed() => {
            info!("device {}: shutting down during backoff", device.name);
            true
        }
    }
}

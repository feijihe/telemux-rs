//! Runtime configuration handle.
//!
//! Mutable in dev builds, read-only in release builds:
//!
//! - 开发环境（`debug_assertions` 或 `feature = "dev-dashboard"`）：
//!   内部为 `Arc<RwLock<Config>>`，`update()` 可热更新并可选 `save()` 写回 TOML。
//! - 生产环境（release、无 feature）：内部为 `Arc<Config>` 纯只读，`update()`/`save()`
//!   **编译期不存在** —— 引用它们的代码在生产构建下直接编译失败，保证生产不可变。
//!
//! All consumers use the uniform `read()` / `revision()` API and carry no cfg
//! branches of their own.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use crate::config::Config;

/// Whether the config is mutable in this build.
#[cfg(any(debug_assertions, feature = "dev-dashboard"))]
const MUTABLE: bool = true;
#[cfg(not(any(debug_assertions, feature = "dev-dashboard")))]
const MUTABLE: bool = false;

/// Shared handle to the runtime configuration.
#[derive(Clone)]
pub struct ConfigHandle {
    #[cfg(any(debug_assertions, feature = "dev-dashboard"))]
    inner: Arc<RwLock<Config>>,
    #[cfg(not(any(debug_assertions, feature = "dev-dashboard")))]
    inner: Arc<Config>,
    /// Bumped on every dev update; always 0 in release builds.
    revision: Arc<AtomicU64>,
    /// Config file path (dev persistence).
    #[cfg(any(debug_assertions, feature = "dev-dashboard"))]
    path: Arc<PathBuf>,
}

impl ConfigHandle {
    pub fn new(config: Config, path: PathBuf) -> Self {
        #[cfg(any(debug_assertions, feature = "dev-dashboard"))]
        {
            Self {
                inner: Arc::new(RwLock::new(config)),
                revision: Arc::new(AtomicU64::new(0)),
                path: Arc::new(path),
            }
        }
        #[cfg(not(any(debug_assertions, feature = "dev-dashboard")))]
        {
            Self {
                inner: Arc::new(config),
                revision: Arc::new(AtomicU64::new(0)),
            }
        }
    }

    /// Whether this build supports runtime config mutation.
    pub fn is_mutable(&self) -> bool {
        MUTABLE
    }

    /// Snapshot of the current configuration.
    pub fn read(&self) -> Config {
        #[cfg(any(debug_assertions, feature = "dev-dashboard"))]
        {
            self.inner.read().expect("config lock poisoned").clone()
        }
        #[cfg(not(any(debug_assertions, feature = "dev-dashboard")))]
        {
            (*self.inner).clone()
        }
    }

    /// Current configuration revision. Changes only via [`Self::update`]
    /// (dev builds); constant 0 in release builds.
    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Relaxed)
    }

    /// Mutate the configuration. The closure may fail (e.g. validation of the
    /// new register) — on error nothing is changed. Full `Config::validate()`
    /// runs before the change sticks.
    ///
    /// Only exists in dev builds; absent in release (compile-time guarantee).
    #[cfg(any(debug_assertions, feature = "dev-dashboard"))]
    pub fn update<F>(&self, f: F) -> Result<(), String>
    where
        F: FnOnce(&mut Config) -> Result<(), String>,
    {
        // Mutate and validate a candidate first; only commit on success so a
        // failed change never leaves a dirty config behind.
        let mut candidate = self.read();
        f(&mut candidate)?;
        candidate.validate().map_err(|e| e.to_string())?;
        let mut guard = self.inner.write().expect("config lock poisoned");
        *guard = candidate;
        self.revision.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Persist the current config to the TOML file (dev builds only).
    #[cfg(any(debug_assertions, feature = "dev-dashboard"))]
    pub fn save(&self) -> Result<(), String> {
        let text = toml::to_string_pretty(&self.read()).map_err(|e| e.to_string())?;
        std::fs::write(self.path.as_ref(), text).map_err(|e| e.to_string())
    }

    /// Config file path (dev builds only).
    #[cfg(any(debug_assertions, feature = "dev-dashboard"))]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

// 热更新/持久化是开发构建功能；release test（debug_assertions=false）下
// update/save 不存在，这些测试一并跳过。
#[cfg(all(test, any(debug_assertions, feature = "dev-dashboard")))]
mod tests {
    use super::*;
    use crate::config::{DeviceConfig, RegisterConfig, RegisterFunction, ValueType, WordOrder};

    fn sample_config() -> Config {
        Config {
            general: Default::default(),
            devices: vec![DeviceConfig {
                name: "pcba-01".into(),
                transport: crate::config::Transport::Tcp,
                unit_id: 1,
                host: "127.0.0.1".into(),
                port: 1502,
                poll_interval_ms: 500,
                timeout_ms: 1000,
                reconnect_initial_ms: 100,
                reconnect_max_ms: 1000,
                serial_port: None,
                baud_rate: None,
                registers: vec![RegisterConfig {
                    name: "r1".into(),
                    sensor_id: "s.1".into(),
                    function: RegisterFunction::Holding,
                    address: 0,
                    count: Some(1),
                    value_type: ValueType::U16,
                    word_order: WordOrder::Big,
                    unit: None,
                }],
            }],
            pipelines: vec![],
        }
    }

    fn new_register(name: &str, sensor_id: &str, address: u16) -> RegisterConfig {
        RegisterConfig {
            name: name.into(),
            sensor_id: sensor_id.into(),
            function: RegisterFunction::Holding,
            address,
            count: Some(1),
            value_type: ValueType::U16,
            word_order: WordOrder::Big,
            unit: None,
        }
    }

    #[test]
    fn update_applies_and_bumps_revision() {
        let handle = ConfigHandle::new(sample_config(), PathBuf::from("unused.toml"));
        let rev0 = handle.revision();
        handle
            .update(|cfg| cfg.add_register("pcba-01", new_register("r2", "s.2", 10), None))
            .unwrap();
        assert_eq!(handle.revision(), rev0 + 1);
        assert_eq!(handle.read().devices[0].registers.len(), 2);
    }

    #[test]
    fn update_failure_leaves_config_unchanged() {
        let handle = ConfigHandle::new(sample_config(), PathBuf::from("unused.toml"));
        let rev0 = handle.revision();
        let err = handle
            .update(|cfg| cfg.add_register("pcba-01", new_register("dup", "s.1", 10), None))
            .unwrap_err();
        assert!(err.contains("duplicate sensor_id"), "got: {err}");
        assert_eq!(handle.revision(), rev0, "revision must not change on failure");
        assert_eq!(handle.read().devices[0].registers.len(), 1);
    }

    #[test]
    fn update_rejects_overlapping_address() {
        let handle = ConfigHandle::new(sample_config(), PathBuf::from("unused.toml"));
        let err = handle
            .update(|cfg| cfg.add_register("pcba-01", new_register("ov", "s.9", 0), None))
            .unwrap_err();
        assert!(err.contains("overlaps"), "got: {err}");
    }

    #[test]
    fn update_rejects_unknown_device() {
        let handle = ConfigHandle::new(sample_config(), PathBuf::from("unused.toml"));
        let err = handle
            .update(|cfg| cfg.add_register("nope", new_register("r", "s.9", 10), None))
            .unwrap_err();
        assert!(err.contains("not found"), "got: {err}");
    }

    #[test]
    fn save_roundtrip() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("telemux-test-{}.toml", std::process::id()));
        let handle = ConfigHandle::new(sample_config(), path.clone());
        handle
            .update(|cfg| cfg.add_register("pcba-01", new_register("r2", "s.2", 10), None))
            .unwrap();
        handle.save().unwrap();
        let reloaded = Config::load(&path).unwrap();
        assert_eq!(reloaded.devices[0].registers.len(), 2);
        assert_eq!(reloaded.devices[0].registers[1].sensor_id, "s.2");
        let _ = std::fs::remove_file(&path);
    }
}

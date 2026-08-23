//! 运行时配置句柄。
//!
//! 开发构建可变更，release 构建只读：
//!
//! - 开发环境（`debug_assertions` 或 `feature = "dev-dashboard"`）：
//!   内部为 `Arc<RwLock<Config>>`，`update()` 可热更新并可选 `save()` 写回 TOML。
//! - 生产环境（release、无 feature）：内部为 `Arc<Config>` 纯只读，`update()`/`save()`
//!   **编译期不存在** —— 引用它们的代码在生产构建下直接编译失败，保证生产不可变。
//!
//! 所有使用者都通过统一的 `read()` / `revision()` API 访问，自身无需携带任何
//! cfg 分支。

#[cfg(any(debug_assertions, feature = "dev-dashboard"))]
use std::path::Path;
use std::path::PathBuf;
#[cfg(any(debug_assertions, feature = "dev-dashboard"))]
use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::config::Config;

/// 本次构建中配置是否可变。
#[cfg(any(debug_assertions, feature = "dev-dashboard"))]
const MUTABLE: bool = true;
#[cfg(not(any(debug_assertions, feature = "dev-dashboard")))]
const MUTABLE: bool = false;

/// 运行时配置的共享句柄。
#[derive(Clone)]
pub struct ConfigHandle {
    #[cfg(any(debug_assertions, feature = "dev-dashboard"))]
    inner: Arc<RwLock<Config>>,
    #[cfg(not(any(debug_assertions, feature = "dev-dashboard")))]
    inner: Arc<Config>,
    /// 每次开发更新时递增；release 构建恒为 0。
    revision: Arc<AtomicU64>,
    /// 配置文件路径（开发持久化）。
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
            // release：配置只读，不持久化路径（参数仅保持签名一致）。
            let _ = path;
            Self {
                inner: Arc::new(config),
                revision: Arc::new(AtomicU64::new(0)),
            }
        }
    }

    /// 本次构建是否支持运行时配置变更。
    pub fn is_mutable(&self) -> bool {
        MUTABLE
    }

    /// 当前配置的快照。
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

    /// 当前配置版本号。仅通过 [`Self::update`] 变化
    /// （开发构建）；release 构建恒为 0。
    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Relaxed)
    }

    /// 变更配置。闭包可能失败（例如新寄存器校验失败）——出错时不做任何修改。
    /// 变更生效前会执行完整的 `Config::validate()`。
    ///
    /// 仅存在于开发构建；release 中不存在（编译期保证）。
    #[cfg(any(debug_assertions, feature = "dev-dashboard"))]
    pub fn update<F>(&self, f: F) -> Result<(), String>
    where
        F: FnOnce(&mut Config) -> Result<(), String>,
    {
        // 先变更并校验候选配置；成功后才提交，失败的变更不会留下脏配置。
        let mut candidate = self.read();
        f(&mut candidate)?;
        candidate.validate().map_err(|e| e.to_string())?;
        let mut guard = self.inner.write().expect("config lock poisoned");
        *guard = candidate;
        self.revision.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// 将当前配置持久化到 TOML 文件（仅开发构建）。
    #[cfg(any(debug_assertions, feature = "dev-dashboard"))]
    pub fn save(&self) -> Result<(), String> {
        let text = toml::to_string_pretty(&self.read()).map_err(|e| e.to_string())?;
        std::fs::write(self.path.as_ref(), text).map_err(|e| e.to_string())
    }

    /// 配置文件路径（仅开发构建）。
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
                    access: crate::config::Access::Read,
                }],
            }],
            pipelines: vec![],
            computed: vec![],
            endpoints: Default::default(),
            sim: Default::default(),
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
            access: crate::config::Access::Read,
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

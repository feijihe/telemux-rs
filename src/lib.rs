//! Telmux-rs: PCBA telemetry multi-protocol gateway.
//!
//! Layers (see `docs/IMPLEMENTATION.md`):
//! - [`config`]: TOML-driven configuration
//! - [`domain`]: core types (sensor id, raw samples, metrics)
//! - [`acquisition`]: sensor acquisition layer (Modbus TCP/RTU via tokio-modbus)
//! - [`store`]: latest-value raw sample store
//! - [`logging`]: tracing + tracing-subscriber setup
//! - [`dashboard`]: dev-only web dashboard (compile-time gated, see Cargo.toml)
//! - [`mock`]: mock PCBA Modbus slave (dev-only, compile-time gated — used by
//!   tests and the `mock_pcba` example, excluded from release builds)

pub mod acquisition;
pub mod config;
pub mod config_handle;
#[cfg(any(debug_assertions, feature = "dev-dashboard"))]
pub mod dashboard;
pub mod domain;
pub mod logging;
// Mock PCBA 从站：仅开发构建编译（tests/example 使用），release 生产排除。
#[cfg(any(debug_assertions, feature = "dev-dashboard"))]
pub mod mock;
pub mod pipeline;
pub mod store;

//! Telmux-rs：PCBA 遥测多协议网关。
//!
//! 分层结构（见 `docs/IMPLEMENTATION.md`）：
//! - [`config`]：TOML 驱动的配置
//! - [`computed`]：虚拟传感器（由其他传感器派生出的指标）
//! - [`domain`]：核心类型（传感器 id、原始样本、指标）
//! - [`acquisition`]：传感器采集层（通过 tokio-modbus 的 Modbus TCP/RTU）
//! - [`store`]：最新值原始样本存储
//! - [`logging`]：tracing + tracing-subscriber 日志设置
//! - [`dashboard`]：仅开发用的 Web 仪表盘（编译期门控，见 Cargo.toml）
//! - [`mock`]：模拟 PCBA Modbus 从站（仅开发用，编译期门控——供测试和
//!   `mock_pcba` 示例使用，release 构建中排除）

pub mod acquisition;
pub mod app;
pub mod cli;
pub mod computed;
pub mod config;
pub mod config_handle;
#[cfg(any(debug_assertions, feature = "dev-dashboard"))]
pub mod dashboard;
pub mod domain;
pub mod health;
pub mod logging;
// Mock PCBA 从站：仅开发构建编译（tests/example 使用），release 生产排除。
#[cfg(any(debug_assertions, feature = "dev-dashboard"))]
pub mod mock;
pub mod pipeline;
pub mod protocol;
#[cfg(windows)]
pub mod service;
pub mod store;

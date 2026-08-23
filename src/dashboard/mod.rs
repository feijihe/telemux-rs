//! 开发仪表盘：用于查看寄存器配置和实时原始样本的本地 Web UI。
//!
//! 编译期门控——见 `Cargo.toml` 中的 `dev-dashboard` feature 和
//! `docs/DEV_DASHBOARD.md`。

pub mod server;
pub mod snapshot;

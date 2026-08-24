//! telemux-sim：CDU 仿真器库。
//!
//! - [`model`]：物理模型（SimConfig + 稳态求值）
//! - [`registers`]：Modbus 寄存器地图
//! - [`server`]：Modbus-TCP 从站
//! - [`web`]：网页 UI（观察寄存器地址/类型/原始值）

pub mod model;
pub mod registers;
pub mod server;
pub mod web;
pub mod web_assets;

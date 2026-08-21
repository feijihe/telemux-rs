//! Dev dashboard: local web UI for inspecting register configuration and
//! live raw samples.
//!
//! Compile-time gated — see the `dev-dashboard` feature in `Cargo.toml` and
//! `docs/DEV_DASHBOARD.md`.

pub mod server;
pub mod snapshot;

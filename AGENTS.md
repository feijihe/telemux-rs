# AGENTS.md

Rust **cargo workspace** (monorepo), edition 2024, single `Cargo.lock` (keep deps in root `[workspace.dependencies]`, not crate-local). Repo convention: code comments, commits, and docs are in **Chinese**.

## Layout

- `crates/telemux` — gateway: Modbus acquisition → processing pipeline → Redfish/Modbus out. The production binary.
- `crates/telemux-sim` — CDU simulator: physical model + Modbus-TCP slave + web UI. Dev/test only, fully decoupled from the gateway (gateway only talks Modbus `transport="tcp"`).
- `web/` — **pnpm workspace** (React + TS + shadcn/ui): `apps/sim-ui` (simulator UI), `apps/dashboard` (gateway dev dashboard), `packages/ui` (shared components/types). Build output lands in each crate's `web/dist`, embedded at compile time via `include_dir!` (`crates/*/src/**/web_assets.rs`); `build.rs` generates a placeholder if dist is missing.
- Docs live in `crates/telemux/docs/` (e.g. `IMPLEMENTATION.md`, `SIMULATION.md`). README says `docs/...` but there is **no root `docs/`** — look under `crates/telemux/docs/`.

## Commands (run from repo root)

```bash
cargo build --workspace           # both binaries
cargo test --workspace            # full suite incl. integration tests
cargo test -p telemux             # one crate
cargo test -p telemux --test cdu_sim   # single integration test (gateway↔sim E2E)
cargo clippy --workspace --all-targets
cargo fmt --all

# 前端（必须先构建一次，产物供 include_dir! 嵌入）
cd web && pnpm install && pnpm run build && cd ..
# 开发模式（vite 热更新，代理 /api 到对应后端）
pnpm --filter sim-ui dev          # 5180 → 8082
pnpm --filter dashboard dev       # 5181 → 8080
```

`--config` paths are relative to repo root; there is no root-default config, so pass the full path:

```bash
cargo run -p telemux -- --config crates/telemux/config/example.toml
cargo run -p telemux-sim -- --config crates/telemux-sim/config/cdu.toml --modbus-port 1502 --web-port 8082
```

Ports: gateway Redfish `8000`, Modbus server `1503`, health `8081`; sim Modbus `1502`, web UI `8082` (web binds 127.0.0.1 only).

## Architecture gotchas

- Both `main`s use `#[tokio::main(flavor = "current_thread")]` — single-threaded runtime; don't add multi-thread assumptions.
- The processing **pipeline is NOT `Send`** (`meval` stages hold `Rc`), so it runs **inline in the main loop** (`crates/telemux/src/app.rs`), not in spawned tasks. Don't try to move a pipeline into `tokio::spawn`.
- `mock` and `dashboard` modules are gated by `cfg(any(debug_assertions, feature = "dev-dashboard"))` (`src/lib.rs`). Mock PCBA and dev dashboard are **excluded from release builds**. To get the dashboard in release: `cargo build --release --features dev-dashboard`.
- Config **hot-update** is dev-only (`ConfigHandle::is_mutable()`); release config is read-only.
- Windows Service (install/uninstall/run) is `#[cfg(windows)]` only; `windows-service` dep is target-gated in `crates/telemux/Cargo.toml`. Keep Windows-specific code behind the same cfg.

## Testing notes

- Integration tests may need the sim/gateway loop; `crates/telemux/tests/cdu_sim.rs` spins up both. `crates/telemux/tests/acquisition_rtu.rs` / `acquisition_tcp.rs` use the dev-only `mock` module — these only compile in debug builds.
- RTU tests exercise real serial via `tokio-serial`; on CI/headless hosts serial may be unavailable — check whether a test requires a physical/`mock` port before assuming it runs everywhere.

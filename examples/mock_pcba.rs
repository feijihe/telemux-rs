//! Mock PCBA Modbus-TCP slave for local development.
//!
//! Usage: `cargo run --example mock_pcba [port]` (default port 1502).
//! Serves the register map that `config/example.toml` expects, with dynamic
//! values that change on every read so you can watch the gateway log live data.

use telemux::mock::MockPcba;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let port = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "1502".to_string());
    let bind = format!("127.0.0.1:{port}");
    let pcba = MockPcba::dynamic();
    let handle = pcba.spawn(&bind).await?;
    println!("mock PCBA listening on {bind} (dynamic register values)");
    println!("  holding[0]      = cpu temp raw   (u16)");
    println!("  holding[1]      = fan speed raw  (u16)");
    println!("  holding[2..4]   = uptime ticks   (u32, big)");
    println!("  holding[4..6]   = vcore          (f32, big)");
    println!("  input[0]        = vin raw        (u16)");
    println!("  input[1]        = iin raw        (u16)");
    println!("press Ctrl+C to stop");
    tokio::signal::ctrl_c().await?;
    drop(handle);
    println!("mock PCBA stopped");
    Ok(())
}

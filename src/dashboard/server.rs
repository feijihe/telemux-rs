//! Dev dashboard HTTP + WebSocket server (axum).
//!
//! Routes:
//! - `GET /`             — single-page dashboard (embedded HTML)
//! - `GET /api/snapshot` — JSON snapshot (polling fallback)
//! - `GET /api/ws`       — WebSocket, pushes a JSON snapshot every `interval`
//! - `POST /api/registers` — hot-add a register (dev builds; validated, then
//!   persisted to the config file; takes effect within one poll interval)
//!
//! Binds to `127.0.0.1:<port>` (default 8080), dev-only.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::{broadcast, watch};
use tracing::{debug, info, warn};

use crate::config::{PipelineConfig, RegisterConfig};
use crate::config_handle::ConfigHandle;
use crate::dashboard::snapshot;
use crate::store::MetricStore;

const DEFAULT_PORT: u16 = 8080;

/// Shared state for all handlers.
#[derive(Clone)]
struct DashboardState {
    config: ConfigHandle,
    store: Arc<MetricStore>,
    broadcast: broadcast::Sender<String>,
}

/// Body of `POST /api/registers`.
#[derive(Debug, Deserialize)]
pub struct CreateRegisterRequest {
    pub device: String,
    pub register: RegisterConfig,
    #[serde(default)]
    pub pipeline: Option<PipelineConfig>,
}

/// Start the dashboard server plus its broadcast task.
///
/// `shutdown` stops the server; returns the server task handle (abort on drop).
pub fn spawn(
    config: ConfigHandle,
    store: Arc<MetricStore>,
    port: Option<u16>,
    shutdown: watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if let Err(e) = run(config, store, port, shutdown).await {
            warn!("dev dashboard stopped: {e}");
        }
    })
}

async fn run(
    config: ConfigHandle,
    store: Arc<MetricStore>,
    port: Option<u16>,
    mut shutdown: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let port = port.unwrap_or(DEFAULT_PORT);
    let addr: SocketAddr = format!("127.0.0.1:{port}")
        .parse()
        .expect("static dashboard address");

    let (broadcast_tx, _) = broadcast::channel::<String>(16);

    // Broadcast interval: smallest poll interval across devices, min 250ms.
    let interval = Duration::from_millis(
        config
            .read()
            .devices
            .iter()
            .map(|d| d.poll_interval_ms)
            .min()
            .unwrap_or(1000)
            .max(250),
    );

    let state = DashboardState {
        config,
        store,
        broadcast: broadcast_tx.clone(),
    };

    let app = Router::new()
        .route("/", get(index))
        .route("/api/snapshot", get(snapshot_json))
        .route("/api/ws", get(ws_handler))
        .route("/api/registers", post(create_register))
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!(
        "dev dashboard listening on http://{addr} (register config + live raw samples)"
    );

    let broadcaster = tokio::spawn(broadcast_loop(
        state.store.clone(),
        broadcast_tx,
        interval,
        shutdown.clone(),
    ));

    tokio::select! {
        res = axum::serve(listener, app) => {
            res?;
        }
        _ = shutdown.changed() => {
            info!("dev dashboard: shutting down");
        }
    }
    broadcaster.abort();
    Ok(())
}

/// Periodically read the store and broadcast an incremental update
/// (raw + metric only) to all clients. Static config goes over HTTP.
async fn broadcast_loop(
    store: Arc<MetricStore>,
    tx: broadcast::Sender<String>,
    interval: Duration,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            _ = tokio::time::sleep(interval) => {}
            _ = shutdown.changed() => return,
        }
        let msg = snapshot::build_update(&store);
        let json = serde_json::to_string(&msg).unwrap_or_else(|_| "{}".to_string());
        // Ignore send errors: a channel with no receivers is normal.
        let _ = tx.send(json);
    }
}

async fn index() -> impl IntoResponse {
    Html(include_str!("index.html"))
}

async fn snapshot_json(State(state): State<DashboardState>) -> impl IntoResponse {
    let snap = snapshot::build_snapshot(&state.config, &state.store);
    axum::Json(snap)
}

/// Hot-add a register (dev builds). Validates, applies to the shared config,
/// and persists to the config file (best-effort).
async fn create_register(
    State(state): State<DashboardState>,
    Json(req): Json<CreateRegisterRequest>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, Json<serde_json::Value>)> {
    match state.config.update(|cfg| {
        cfg.add_register(&req.device, req.register.clone(), req.pipeline.clone())
    }) {
        Ok(()) => {
            if let Err(e) = state.config.save() {
                warn!("failed to persist config: {e} (change is active in memory)");
            }
            info!(
                "dev dashboard: register `{}` added to device `{}`",
                req.register.sensor_id, req.device
            );
            Ok(Json(json!({ "ok": true, "message": "register added" })))
        }
        Err(e) => {
            warn!("dev dashboard: register rejected: {e}");
            Err((
                axum::http::StatusCode::BAD_REQUEST,
                Json(json!({ "ok": false, "error": e })),
            ))
        }
    }
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<DashboardState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

/// Push an initial incremental update, then every broadcast update, until the
/// client disconnects or the broadcast channel closes. Full config is fetched
/// by the client over HTTP.
async fn handle_ws(mut socket: WebSocket, state: DashboardState) {
    debug!("dev dashboard: websocket client connected");
    let mut updates = state.broadcast.subscribe();

    // Initial update immediately (client already has the full table from HTTP).
    let initial = snapshot::build_update(&state.store);
    let json = serde_json::to_string(&initial).unwrap_or_else(|_| "{}".to_string());
    if socket.send(Message::Text(json.into())).await.is_err() {
        return;
    }

    let (mut sink, mut stream) = socket.split();
    loop {
        tokio::select! {
            res = updates.recv() => {
                match res {
                    Ok(json) => {
                        if sink.send(Message::Text(json.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break, // broadcaster gone
                }
            }
            msg = stream.next() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {} // ignore pings/pongs/other
                    Some(Err(_)) => break,
                }
            }
        }
    }
    debug!("dev dashboard: websocket client disconnected");
}

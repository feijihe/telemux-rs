//! 开发仪表盘 HTTP + WebSocket 服务器（axum）。
//!
//! 路由：
//! - `GET /`             — 单页仪表盘（内嵌 HTML）
//! - `GET /api/snapshot` — JSON 快照（轮询回退）
//! - `GET /api/ws`       — WebSocket，每隔 `interval` 推送一次 JSON 快照
//! - `POST /api/registers` — 热添加寄存器（开发构建；校验后
//!   持久化到配置文件；在一个轮询间隔内生效）
//!
//! 绑定到 `127.0.0.1:<port>`（默认 8080），仅开发使用。

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

/// 所有处理程序共享的状态。
#[derive(Clone)]
struct DashboardState {
    config: ConfigHandle,
    store: Arc<MetricStore>,
    broadcast: broadcast::Sender<String>,
}

/// `POST /api/registers` 的请求体。
#[derive(Debug, Deserialize)]
pub struct CreateRegisterRequest {
    pub device: String,
    pub register: RegisterConfig,
    #[serde(default)]
    pub pipeline: Option<PipelineConfig>,
}

/// 启动仪表盘服务器及其广播任务。
///
/// `shutdown` 停止服务器；返回服务器任务句柄（drop 时 abort）。
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

    // 广播间隔：所有设备中最小的轮询间隔，下限 250ms。
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

/// 周期性读取存储并向所有客户端广播增量更新
/// （仅 raw + metric）。静态配置走 HTTP。
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
        // 忽略发送错误：没有接收者的通道是正常的。
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

/// 热添加寄存器（开发构建）。校验后应用到共享配置，
/// 并持久化到配置文件（尽力而为）。
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

/// 先推送一次初始增量更新，随后推送每次广播更新，直到客户端断开或
/// 广播通道关闭。完整配置由客户端通过 HTTP 获取。
async fn handle_ws(mut socket: WebSocket, state: DashboardState) {
    debug!("dev dashboard: websocket client connected");
    let mut updates = state.broadcast.subscribe();

    // 立即发送初始更新（客户端已通过 HTTP 拿到完整表格）。
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
                    Err(_) => break, // 广播者已退出
                }
            }
            msg = stream.next() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {} // 忽略 ping/pong/其他
                    Some(Err(_)) => break,
                }
            }
        }
    }
    debug!("dev dashboard: websocket client disconnected");
}

//! 网页 UI：观察模拟器所有寄存器（地址 / 类型 / 原始值）并设置控制变量。
//!
//! - `GET /`            — React SPA（由 `web_assets` 嵌入 `web/dist` 服务）
//! - `GET /api/state`   — JSON：控制变量 + 传感器 + 寄存器地图原始值
//! - `GET /api/ws`      — WebSocket：按间隔推送状态 JSON（与 HTTP 同一份构建）
//! - `POST /api/control`— 设置控制变量（JSON `{"name","value"}`），立即驱动模型

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::server::SimSlaveState;

/// WebSocket 推送间隔（毫秒）。
const WS_INTERVAL_MS: u64 = 500;

#[derive(Clone)]
struct WebState {
    slave: Arc<SimSlaveState>,
}

/// 构建网页 UI 路由。
pub fn router(slave: Arc<SimSlaveState>) -> Router {
    let state = WebState { slave };
    Router::new()
        // React SPA：精确路径命中资源，其余回退 index.html（history 回退）。
        .route("/", get(index_or_static))
        .route("/assets/{*path}", get(index_or_static))
        .route("/{*path}", get(index_or_static))
        .route("/api/state", get(state_json))
        .route("/api/ws", get(ws_handler))
        .route("/api/control", post(set_control))
        .with_state(state)
}

/// 前端 SPA 服务（嵌入 dist + history 回退）。
async fn index_or_static(req: Request<Body>) -> impl IntoResponse {
    crate::web_assets::index_or_static(req).await
}

/// `POST /api/control` 请求体。
#[derive(Debug, Deserialize)]
struct ControlBody {
    name: String,
    value: f64,
}

/// 设置控制变量（如 primary_cold_temp / secondary_hot_temp / *_duty）。
/// 成功后模型立即重算，下一次状态推送即返回新值。
async fn set_control(
    State(state): State<WebState>,
    Json(body): Json<ControlBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let mut engine = state
        .slave
        .engine
        .lock()
        .expect("sim engine lock poisoned");
    match engine.set_control(&body.name, body.value) {
        Ok(()) => Ok(Json(json!({ "ok": true, "name": body.name, "value": body.value }))),
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": e.to_string() })),
        )),
    }
}

/// WebSocket 握手：升级后按间隔推送状态，直到客户端断开。
async fn ws_handler(ws: WebSocketUpgrade, State(state): State<WebState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| ws_push_loop(socket, state))
}

async fn ws_push_loop(socket: WebSocket, state: WebState) {
    let (mut sink, mut stream) = socket.split();
    let mut interval = tokio::time::interval(Duration::from_millis(WS_INTERVAL_MS));
    loop {
        tokio::select! {
            _ = interval.tick() => {
                let payload = build_state_json(&state.slave);
                let text = serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string());
                if sink.send(Message::Text(text.into())).await.is_err() {
                    break;
                }
            }
            msg = stream.next() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {} // 忽略 ping/pong/其它
                    Some(Err(_)) => break,
                }
            }
        }
    }
}

/// `GET /api/state`（HTTP 轮询回退）。
async fn state_json(State(state): State<WebState>) -> Json<Value> {
    Json(build_state_json(&state.slave))
}

/// 构建完整状态 JSON：控制变量 + 传感器 + 寄存器地图原始值。
/// HTTP 与 WebSocket 共用，保证两条通道数据一致。
fn build_state_json(slave: &SimSlaveState) -> Value {
    let engine = slave.engine.lock().expect("sim engine lock poisoned");
    let config = engine.config().clone();

    let controls: Vec<Value> = config
        .controls
        .iter()
        .map(|c| {
            json!({
                "name": c.name,
                "value": engine.control(&c.name).unwrap_or(c.initial),
                "unit": c.unit,
                "writable": c.writable,
            })
        })
        .collect();

    let values: std::collections::HashMap<String, f64> = engine.eval_all().into_iter().collect();
    let sensor_json = |s: &crate::model::SimSensor| {
        json!({
            "sensor_id": s.sensor_id,
            "name": s.name,
            "kind": s.kind,
            "unit": s.unit,
            "formula": s.formula,
            "value": values.get(&s.sensor_id).copied(),
        })
    };
    // 分组展示（按回路 + 出入口/辅助），供 UI 归类。
    let group_json = |side: &crate::model::Side| json!({
        "in": side.input.iter().map(&sensor_json).collect::<Vec<_>>(),
        "out": side.output.iter().map(&sensor_json).collect::<Vec<_>>(),
        "aux": side.auxiliary.iter().map(&sensor_json).collect::<Vec<_>>(),
    });
    let pri: Option<Value> = config.pri.as_ref().map(group_json);
    let sec: Option<Value> = config.sec.as_ref().map(group_json);
    // 扁平全量列表（pri + sec + 未分组），前端 canvas/表格继续按 sensor_id 使用。
    let sensors: Vec<Value> = config
        .iter_sensors()
        .map(&sensor_json)
        .collect();

    // 寄存器地图原始值：保持区（控制 + holding 传感器）+ 输入区（f32 解码 / u16 原始）。
    let holding: Vec<Value> = (0..slave.map.holding.len())
        .map(|i| {
            let slot = slave.map.holding[i].as_ref().map(|s| match s {
                crate::registers::HoldingSlot::Control { control, writable } => json!({
                    "type": "control",
                    "control": control,
                    "writable": writable,
                }),
                crate::registers::HoldingSlot::Sensor(slot) => json!({
                    "type": "sensor",
                    "sensor": slot.sensor_id,
                    "storage": match slot.storage {
                        crate::model::Storage::F32 => "f32",
                        crate::model::Storage::U16 => "u16",
                    },
                }),
            });
            json!({
                "addr": i,
                "slot": slot,
                "raw": slave.map.read_holding(&engine, i),
            })
        })
        .collect();
    let inputs: Vec<Value> = {
        let mut out = Vec::new();
        let mut i = 0;
        while i < slave.map.inputs.len() {
            let slot = match slave.map.inputs[i].as_ref() {
                Some(s) => s,
                None => {
                    i += 1;
                    continue;
                }
            };
            match slot.storage {
                crate::model::Storage::F32 => {
                    let hi = slave.map.read_input(&engine, i);
                    let lo = slave.map.read_input(&engine, i + 1);
                    let f = f32::from_bits(((hi as u32) << 16) | lo as u32);
                    out.push(json!({
                        "addr": i,
                        "sensor": slot.sensor_id.clone(),
                        "storage": "f32",
                        "raw_hi": hi,
                        "raw_lo": lo,
                        "value_f32": f,
                    }));
                    i += 2;
                }
                crate::model::Storage::U16 => {
                    let raw = slave.map.read_input(&engine, i);
                    out.push(json!({
                        "addr": i,
                        "sensor": slot.sensor_id.clone(),
                        "storage": "u16",
                        "raw_hi": raw,
                        "raw_lo": null,
                        "value_f32": null,
                    }));
                    i += 1;
                }
            }
        }
        out
    };

    json!({
        "controls": controls,
        "pri": pri,
        "sec": sec,
        "sensors": sensors,
        "holding": holding,
        "inputs": inputs,
    })
}

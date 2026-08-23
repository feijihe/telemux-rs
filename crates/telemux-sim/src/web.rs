//! 网页 UI：观察模拟器所有寄存器（地址 / 类型 / 原始值）并设置控制变量。
//!
//! - `GET /`            — 单页仪表盘（SVG 系统图 + 表格，JS 轮询刷新）
//! - `GET /api/state`   — JSON：控制变量 + 传感器 + 寄存器地图原始值
//! - `POST /api/control`— 设置控制变量（JSON `{"name","value"}`），立即驱动模型

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::server::SimSlaveState;

#[derive(Clone)]
struct WebState {
    slave: Arc<SimSlaveState>,
}

/// 构建网页 UI 路由。
pub fn router(slave: Arc<SimSlaveState>) -> Router {
    let state = WebState { slave };
    Router::new()
        .route("/", get(index))
        .route("/api/state", get(state_json))
        .route("/api/control", post(set_control))
        .with_state(state)
}

async fn index() -> impl IntoResponse {
    Html(include_str!("index.html"))
}

/// `POST /api/control` 请求体。
#[derive(Debug, Deserialize)]
struct ControlBody {
    name: String,
    value: f64,
}

/// 设置控制变量（如 primary_cold_temp / secondary_hot_temp / *_duty）。
/// 成功后模型立即重算，下一次 `GET /api/state` 即返回新值。
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

/// 当前状态 JSON：控制变量 + 传感器（含每个传感器的 f32 原始值与解码）。
async fn state_json(State(state): State<WebState>) -> Json<Value> {
    let engine = state
        .slave
        .engine
        .lock()
        .expect("sim engine lock poisoned");
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
    let sensors: Vec<Value> = config
        .sensors
        .iter()
        .map(|s| {
            json!({
                "sensor_id": s.sensor_id,
                "name": s.name,
                "kind": s.kind,
                "unit": s.unit,
                "formula": s.formula,
                "value": values.get(&s.sensor_id).copied(),
            })
        })
        .collect();

    // 寄存器地图原始值：保持区（u16）+ 输入区（f32 解码）。
    let holding: Vec<Value> = (0..state.slave.map.holding.len())
        .map(|i| {
            json!({
                "addr": i,
                "slot": state.slave.map.holding[i].as_ref().map(|s| json!({
                    "control": s.control,
                    "writable": s.writable,
                })),
                "raw": state.slave.map.read_holding(&engine, i),
            })
        })
        .collect();
    let inputs: Vec<Value> = {
        let mut out = Vec::new();
        let mut i = 0;
        while i < state.slave.map.inputs.len() {
            let hi = state.slave.map.read_input(&engine, i);
            let lo = state.slave.map.read_input(&engine, i + 1);
            let f = f32::from_bits(((hi as u32) << 16) | lo as u32);
            out.push(json!({
                "addr": i,
                "sensor": state.slave.map.inputs[i].as_ref().map(|s| s.sensor_id.clone()),
                "raw_hi": hi,
                "raw_lo": lo,
                "value_f32": f,
            }));
            i += 2;
        }
        out
    };

    Json(json!({
        "controls": controls,
        "sensors": sensors,
        "holding": holding,
        "inputs": inputs,
    }))
}

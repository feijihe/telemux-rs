//! Modbus-TCP 从站：暴露寄存器地图，接收控制变量写入。
//!
//! 复用 tokio-modbus 的 TCP 服务骨架（与网关 mock 一致）。
//! 请求处理直接读引擎状态；写保持寄存器 → 更新控制变量。

use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;

use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio_modbus::server::tcp::{accept_tcp_connection, Server as TcpModbusServer};
use tokio_modbus::server::Service;
use tokio_modbus::{ExceptionCode, Request, Response};
use tracing::{info, warn};

use crate::model::SimEngine;
use crate::registers::RegisterMap;

/// 从站共享状态：引擎 + 寄存器地图。
pub struct SimSlaveState {
    pub engine: std::sync::Mutex<SimEngine>,
    pub map: RegisterMap,
}

impl SimSlaveState {
    pub fn new(engine: SimEngine) -> Self {
        let map = RegisterMap::build(engine.config());
        Self {
            engine: std::sync::Mutex::new(engine),
            map,
        }
    }
}

pub struct SimService {
    state: Arc<SimSlaveState>,
}

impl Service for SimService {
    type Request = Request<'static>;
    type Response = Response;
    type Exception = ExceptionCode;
    type Future = Pin<Box<dyn Future<Output = Result<Response, ExceptionCode>> + Send>>;

    fn call(&self, req: Self::Request) -> Self::Future {
        let state = self.state.clone();
        Box::pin(async move { handle(state, req) })
    }
}

fn handle(state: Arc<SimSlaveState>, req: Request<'static>) -> Result<Response, ExceptionCode> {
    match req {
        Request::ReadHoldingRegisters(addr, cnt) => {
            let engine = state.engine.lock().map_err(|_| ExceptionCode::ServerDeviceFailure)?;
            let words: Vec<u16> = (0..cnt as usize)
                .map(|i| state.map.read_holding(&engine, addr as usize + i))
                .collect();
            Ok(Response::ReadHoldingRegisters(words))
        }
        Request::ReadInputRegisters(addr, cnt) => {
            let engine = state.engine.lock().map_err(|_| ExceptionCode::ServerDeviceFailure)?;
            let words: Vec<u16> = (0..cnt as usize)
                .map(|i| state.map.read_input(&engine, addr as usize + i))
                .collect();
            Ok(Response::ReadInputRegisters(words))
        }
        Request::WriteSingleRegister(addr, word) => {
            write_control(&state, addr, word)?;
            Ok(Response::WriteSingleRegister(addr, word))
        }
        Request::WriteMultipleRegisters(addr, words) => {
            for (i, w) in words.iter().enumerate() {
                write_control(&state, addr.wrapping_add(i as u16), *w)?;
            }
            Ok(Response::WriteMultipleRegisters(addr, words.len() as u16))
        }
        // 无线圈/离散输入。
        Request::ReadCoils(..) | Request::ReadDiscreteInputs(..) | Request::WriteSingleCoil(..) => {
            Err(ExceptionCode::IllegalFunction)
        }
        _ => Err(ExceptionCode::IllegalFunction),
    }
}

fn write_control(
    state: &SimSlaveState,
    addr: u16,
    word: u16,
) -> Result<(), ExceptionCode> {
    let slot = state
        .map
        .holding
        .get(addr as usize)
        .and_then(|s| s.as_ref())
        .ok_or(ExceptionCode::IllegalDataAddress)?;
    // 保持区的传感器槽位只读（对齐真实 CDU 中 read_holding_registers 的测量点）。
    let crate::registers::HoldingSlot::Control { control, writable } = slot else {
        return Err(ExceptionCode::IllegalFunction);
    };
    if !writable {
        return Err(ExceptionCode::IllegalFunction);
    }
    // 控制变量值域 0-100（duty %）。
    let value = f64::from(word).clamp(0.0, 100.0);
    state
        .engine
        .lock()
        .map_err(|_| ExceptionCode::ServerDeviceFailure)?
        .set_control(control, value)
        .map_err(|e| {
            warn!("sim: write control `{}`: {e}", control);
            ExceptionCode::IllegalDataValue
        })
}

/// 启动 Modbus-TCP 从站；`shutdown` 触发时返回。
pub async fn run(
    state: Arc<SimSlaveState>,
    port: u16,
    mut shutdown: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let addr: SocketAddr = format!("0.0.0.0:{port}").parse()?;
    let listener = TcpListener::bind(addr).await?;
    let server = TcpModbusServer::new(listener);
    info!("telemux-sim modbus listening on {addr}");

    let on_connected = {
        let state = state.clone();
        move |stream: TcpStream, socket_addr: SocketAddr| {
            let state = state.clone();
            async move {
                accept_tcp_connection(stream, socket_addr, move |_| {
                    Ok(Some(SimService { state: state.clone() }))
                })
            }
        }
    };
    let abort = async move {
        let _ = shutdown.changed().await;
    };
    let _ = server
        .serve_until(&on_connected, |e| warn!("telemux-sim modbus: {e}"), abort)
        .await?;
    Ok(())
}

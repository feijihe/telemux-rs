//! Mock PCBA Modbus-TCP slave for development and integration tests,
//! implemented on top of the `tokio-modbus` TCP server skeleton.
//!
//! - [`MockPcba::fixed`]: deterministic values, for exact assertions in tests.
//! - [`MockPcba::dynamic`]: values change with every read request, for manual
//!   demos against `config/example.toml`.
//!
//! Supports read holding/input registers plus write single/multiple echoes;
//! anything else gets an illegal-function exception.

use std::future::{self, Ready};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::net::{TcpListener, TcpStream};
use tokio_modbus::server::tcp::{accept_tcp_connection, Server as TcpModbusServer};
use tokio_modbus::server::Service;
use tokio_modbus::{ExceptionCode, Request, Response};

/// In-memory Modbus-TCP slave.
#[derive(Clone)]
pub struct MockPcba {
    /// holding register map, indexed by address
    holding: Arc<Mutex<Vec<u16>>>,
    /// input register map, indexed by address
    input: Arc<Mutex<Vec<u16>>>,
    /// generate changing values on each read (ignores the maps)
    dynamic: bool,
    /// increments per read request; drives dynamic values
    request_count: Arc<AtomicU64>,
}

impl MockPcba {
    /// Deterministic register map:
    /// holding[0]=0x1234 (u16), holding[1]=0xFFFF (i16=-1),
    /// holding[2..4]=0xDEADBEEF (u32 big), holding[4..6]=12.5 (f32 big),
    /// input[0]=200, input[1]=0xFFFF (i16=-1).
    pub fn fixed() -> Self {
        let mut holding = vec![0u16; 8];
        holding[0] = 0x1234;
        holding[1] = 0xFFFF;
        holding[2] = 0xDEAD;
        holding[3] = 0xBEEF;
        holding[4] = 0x4148; // 12.5f32 = 0x41480000
        holding[5] = 0x0000;
        let mut input = vec![0u16; 4];
        input[0] = 200;
        input[1] = 0xFFFF;
        Self::from_maps(holding, input)
    }

    /// Dynamic register map matching `config/example.toml`:
    /// holding[0]=cpu temp raw 250..299, holding[1]=fan speed raw,
    /// holding[2..4]=uptime ticks (u32), holding[4..6]=vcore 12.5 (f32),
    /// input[0]=vin raw, input[1]=iin raw.
    pub fn dynamic() -> Self {
        Self::from_maps(vec![0u16; 8], vec![0u16; 4]).with_dynamic()
    }

    pub fn from_maps(holding: Vec<u16>, input: Vec<u16>) -> Self {
        Self {
            holding: Arc::new(Mutex::new(holding)),
            input: Arc::new(Mutex::new(input)),
            dynamic: false,
            request_count: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn with_dynamic(mut self) -> Self {
        self.dynamic = true;
        self
    }

    /// Bind and serve. `bind` is e.g. "127.0.0.1:0" (ephemeral port) —
    /// the actual address is in [`MockHandle::addr`].
    pub async fn spawn(&self, bind: &str) -> std::io::Result<MockHandle> {
        let listener = TcpListener::bind(bind).await?;
        let addr = listener.local_addr()?;
        let server = TcpModbusServer::new(listener);
        let service = Arc::new(self.clone());

        let on_connected = {
            let service = service.clone();
            move |stream: TcpStream, socket_addr: SocketAddr| {
                let service = service.clone();
                async move {
                    accept_tcp_connection(stream, socket_addr, move |_| Ok(Some(service.clone())))
                }
            }
        };

        let task = tokio::spawn(async move {
            let result = server
                .serve(&on_connected, |e| {
                    eprintln!("mock pcba: request processing error: {e}");
                })
                .await;
            if let Err(e) = result {
                eprintln!("mock pcba: server stopped: {e}");
            }
        });
        Ok(MockHandle { addr, task })
    }

    fn words(&self, holding: bool, addr: u16, cnt: u16) -> Vec<u16> {
        // One snapshot per request so multi-register values stay consistent.
        let snapshot = self.request_count.fetch_add(1, Ordering::Relaxed) + 1;
        if self.dynamic {
            (0..cnt)
                .map(|i| dynamic_value(holding, addr.wrapping_add(i), snapshot))
                .collect()
        } else {
            let map = if holding {
                self.holding.lock().expect("mock map poisoned")
            } else {
                self.input.lock().expect("mock map poisoned")
            };
            (0..cnt)
                .map(|i| {
                    map.get(usize::from(addr.wrapping_add(i)))
                        .copied()
                        .unwrap_or(0)
                })
                .collect()
        }
    }
}

impl Service for MockPcba {
    type Request = Request<'static>;
    type Response = Response;
    type Exception = ExceptionCode;
    type Future = Ready<Result<Response, ExceptionCode>>;

    fn call(&self, req: Self::Request) -> Self::Future {
        let response = match req {
            Request::ReadHoldingRegisters(addr, cnt) => {
                Response::ReadHoldingRegisters(self.words(true, addr, cnt))
            }
            Request::ReadInputRegisters(addr, cnt) => {
                Response::ReadInputRegisters(self.words(false, addr, cnt))
            }
            Request::WriteSingleRegister(addr, word) => Response::WriteSingleRegister(addr, word),
            Request::WriteMultipleRegisters(addr, words) => {
                Response::WriteMultipleRegisters(addr, words.len() as u16)
            }
            _ => return future::ready(Err(ExceptionCode::IllegalFunction)),
        };
        future::ready(Ok(response))
    }
}

/// A running mock server; aborting it also cancels all active connections
/// (the tokio-modbus server uses a cancellation token for spawned tasks).
pub struct MockHandle {
    pub addr: SocketAddr,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for MockHandle {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Values for the dynamic map, keyed on request snapshot so they evolve over time.
fn dynamic_value(holding: bool, addr: u16, snapshot: u64) -> u16 {
    match (holding, addr) {
        (true, 0) => (250 + (snapshot % 50)) as u16,   // cpu temp raw 250..299
        (true, 1) => (3200 + (snapshot % 200)) as u16, // fan speed raw
        (true, 2) => ((snapshot >> 16) & 0xFFFF) as u16, // uptime ticks, high word
        (true, 3) => (snapshot & 0xFFFF) as u16,       // uptime ticks, low word
        (true, 4) => (12.5f32.to_bits() >> 16) as u16, // vcore, high word
        (true, 5) => (12.5f32.to_bits() & 0xFFFF) as u16, // vcore, low word
        (false, 0) => (12500 + (snapshot % 10)) as u16, // vin raw (mV)
        (false, 1) => (900 + (snapshot % 20)) as u16,  // iin raw (mA)
        _ => 0,
    }
}

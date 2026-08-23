//! 用于开发和集成测试的模拟 PCBA Modbus-TCP 从站，
//! 基于 `tokio-modbus` 的 TCP 服务器骨架实现。
//!
//! - [`MockPcba::fixed`]：确定性值，供测试中的精确断言使用。
//! - [`MockPcba::dynamic`]：每次读取请求值都变化，供对照
//!   `config/example.toml` 的手动演示使用。
//!
//! 支持读取保持/输入寄存器以及写入单/多寄存器回显；
//! 其他请求返回 illegal-function 异常。

use std::future::{self, Ready};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::net::{TcpListener, TcpStream};
use tokio_modbus::server::tcp::{accept_tcp_connection, Server as TcpModbusServer};
use tokio_modbus::server::Service;
use tokio_modbus::{ExceptionCode, Request, Response};

/// 内存中的 Modbus-TCP 从站。
#[derive(Clone)]
pub struct MockPcba {
    /// 保持寄存器映射，按地址索引
    holding: Arc<Mutex<Vec<u16>>>,
    /// 输入寄存器映射，按地址索引
    input: Arc<Mutex<Vec<u16>>>,
    /// 每次读取生成变化的值（忽略映射）
    dynamic: bool,
    /// 每次读取请求递增；驱动动态值
    request_count: Arc<AtomicU64>,
    /// 线圈映射（可写入的 bit）
    coils: Arc<Mutex<Vec<bool>>>,
}

impl MockPcba {
    /// 确定性寄存器映射：
    /// holding[0]=0x1234 (u16), holding[1]=0xFFFF (i16=-1),
    /// holding[2..4]=0xDEADBEEF (u32 big), holding[4..6]=12.5 (f32 big),
    /// input[0]=200, input[1]=0xFFFF (i16=-1)。
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

    /// 与 `config/example.toml` 匹配的动态寄存器映射：
    /// holding[0]=cpu 温度原始值 250..299, holding[1]=风扇转速原始值,
    /// holding[2..4]=运行时长 ticks (u32), holding[4..6]=vcore 12.5 (f32),
    /// input[0]=vin 原始值, input[1]=iin 原始值。
    pub fn dynamic() -> Self {
        Self::from_maps(vec![0u16; 8], vec![0u16; 4]).with_dynamic()
    }

    pub fn from_maps(holding: Vec<u16>, input: Vec<u16>) -> Self {
        Self {
            holding: Arc::new(Mutex::new(holding)),
            input: Arc::new(Mutex::new(input)),
            dynamic: false,
            request_count: Arc::new(AtomicU64::new(0)),
            coils: Arc::new(Mutex::new(vec![false; 8])),
        }
    }

    pub fn with_dynamic(mut self) -> Self {
        self.dynamic = true;
        self
    }

    /// 绑定并服务。`bind` 例如 "127.0.0.1:0"（临时端口）——
    /// 实际地址在 [`MockHandle::addr`] 中。
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
        // 每个请求一次快照，保证多寄存器值一致。
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

    fn bits(&self, coils: bool, addr: u16, cnt: u16) -> Vec<bool> {
        let map = if coils {
            self.coils.lock().expect("mock map poisoned")
        } else {
            // 离散输入：固定 bit 模式（地址 0 = true，其余 false）
            return (0..cnt)
                .map(|i| addr.wrapping_add(i) == 0)
                .collect();
        };
        (0..cnt)
            .map(|i| {
                map.get(usize::from(addr.wrapping_add(i)))
                    .copied()
                    .unwrap_or(false)
            })
            .collect()
    }

    fn write_holding(&self, addr: u16, word: u16) {
        let mut map = self.holding.lock().expect("mock map poisoned");
        let i = usize::from(addr);
        if i < map.len() {
            map[i] = word;
        }
    }

    fn write_coil(&self, addr: u16, value: bool) {
        let mut map = self.coils.lock().expect("mock map poisoned");
        let i = usize::from(addr);
        if i < map.len() {
            map[i] = value;
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
            Request::ReadCoils(addr, cnt) => {
                Response::ReadCoils(self.bits(true, addr, cnt))
            }
            Request::ReadDiscreteInputs(addr, cnt) => {
                Response::ReadDiscreteInputs(self.bits(false, addr, cnt))
            }
            Request::WriteSingleRegister(addr, word) => {
                self.write_holding(addr, word);
                Response::WriteSingleRegister(addr, word)
            }
            Request::WriteMultipleRegisters(addr, words) => {
                for (i, w) in words.iter().enumerate() {
                    self.write_holding(addr.wrapping_add(i as u16), *w);
                }
                Response::WriteMultipleRegisters(addr, words.len() as u16)
            }
            Request::WriteSingleCoil(addr, coil) => {
                self.write_coil(addr, coil);
                Response::WriteSingleCoil(addr, coil)
            }
            _ => return future::ready(Err(ExceptionCode::IllegalFunction)),
        };
        future::ready(Ok(response))
    }
}

/// 一个正在运行的模拟服务器；abort 它也会取消所有活动连接
/// （tokio-modbus 服务器对派生任务使用取消令牌）。
pub struct MockHandle {
    pub addr: SocketAddr,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for MockHandle {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// 动态映射的值，基于请求快照，随时间演化。
fn dynamic_value(holding: bool, addr: u16, snapshot: u64) -> u16 {
    match (holding, addr) {
        (true, 0) => (250 + (snapshot % 50)) as u16,   // cpu 温度原始值 250..299
        (true, 1) => (3200 + (snapshot % 200)) as u16, // 风扇转速原始值
        (true, 2) => ((snapshot >> 16) & 0xFFFF) as u16, // 运行时长 ticks，高字
        (true, 3) => (snapshot & 0xFFFF) as u16,       // 运行时长 ticks，低字
        (true, 4) => (12.5f32.to_bits() >> 16) as u16, // vcore，高字
        (true, 5) => (12.5f32.to_bits() & 0xFFFF) as u16, // vcore，低字
        (false, 0) => (12500 + (snapshot % 10)) as u16, // vin 原始值 (mV)
        (false, 1) => (900 + (snapshot % 20)) as u16,  // iin 原始值 (mA)
        _ => 0,
    }
}

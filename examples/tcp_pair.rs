//! TCP + PSICOSE on the host (same pattern as `boards/esp32-wifi`).
//!
//! ```text
//! TcpStream (non-blocking) → LinkFace → Pump
//! ```
//!
//! ## Modes
//!
//! ```sh
//! cargo run --example tcp_pair
//! # two peers on 127.0.0.1 (local smoke test)
//!
//! cargo run --example tcp_pair -- --listen 0.0.0.0:19876
//! # receive-only server for an ESP32 initiator
//!
//! cargo run --example tcp_pair -- --connect 192.168.1.20:19876
//! # initiator client toward a listening peer
//! ```

use std::env;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

use psicose::{ByteTransport, LinkFace, PumpEvent, TxState};

const PING: &[u8] = b"ping";
const LOCAL: &str = "127.0.0.1:19876";

struct TcpPort {
    stream: TcpStream,
}

impl TcpPort {
    fn new(stream: TcpStream) -> io::Result<Self> {
        stream.set_nonblocking(true)?;
        stream.set_nodelay(true)?;
        Ok(TcpPort { stream })
    }
}

impl ByteTransport for TcpPort {
    type Error = io::Error;

    fn write_byte(&mut self, byte: u8) -> Result<(), Self::Error> {
        loop {
            match self.stream.write(&[byte]) {
                Ok(0) => return Err(io::Error::new(io::ErrorKind::WriteZero, "tcp closed")),
                Ok(_) => return Ok(()),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_micros(50));
                }
                Err(e) => return Err(e),
            }
        }
    }

    fn read_byte(&mut self) -> Result<Option<u8>, Self::Error> {
        let mut buf = [0u8; 1];
        match self.stream.read(&mut buf) {
            Ok(0) => Ok(None),
            Ok(_) => Ok(Some(buf[0])),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(e),
        }
    }
}

fn run_peer(stream: TcpStream, initiator: bool, label: &str) {
    let port = match TcpPort::new(stream) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{label}: tcp setup failed: {e}");
            return;
        }
    };
    let face = LinkFace::new(port);
    let mut pump = face.pump();

    let mut need_start = initiator;
    let mut out_i = 0usize;
    let mut finish_offered = false;
    let mut got = [0u8; 4];
    let mut got_n = 0usize;
    let mut rounds = 0u32;

    for _ in 0..2_000_000 {
        if initiator {
            let st = pump.sender().state();
            if matches!(
                st,
                TxState::Idle | TxState::Finished | TxState::Aborted | TxState::Failed
            ) {
                if need_start {
                    if pump.sender_mut().offer_start().is_ok() {
                        need_start = false;
                        out_i = 0;
                        finish_offered = false;
                        got_n = 0;
                    }
                } else if st == TxState::Idle {
                    if out_i < PING.len() {
                        if pump.sender_mut().offer(PING[out_i]).is_ok() {
                            out_i += 1;
                        }
                    } else if !finish_offered {
                        if pump.sender_mut().offer_finish().is_ok() {
                            finish_offered = true;
                        }
                    }
                }
            }
        }

        match pump.poll() {
            Ok(PumpEvent::Received(b)) => {
                if got_n < got.len() {
                    got[got_n] = b;
                    got_n += 1;
                }
                if got_n == PING.len() {
                    println!(
                        "tcp_pair[{label}]: got {:?}",
                        core::str::from_utf8(&got[..got_n]).unwrap_or("?")
                    );
                    got_n = 0;
                }
            }
            Ok(PumpEvent::Completed) => {
                println!("tcp_pair[{label}]: session completed");
                rounds += 1;
                if rounds >= 1 {
                    return;
                }
                if initiator {
                    need_start = true;
                }
            }
            Ok(PumpEvent::Aborted) => {
                eprintln!("tcp_pair[{label}]: aborted");
                if initiator {
                    need_start = true;
                }
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("tcp_pair[{label}]: poll err {e:?}");
                let _ = pump.abort();
                return;
            }
        }
    }
    eprintln!("tcp_pair[{label}]: stalled");
}

fn usage() -> ! {
    eprintln!(
        "usage:\n  tcp_pair\n  tcp_pair --listen [ADDR]\n  tcp_pair --connect ADDR"
    );
    std::process::exit(2);
}

fn main() {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        None => {
            let listener = TcpListener::bind(LOCAL).expect("bind");
            let client = thread::spawn(|| {
                thread::sleep(Duration::from_millis(50));
                let stream = TcpStream::connect(LOCAL).expect("connect");
                run_peer(stream, true, "client");
            });
            let (server, _) = listener.accept().expect("accept");
            run_peer(server, false, "server");
            client.join().expect("client thread");
            println!("tcp_pair: ok");
        }
        Some("--listen") => {
            let addr = args.next().unwrap_or_else(|| LOCAL.to_string());
            println!("tcp_pair: listening on {addr} (receive-only)");
            let listener = TcpListener::bind(&addr).expect("bind");
            let (stream, peer) = listener.accept().expect("accept");
            println!("tcp_pair: accepted {peer}");
            run_peer(stream, false, "listen");
            println!("tcp_pair: ok");
        }
        Some("--connect") => {
            let addr = args.next().unwrap_or_else(|| usage());
            println!("tcp_pair: connecting to {addr} (initiator)");
            let stream = TcpStream::connect(&addr).expect("connect");
            run_peer(stream, true, "connect");
            println!("tcp_pair: ok");
        }
        Some(_) => usage(),
    }
}

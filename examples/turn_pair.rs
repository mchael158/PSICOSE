//! Both TCP peers send, one PSICOSE session at a time.
//!
//! ```text
//! client (first)  -- ping -->  server
//! client          <-- pong --  server
//! ```
//!
//! `Turn` on a [`LinkFace`] is what keeps the two `START`s from overlapping.
//!
//! ```sh
//! cargo run --example turn_pair
//! # both peers on 127.0.0.1
//!
//! cargo run --example turn_pair -- --listen 0.0.0.0:19876
//! # answers `pong` — peer for boards/esp32-wifi (FIRST = true)
//!
//! cargo run --example turn_pair -- --connect ADDR
//! # opens with `ping`
//! ```

use std::env;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

use psicose::{ByteTransport, LinkFace, TurnEvent};

const LOCAL: &str = "127.0.0.1:19878";

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

fn run_peer(stream: TcpStream, first: bool, label: &str, payload: &[u8]) {
    let port = TcpPort::new(stream).unwrap_or_else(|e| panic!("{label}: {e}"));
    let face = LinkFace::new(port);
    let mut turn = face.turn::<8>(first);
    assert_eq!(turn.push(payload), payload.len());

    let mut got = [0u8; 4];
    let mut n = 0usize;
    let mut yielded = false;
    let mut granted = false;

    for _ in 0..2_000_000 {
        match turn.poll() {
            Ok(TurnEvent::Received(b)) => {
                if n < got.len() {
                    got[n] = b;
                    n += 1;
                }
            }
            Ok(TurnEvent::Yielded) => {
                println!("turn_pair[{label}]: sent");
                yielded = true;
            }
            Ok(TurnEvent::Granted) => {
                println!("turn_pair[{label}]: our turn");
                granted = true;
            }
            Ok(TurnEvent::Idle) => {}
            Err(e) => panic!("{label}: {e:?}"),
        }
        // `Granted` means we already wrote the ACK for the peer's FINISH.
        // Leaving earlier drops the socket before that ACK (Windows 10053).
        if yielded && granted && n == payload.len() {
            println!(
                "turn_pair[{label}]: got {:?}",
                core::str::from_utf8(&got[..n]).unwrap_or("?")
            );
            return;
        }
    }
    panic!("{label}: stalled");
}

fn usage() -> ! {
    eprintln!("usage:\n  turn_pair\n  turn_pair --listen [ADDR]\n  turn_pair --connect ADDR");
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
                run_peer(stream, true, "client", b"ping");
            });
            let (server, _) = listener.accept().expect("accept");
            run_peer(server, false, "server", b"pong");
            client.join().expect("client thread");
            println!("turn_pair: ok");
        }
        Some("--listen") => {
            let addr = args.next().unwrap_or_else(|| "0.0.0.0:19876".to_string());
            println!("turn_pair: listening on {addr} (answers pong)");
            let listener = TcpListener::bind(&addr).expect("bind");
            let (stream, peer) = listener.accept().expect("accept");
            println!("turn_pair: accepted {peer}");
            run_peer(stream, false, "listen", b"pong");
            println!("turn_pair: ok");
        }
        Some("--connect") => {
            let addr = args.next().unwrap_or_else(|| usage());
            println!("turn_pair: connecting to {addr} (opens ping)");
            let stream = TcpStream::connect(&addr).expect("connect");
            run_peer(stream, true, "connect", b"ping");
            println!("turn_pair: ok");
        }
        Some(_) => usage(),
    }
}

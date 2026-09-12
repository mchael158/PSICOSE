//! Transport path through the **PSICOSE motor** (`Wire::copy`).
//!
//! No external P2P. No hand-rolled UART ring. A sends `ping`, B sends `pong`.
//!
//! ```text
//! exemplos ──► psicose::Wire::copy ──► Pump / CRC / ACK
//! ```
//!
//! Run: `cargo run --example ab_direct`

use psicose::{SliceSink, SliceSource, Wire};

fn main() {
    let ping = b"ping";
    let mut src = SliceSource::new(ping);
    let mut board = [0u8; 8];
    let mut sink = SliceSink::new(&mut board);
    let n = match Wire::new().copy(&mut src, &mut sink) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("ab_direct: A→B failed: {e}");
            std::process::exit(1);
        }
    };
    assert_eq!(n, ping.len());
    assert_eq!(sink.written(), ping.as_slice());
    println!(
        "ab_direct: A -> B  {:?}  {n} B",
        core::str::from_utf8(ping).unwrap_or("?")
    );

    let pong = b"pong";
    let mut src = SliceSource::new(pong);
    let mut board = [0u8; 8];
    let mut sink = SliceSink::new(&mut board);
    let n = match Wire::new().copy(&mut src, &mut sink) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("ab_direct: B→A failed: {e}");
            std::process::exit(1);
        }
    };
    assert_eq!(n, pong.len());
    assert_eq!(sink.written(), pong.as_slice());
    println!(
        "ab_direct: B -> A  {:?}  {n} B",
        core::str::from_utf8(pong).unwrap_or("?")
    );
}

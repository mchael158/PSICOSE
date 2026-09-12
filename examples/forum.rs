//! Alice ↔ Bob: payload bytes on a live P2P link.
//!
//! This is not a forum. It is the smallest picture of the P2P layer:
//! handshake, then information A→B and B→A, one cooperative poll at a
//! time. The crate never owns the blob. The 4-byte frame never sees a
//! peer.
//!
//! ```text
//! alice  --PeerLink-->  Wire  --PeerLink-->  bob
//!              ping  ------->
//!                    <-------  pong
//! ```
//!
//! Run: `cargo run --example forum`

#[path = "common/link.rs"]
mod link;

use psicose::prelude::*;

fn main() {
    let cfg = SessionConfig::FORUM;
    let wire = Wire::new();
    let (pump_a, pump_b) = wire.pumps();

    let mut alice = PeerTable::<4>::with(PeerId::from_label(b"alice"), cfg);
    let mut bob = PeerTable::<4>::with(PeerId::from_label(b"bob"), cfg);

    let mut a = match PeerLink::connect(&mut alice, pump_a) {
        Ok(link) => link,
        Err(_) => fail("connect failed"),
    };
    let mut b = PeerLink::accept(&bob, pump_b);

    if !link::established(&mut a, &mut alice, &mut b, &mut bob) {
        fail("handshake failed");
    }

    let mut board = [0u8; 32];
    let mut inbox = Defragmenter::new(&mut board);

    let before = b.pump().stats();
    if !link::send_message(&mut a, &mut alice, &mut b, &mut bob, 1, b"ping", &mut inbox) {
        fail("ping did not arrive");
    }
    report("alice -> bob", inbox.as_slice(), b.pump().stats(), before);

    let before = a.pump().stats();
    if !link::send_message(&mut b, &mut bob, &mut a, &mut alice, 2, b"pong", &mut inbox) {
        fail("pong did not arrive");
    }
    report("bob -> alice", inbox.as_slice(), a.pump().stats(), before);
}

fn fail(what: &str) -> ! {
    eprintln!("p2p: {what}");
    std::process::exit(1);
}

fn report(path: &str, payload: &[u8], after: SessionStats, before: SessionStats) {
    let ticks = after.ticks.saturating_sub(before.ticks);
    let retries = after.retries.saturating_sub(before.retries);
    let n = payload.len();
    match core::str::from_utf8(payload) {
        Ok(s) => println!("p2p: {path}  {s:?}  {n} B  ticks={ticks}  retries={retries}"),
        Err(_) => println!("p2p: {path}  {n} B  ticks={ticks}  retries={retries}"),
    }
}

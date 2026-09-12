//! P2P path through the **PSICOSE motor** (`Node` + `PeerLink`).
//!
//! ```text
//! exemplos ──► psicose::Node::connect/accept ──► PeerLink ──► Wire
//! ```
//!
//! You do not create an external P2P stack and pass a connection in.
//!
//! Run: `cargo run --example p2p_pair`

use psicose::prelude::*;

fn main() {
    let cfg = SessionConfig::FORUM;
    let wire = Wire::new();
    let (pump_a, pump_b) = wire.link_pumps();

    let mut alice = Node::<4>::with(PeerId::from_label(b"alice"), cfg);
    let mut bob = Node::<4>::with(PeerId::from_label(b"bob"), cfg);

    let mut a = match alice.connect(pump_a) {
        Ok(link) => link,
        Err(_) => fail("connect failed"),
    };
    let mut b = bob.accept(pump_b);

    if !establish(&mut a, &mut alice, &mut b, &mut bob) {
        fail("handshake failed");
    }

    let mut board = [0u8; 32];
    let mut inbox = Defragmenter::new(&mut board);

    let before = b.pump().stats();
    if !send_message(&mut a, &mut alice, &mut b, &mut bob, 1, b"ping", &mut inbox) {
        fail("ping did not arrive");
    }
    report("alice -> bob", inbox.as_slice(), b.pump().stats(), before);

    let before = a.pump().stats();
    if !send_message(&mut b, &mut bob, &mut a, &mut alice, 2, b"pong", &mut inbox) {
        fail("pong did not arrive");
    }
    report("bob -> alice", inbox.as_slice(), a.pump().stats(), before);
}

fn fail(what: &str) -> ! {
    eprintln!("p2p_pair: {what}");
    std::process::exit(1);
}

fn report(path: &str, payload: &[u8], after: SessionStats, before: SessionStats) {
    let ticks = after.ticks.saturating_sub(before.ticks);
    let retries = after.retries.saturating_sub(before.retries);
    let n = payload.len();
    match core::str::from_utf8(payload) {
        Ok(s) => println!("p2p_pair: {path}  {s:?}  {n} B  ticks={ticks}  retries={retries}"),
        Err(_) => println!("p2p_pair: {path}  {n} B  ticks={ticks}  retries={retries}"),
    }
}

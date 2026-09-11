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

use psicose::prelude::*;

type Link<'w> = PeerLink<DuplexPort<'w>, DuplexPort<'w>>;

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

    if !established(&mut a, &mut alice, &mut b, &mut bob) {
        fail("handshake failed");
    }

    let mut board = [0u8; 32];
    let mut inbox = Defragmenter::new(&mut board);

    let before = b.pump().stats();
    if !send(&mut a, &mut alice, &mut b, &mut bob, 1, b"ping", &mut inbox) {
        fail("ping did not arrive");
    }
    report("alice -> bob", inbox.as_slice(), b.pump().stats(), before);

    let before = a.pump().stats();
    if !send(&mut b, &mut bob, &mut a, &mut alice, 2, b"pong", &mut inbox) {
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

fn established<const N: usize>(
    a: &mut Link<'_>,
    alice: &mut PeerTable<N>,
    b: &mut Link<'_>,
    bob: &mut PeerTable<N>,
) -> bool {
    let mut a_ok = false;
    let mut b_ok = false;
    let mut i = 0usize;
    while i < 100_000 {
        i += 1;
        match a.poll(alice) {
            Ok(LinkEvent::Established) => a_ok = true,
            Ok(LinkEvent::Aborted) | Err(_) => return false,
            Ok(_) => {}
        }
        match b.poll(bob) {
            Ok(LinkEvent::Established) => b_ok = true,
            Ok(LinkEvent::Aborted) | Err(_) => return false,
            Ok(_) => {}
        }
        if a_ok && b_ok {
            return true;
        }
    }
    false
}

fn send<const N: usize>(
    tx: &mut Link<'_>,
    tx_table: &mut PeerTable<N>,
    rx: &mut Link<'_>,
    rx_table: &mut PeerTable<N>,
    id: u16,
    body: &[u8],
    inbox: &mut Defragmenter<'_>,
) -> bool {
    inbox.reset();
    let mut frag = Fragmenter::new(StreamId::FORUM, MessageId::new(id), body, 4);
    while let Some((header, chunk)) = frag.next_fragment() {
        for b in header.to_bytes() {
            if !offer(tx, tx_table, rx, rx_table, b, inbox) {
                return false;
            }
        }
        for &b in chunk {
            if !offer(tx, tx_table, rx, rx_table, b, inbox) {
                return false;
            }
        }
    }
    inbox.is_complete()
}

fn offer<const N: usize>(
    tx: &mut Link<'_>,
    tx_table: &mut PeerTable<N>,
    rx: &mut Link<'_>,
    rx_table: &mut PeerTable<N>,
    byte: u8,
    inbox: &mut Defragmenter<'_>,
) -> bool {
    let mut hold = Some(byte);
    let mut steps = 0usize;
    while steps < 20_000 {
        steps += 1;
        if let Some(b) = hold {
            if tx.offer(b).is_ok() {
                hold = None;
            }
        }
        match tx.poll(tx_table) {
            Ok(LinkEvent::Aborted) | Err(_) => return false,
            Ok(_) => {}
        }
        match rx.poll(rx_table) {
            Ok(LinkEvent::Received(got)) => {
                if inbox.push(got).is_err() {
                    return false;
                }
            }
            Ok(LinkEvent::Aborted) | Err(_) => return false,
            Ok(_) => {}
        }
        if hold.is_none() && tx.pump().sender().state() == TxState::Idle {
            return true;
        }
    }
    false
}

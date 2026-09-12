//! Alice ↔ Bob: payload bytes on a live P2P link.
//!
//! Not a forum. Handshake, then `ping` A→B and `pong` B→A. The crate
//! never owns the blob; the 4-byte frame never sees a peer.

mod common;

use psicose::prelude::*;

use common::{established, send};

#[test]
fn peers_exchange_bytes_both_ways() {
    let cfg = SessionConfig::FORUM;
    let wire = Wire::new();
    let (pump_a, pump_b) = wire.link_pumps();

    let mut alice = PeerTable::<4>::with(PeerId::from_label(b"alice"), cfg);
    let mut bob = PeerTable::<4>::with(PeerId::from_label(b"bob"), cfg);

    let mut a = match PeerLink::connect(&mut alice, pump_a) {
        Ok(link) => link,
        Err(_) => return,
    };
    let mut b = PeerLink::accept(&bob, pump_b);

    assert!(established(&mut a, &mut alice, &mut b, &mut bob));

    let ping = b"ping";
    let mut board = [0u8; 32];
    let mut inbox = Defragmenter::new(&mut board);
    let before = b.pump().stats();
    assert!(send(
        &mut a, &mut alice, &mut b, &mut bob, 1, ping, &mut inbox
    ));
    assert_eq!(inbox.as_slice(), ping);
    assert_eq!(b.pump().stats().retries, before.retries);
    assert!(b.pump().stats().bytes_delivered > before.bytes_delivered);

    let pong = b"pong";
    let before = a.pump().stats();
    assert!(send(
        &mut b, &mut bob, &mut a, &mut alice, 2, pong, &mut inbox
    ));
    assert_eq!(inbox.as_slice(), pong);
    assert_eq!(a.pump().stats().retries, before.retries);
}

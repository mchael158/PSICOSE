//! Cooperative helpers driven by the PSICOSE motor ([`super::Node`] + [`super::PeerLink`]).
//!
//! These live **inside** the crate so examples do not reinvent P2P.
//! Prefer them for demos and tests; production firmware usually calls
//! [`PeerLink::poll`](super::PeerLink::poll) from its own scheduler.
//!
//! Both helpers **busy-loop** (bounded) until success or failure — they
//! never block on OS I/O; they only call `poll` on in-memory or
//! non-blocking transports.

use crate::transport::ByteTransport;
use crate::tx::TxState;

use super::link::{LinkEvent, PeerLink};
use super::node::Node;
use super::stream::{Defragmenter, Fragmenter, MessageId, StreamId};

/// Poll both links until both report [`LinkEvent::Established`], or fail.
///
/// Runs up to **100_000** cooperative steps. Returns `false` on abort,
/// transport error, or if the budget is exhausted (stalled handshake).
///
/// # Example
///
/// ```
/// use psicose::prelude::*;
///
/// let wire = Wire::new();
/// let (pa, pb) = wire.link_pumps();
/// let mut alice = Node::<4>::new(PeerId::from_label(b"alice"));
/// let mut bob = Node::<4>::new(PeerId::from_label(b"bob"));
/// let mut a = alice.connect(pa).unwrap();
/// let mut b = bob.accept(pb);
/// assert!(psicose::establish(&mut a, &mut alice, &mut b, &mut bob));
/// ```
pub fn establish<const N: usize, Tx, Rx>(
    a: &mut PeerLink<Tx, Rx>,
    alice: &mut Node<N>,
    b: &mut PeerLink<Tx, Rx>,
    bob: &mut Node<N>,
) -> bool
where
    Tx: ByteTransport,
    Rx: ByteTransport<Error = Tx::Error>,
{
    let mut a_ok = false;
    let mut b_ok = false;
    let mut i = 0usize;
    while i < 100_000 {
        i += 1;
        match a.poll(alice.table_mut()) {
            Ok(LinkEvent::Established) => a_ok = true,
            Ok(LinkEvent::Aborted) | Err(_) => return false,
            Ok(_) => {}
        }
        match b.poll(bob.table_mut()) {
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

/// Fragment `body` on `tx` and rebuild into `inbox` via `rx` (same wire).
///
/// Assumes both links are already [`LinkEvent::Established`]. Uses
/// [`StreamId::FORUM`] and chunk size 4. Each offered byte is driven with
/// up to **20_000** poll steps. Returns `true` only when `inbox` is
/// complete.
pub fn send_message<const N: usize, Tx, Rx>(
    tx: &mut PeerLink<Tx, Rx>,
    tx_node: &mut Node<N>,
    rx: &mut PeerLink<Tx, Rx>,
    rx_node: &mut Node<N>,
    id: u16,
    body: &[u8],
    inbox: &mut Defragmenter<'_>,
) -> bool
where
    Tx: ByteTransport,
    Rx: ByteTransport<Error = Tx::Error>,
{
    inbox.reset();
    let mut frag = Fragmenter::new(StreamId::FORUM, MessageId::new(id), body, 4);
    while let Some((header, chunk)) = frag.next_fragment() {
        for b in header.to_bytes() {
            if !offer_byte(tx, tx_node, rx, rx_node, b, inbox) {
                return false;
            }
        }
        for &b in chunk {
            if !offer_byte(tx, tx_node, rx, rx_node, b, inbox) {
                return false;
            }
        }
    }
    inbox.is_complete()
}

fn offer_byte<const N: usize, Tx, Rx>(
    tx: &mut PeerLink<Tx, Rx>,
    tx_node: &mut Node<N>,
    rx: &mut PeerLink<Tx, Rx>,
    rx_node: &mut Node<N>,
    byte: u8,
    inbox: &mut Defragmenter<'_>,
) -> bool
where
    Tx: ByteTransport,
    Rx: ByteTransport<Error = Tx::Error>,
{
    let mut hold = Some(byte);
    let mut steps = 0usize;
    while steps < 20_000 {
        steps += 1;
        if let Some(b) = hold {
            if tx.offer(b).is_ok() {
                hold = None;
            }
        }
        match tx.poll(tx_node.table_mut()) {
            Ok(LinkEvent::Aborted) | Err(_) => return false,
            Ok(_) => {}
        }
        match rx.poll(rx_node.table_mut()) {
            Ok(LinkEvent::Received(got)) => {
                if inbox.push(got).is_err() {
                    return false;
                }
            }
            Ok(LinkEvent::Aborted) | Err(_) => return false,
            Ok(_) => {}
        }
        if hold.is_none()
            && tx.pump().sender().state() == TxState::Idle
            && tx.pump().sender().outstanding() == 0
        {
            return true;
        }
    }
    false
}

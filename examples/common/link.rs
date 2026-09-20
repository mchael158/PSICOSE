//! In-memory stand-in for a UART / SPI / radio.
//!
//! Each example is a single thread. The "wire" is two heapless rings.
//! On hardware, implement [`psicose::ByteTransport`] on your UART and
//! wrap with [`psicose::LinkFace`] — the rest stays the same.
//!
//! Shared harness for sibling examples (`#[path = "common/link.rs"]`).
//! Not a runnable demo — each example uses only a subset of these helpers.
#![allow(dead_code)]

use core::cell::RefCell;

use psicose::error::Error;
use psicose::pump::{Pump, PumpEvent};
use psicose::rx::PollOutcome;
use psicose::transport::{ByteSink, ByteSource, ByteTransport};
use psicose::tx::{TxPoll, TxState};
use psicose::window::{WindowedReceiver, WindowedSender};
use psicose::{
    Defragmenter, DuplexPort, Fragmenter, LinkEvent, MessageId, PeerLink, PeerTable, StreamId,
};

const RING: usize = 256;

struct Ring {
    buf: [u8; RING],
    head: usize,
    tail: usize,
    len: usize,
}

impl Ring {
    const fn new() -> Self {
        Ring {
            buf: [0; RING],
            head: 0,
            tail: 0,
            len: 0,
        }
    }

    fn push(&mut self, byte: u8) -> bool {
        if self.len == RING {
            return false;
        }
        self.buf[self.tail] = byte;
        self.tail = (self.tail + 1) % RING;
        self.len += 1;
        true
    }

    fn pop(&mut self) -> Option<u8> {
        if self.len == 0 {
            return None;
        }
        let byte = self.buf[self.head];
        self.head = (self.head + 1) % RING;
        self.len -= 1;
        Some(byte)
    }
}

pub struct Wires {
    a_to_b: Ring,
    b_to_a: Ring,
}

impl Wires {
    pub const fn new() -> Self {
        Wires {
            a_to_b: Ring::new(),
            b_to_a: Ring::new(),
        }
    }
}

pub struct End<'a> {
    wires: &'a RefCell<Wires>,
    is_a: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkFull;

impl ByteTransport for End<'_> {
    type Error = LinkFull;

    fn write_byte(&mut self, byte: u8) -> Result<(), Self::Error> {
        let mut w = self.wires.borrow_mut();
        let ok = if self.is_a {
            w.a_to_b.push(byte)
        } else {
            w.b_to_a.push(byte)
        };
        if ok {
            Ok(())
        } else {
            Err(LinkFull)
        }
    }

    fn read_byte(&mut self) -> Result<Option<u8>, Self::Error> {
        let mut w = self.wires.borrow_mut();
        Ok(if self.is_a {
            w.b_to_a.pop()
        } else {
            w.a_to_b.pop()
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyError {
    Source,
    Sink,
    Protocol,
    Stalled,
}

/// Stop-and-wait copy. `src` is the file / JPEG / sensor; `sink` is flash / RAM.
pub fn copy_stop_and_wait<S, K>(src: &mut S, sink: &mut K) -> Result<usize, CopyError>
where
    S: ByteSource,
    K: ByteSink,
{
    let wires = RefCell::new(Wires::new());
    let mut pump = Pump::on(
        End {
            wires: &wires,
            is_a: true,
        },
        End {
            wires: &wires,
            is_a: false,
        },
    );

    let mut hold: Option<u8> = None;
    let mut exhausted = false;
    let mut n = 0usize;

    for _ in 0..1_000_000 {
        if hold.is_none() && !exhausted {
            match src.read_byte() {
                Ok(Some(b)) => hold = Some(b),
                Ok(None) => exhausted = true,
                Err(_) => return Err(CopyError::Source),
            }
        }

        if pump.sender().state() == TxState::Idle {
            if let Some(b) = hold.take() {
                if pump.sender_mut().offer(b).is_err() {
                    hold = Some(b);
                }
            } else if exhausted {
                match pump.sender_mut().offer_finish() {
                    Ok(()) => {}
                    Err(Error::NotIdle) => {}
                    Err(_) => return Err(CopyError::Protocol),
                }
            }
        }

        match pump.poll() {
            Ok(PumpEvent::Received(byte)) => {
                if sink.write_byte(byte).is_err() {
                    return Err(CopyError::Sink);
                }
                n = n.saturating_add(1);
            }
            Ok(PumpEvent::Completed) => return Ok(n),
            Ok(PumpEvent::Aborted) => return Err(CopyError::Protocol),
            Ok(_) => {}
            Err(_) => return Err(CopyError::Protocol),
        }
    }

    Err(CopyError::Stalled)
}

/// Selective-repeat copy (`N ≤ 8`). Same sources and sinks as stop-and-wait.
pub fn copy_windowed<S, K, const N: usize>(src: &mut S, sink: &mut K) -> Result<usize, CopyError>
where
    S: ByteSource,
    K: ByteSink,
{
    let wires = RefCell::new(Wires::new());
    let mut tx = WindowedSender::<_, N>::new(End {
        wires: &wires,
        is_a: true,
    });
    let mut rx = WindowedReceiver::<_, N>::new(End {
        wires: &wires,
        is_a: false,
    });

    let mut hold: Option<u8> = None;
    let mut exhausted = false;
    let mut finish_offered = false;
    let mut n = 0usize;

    for _ in 0..2_000_000 {
        if hold.is_none() && !exhausted {
            match src.read_byte() {
                Ok(Some(b)) => hold = Some(b),
                Ok(None) => exhausted = true,
                Err(_) => return Err(CopyError::Source),
            }
        }

        if let Some(b) = hold {
            match tx.offer(b) {
                Ok(()) => hold = None,
                Err(Error::WindowFull) | Err(Error::NotIdle) => {}
                Err(_) => return Err(CopyError::Protocol),
            }
        } else if exhausted && !finish_offered {
            match tx.offer_finish() {
                Ok(()) => finish_offered = true,
                Err(Error::NotIdle) => {}
                Err(_) => return Err(CopyError::Protocol),
            }
        }

        match tx.poll() {
            Ok(TxPoll::TransferDone) => {}
            Ok(_) => {}
            Err(_) => return Err(CopyError::Protocol),
        }

        match rx.poll() {
            Ok(PollOutcome::Delivered(byte)) => {
                if sink.write_byte(byte).is_err() {
                    return Err(CopyError::Sink);
                }
                n = n.saturating_add(1);
            }
            Ok(PollOutcome::TransferFinished) => return Ok(n),
            Ok(_) => {}
            Err(_) => return Err(CopyError::Protocol),
        }
    }

    Err(CopyError::Stalled)
}

/// Pair of ends on the same wire, for examples that wrap the transport.
pub fn pair(wires: &RefCell<Wires>) -> (End<'_>, End<'_>) {
    (End { wires, is_a: true }, End { wires, is_a: false })
}

/// In-memory A↔B [`PeerLink`] over [`psicose::Wire`] (framework P2P layer).
pub type Peer<'w> = PeerLink<DuplexPort<'w>, DuplexPort<'w>>;

/// Drive both links until hellos cross (or fail).
pub fn established<const N: usize>(
    a: &mut Peer<'_>,
    alice: &mut PeerTable<N>,
    b: &mut Peer<'_>,
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

/// Fragment `body` over `tx` and rebuild into `inbox` via `rx`.
pub fn send_message<const N: usize>(
    tx: &mut Peer<'_>,
    tx_table: &mut PeerTable<N>,
    rx: &mut Peer<'_>,
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
    tx: &mut Peer<'_>,
    tx_table: &mut PeerTable<N>,
    rx: &mut Peer<'_>,
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

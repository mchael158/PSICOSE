//! Heapless in-memory duplex used by integration tests. No `Vec`, no threads.

use core::cell::RefCell;

use psicose::{ByteTransport, PollOutcome, Receiver, Sender, TxPoll};

pub const RING: usize = 64;

pub struct Ring {
    buf: [u8; RING],
    head: usize,
    tail: usize,
    len: usize,
}

impl Ring {
    pub const fn new() -> Self {
        Ring {
            buf: [0; RING],
            head: 0,
            tail: 0,
            len: 0,
        }
    }

    pub fn push(&mut self, byte: u8) -> bool {
        if self.len == RING {
            return false;
        }
        self.buf[self.tail] = byte;
        self.tail = (self.tail + 1) % RING;
        self.len += 1;
        true
    }

    pub fn pop(&mut self) -> Option<u8> {
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
    pub a_to_b: Ring,
    pub b_to_a: Ring,
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
    pub wires: &'a RefCell<Wires>,
    pub is_a: bool,
}

/// The heapless ring had no room for another byte.
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

pub fn pump_rx<T: ByteTransport>(
    rx: &mut Receiver<T>,
    out: &mut [u8],
    filled: &mut usize,
) -> bool {
    match rx.poll() {
        Ok(PollOutcome::Delivered(byte)) => {
            if *filled < out.len() {
                out[*filled] = byte;
                *filled += 1;
            }
            false
        }
        Ok(outcome) => outcome.is_closed(),
        Err(_) => false,
    }
}

#[allow(dead_code)]
pub fn send_byte_coop<Tx, Rx>(
    sender: &mut Sender<Tx>,
    receiver: &mut Receiver<Rx>,
    data: u8,
    out: &mut [u8],
    filled: &mut usize,
) where
    Tx: ByteTransport,
    Rx: ByteTransport,
    Tx::Error: core::fmt::Debug,
    Rx::Error: core::fmt::Debug,
{
    if sender.offer(data).is_err() {
        return;
    }
    loop {
        match sender.poll() {
            Ok(TxPoll::Acked) => return,
            Ok(TxPoll::Pending) => {}
            Ok(_) | Err(_) => return,
        }
        let finished = pump_rx(receiver, out, filled);
        if finished {
            return;
        }
    }
}

#[allow(dead_code)]
pub fn start_coop<Tx, Rx>(
    sender: &mut Sender<Tx>,
    receiver: &mut Receiver<Rx>,
    out: &mut [u8],
    filled: &mut usize,
) where
    Tx: ByteTransport,
    Rx: ByteTransport,
    Tx::Error: core::fmt::Debug,
    Rx::Error: core::fmt::Debug,
{
    if sender.offer_start().is_err() {
        return;
    }
    loop {
        match sender.poll() {
            Ok(TxPoll::SessionReady) => return,
            Ok(TxPoll::Pending) => {}
            Ok(_) | Err(_) => return,
        }
        let _ = pump_rx(receiver, out, filled);
    }
}

#[allow(dead_code)]
pub fn finish_coop<Tx, Rx>(
    sender: &mut Sender<Tx>,
    receiver: &mut Receiver<Rx>,
    out: &mut [u8],
    filled: &mut usize,
) where
    Tx: ByteTransport,
    Rx: ByteTransport,
    Tx::Error: core::fmt::Debug,
    Rx::Error: core::fmt::Debug,
{
    if sender.offer_finish().is_err() {
        return;
    }
    loop {
        match sender.poll() {
            Ok(TxPoll::TransferDone) => return,
            Ok(TxPoll::Pending) => {}
            Ok(_) | Err(_) => return,
        }
        let _ = pump_rx(receiver, out, filled);
    }
}

#[allow(dead_code)]
pub fn abort_coop<Tx, Rx>(
    sender: &mut Sender<Tx>,
    receiver: &mut Receiver<Rx>,
    out: &mut [u8],
    filled: &mut usize,
) where
    Tx: ByteTransport,
    Rx: ByteTransport,
    Tx::Error: core::fmt::Debug,
    Rx::Error: core::fmt::Debug,
{
    if sender.offer_abort().is_err() {
        return;
    }
    loop {
        match sender.poll() {
            Ok(TxPoll::Aborted) => return,
            Ok(TxPoll::Pending) => {}
            Ok(_) | Err(_) => return,
        }
        let _ = pump_rx(receiver, out, filled);
    }
}

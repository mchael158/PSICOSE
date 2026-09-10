//! In-memory stand-in for a UART / SPI / radio.
//!
//! Each example is a single thread. The "wire" is two heapless rings.
//! Swap `End` for a real UART driver in the field — the rest stays.
#![allow(dead_code)]

use core::cell::RefCell;

use psicose::error::Error;
use psicose::rx::{PollOutcome, Receiver};
use psicose::transport::{ByteSink, ByteSource, ByteTransport};
use psicose::tx::{Sender, TxPoll, TxState};
use psicose::window::{WindowedReceiver, WindowedSender};

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
    let mut tx = Sender::new(End {
        wires: &wires,
        is_a: true,
    });
    let mut rx = Receiver::new(End {
        wires: &wires,
        is_a: false,
    });

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

        if tx.state() == TxState::Idle {
            if let Some(b) = hold.take() {
                if tx.offer(b).is_err() {
                    hold = Some(b);
                }
            } else if exhausted {
                match tx.offer_finish() {
                    Ok(()) => {}
                    Err(Error::NotIdle) => {}
                    Err(_) => return Err(CopyError::Protocol),
                }
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
    (
        End {
            wires,
            is_a: true,
        },
        End {
            wires,
            is_a: false,
        },
    )
}

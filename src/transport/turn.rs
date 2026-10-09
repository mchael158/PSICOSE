//! Cooperative turns on one [`crate::LinkFace`].
//!
//! Both peers may send. Only the peer whose turn it is may open `START`.
//! After `FINISH` is acknowledged the turn flips, so ACK and DATA never
//! share the wire from two speakers at once. The 4-byte frame is unchanged.
//!
//! ```text
//! first  -- START ping FINISH -->  second   (only first may START)
//! first  <-- START pong FINISH --  second   (turn flipped)
//! ```
//!
//! Pass `first = true` on one peer and `first = false` on the other.

use crate::error::Error;
use crate::pump::{Pump, PumpEvent};
use crate::transport::ByteTransport;
use crate::tx::TxState;

/// What one [`Turn::poll`] observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnEvent {
    /// Application byte from the peer's session.
    Received(u8),
    /// Our session finished. The peer may speak next.
    Yielded,
    /// The peer's session finished. We may speak next.
    Granted,
    /// No application event this step (ACK, retry, or waiting).
    Idle,
}

/// Stop-and-wait pump that alternates who may open a session.
///
/// `N` is the outbound queue for **this** peer's next session (stack only).
pub struct Turn<Tx, Rx, const N: usize>
where
    Tx: ByteTransport,
    Rx: ByteTransport<Error = Tx::Error>,
{
    pump: Pump<Tx, Rx>,
    /// `true` when this peer is allowed to `START`.
    speak: bool,
    buf: [u8; N],
    len: usize,
    index: usize,
    need_start: bool,
    finish_offered: bool,
}

impl<Tx, Rx, const N: usize> Turn<Tx, Rx, N>
where
    Tx: ByteTransport,
    Rx: ByteTransport<Error = Tx::Error>,
{
    /// `first` sends the opening session. The other peer waits.
    pub const fn new(pump: Pump<Tx, Rx>, first: bool) -> Self {
        Turn {
            pump,
            speak: first,
            buf: [0; N],
            len: 0,
            index: 0,
            need_start: true,
            finish_offered: false,
        }
    }

    /// Whether this peer may open the next session.
    pub const fn speaking(&self) -> bool {
        self.speak
    }

    /// Bytes still queued for our next (or current) session.
    pub const fn queued(&self) -> usize {
        self.len
    }

    /// The pump underneath (stats).
    pub const fn pump(&self) -> &Pump<Tx, Rx> {
        &self.pump
    }

    /// Cancel the in-flight session. The turn does not flip.
    pub fn abort(&mut self) -> Result<(), Error<Tx::Error>> {
        self.need_start = true;
        self.index = 0;
        self.finish_offered = false;
        self.pump.abort()
    }

    /// Queue application bytes for the next session we open.
    ///
    /// Returns how many bytes fit. Returns `0` once `FINISH` for the
    /// current session is already on the wire — call again after
    /// [`TurnEvent::Granted`].
    pub fn push(&mut self, bytes: &[u8]) -> usize {
        if self.finish_offered {
            return 0;
        }
        let mut n = 0usize;
        while n < bytes.len() && self.len < N {
            self.buf[self.len] = bytes[n];
            self.len += 1;
            n += 1;
        }
        n
    }

    /// One cooperative step. Offer our frame only when it is our turn.
    pub fn poll(&mut self) -> Result<TurnEvent, Error<Tx::Error>> {
        self.drive_tx();
        match self.pump.poll()? {
            PumpEvent::Received(byte) => Ok(TurnEvent::Received(byte)),
            PumpEvent::Completed if self.speak => {
                self.speak = false;
                self.len = 0;
                self.index = 0;
                self.need_start = true;
                self.finish_offered = false;
                Ok(TurnEvent::Yielded)
            }
            PumpEvent::Completed => {
                self.speak = true;
                self.need_start = true;
                self.index = 0;
                self.finish_offered = false;
                Ok(TurnEvent::Granted)
            }
            PumpEvent::Aborted if self.speak => {
                // Same turn, same payload: open START again. Do not flip,
                // or both peers would START together.
                self.need_start = true;
                self.index = 0;
                self.finish_offered = false;
                Ok(TurnEvent::Idle)
            }
            PumpEvent::Aborted | PumpEvent::Sent | PumpEvent::Progress | PumpEvent::Idle => {
                Ok(TurnEvent::Idle)
            }
        }
    }

    fn drive_tx(&mut self) {
        if !self.speak || self.len == 0 {
            return;
        }
        let st = self.pump.sender().state();
        if !matches!(
            st,
            TxState::Idle | TxState::Finished | TxState::Aborted | TxState::Failed
        ) {
            return;
        }
        if self.need_start {
            if self.pump.sender_mut().offer_start().is_ok() {
                self.need_start = false;
                self.index = 0;
                self.finish_offered = false;
            }
        } else if st == TxState::Idle {
            if self.index < self.len {
                let byte = self.buf[self.index];
                if self.pump.sender_mut().offer(byte).is_ok() {
                    self.index += 1;
                }
            } else if !self.finish_offered && self.pump.sender_mut().offer_finish().is_ok() {
                self.finish_offered = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::{ByteTransport, LinkFace};
    use core::cell::RefCell;

    const RING: usize = 256;

    struct Ring {
        buf: [u8; RING],
        head: usize,
        len: usize,
    }

    impl Ring {
        const fn new() -> Self {
            Ring {
                buf: [0; RING],
                head: 0,
                len: 0,
            }
        }

        fn push(&mut self, byte: u8) -> bool {
            if self.len == RING {
                return false;
            }
            let tail = (self.head + self.len) % RING;
            self.buf[tail] = byte;
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

    struct Bus {
        ab: RefCell<Ring>,
        ba: RefCell<Ring>,
    }

    struct End<'a> {
        bus: &'a Bus,
        is_a: bool,
    }

    impl ByteTransport for End<'_> {
        type Error = ();

        fn write_byte(&mut self, byte: u8) -> Result<(), Self::Error> {
            let ok = if self.is_a {
                self.bus.ab.borrow_mut().push(byte)
            } else {
                self.bus.ba.borrow_mut().push(byte)
            };
            if ok {
                Ok(())
            } else {
                Err(())
            }
        }

        fn read_byte(&mut self) -> Result<Option<u8>, Self::Error> {
            let byte = if self.is_a {
                self.bus.ba.borrow_mut().pop()
            } else {
                self.bus.ab.borrow_mut().pop()
            };
            Ok(byte)
        }
    }

    #[test]
    fn peers_exchange_ping_then_pong_without_overlapping_start() {
        let bus = Bus {
            ab: RefCell::new(Ring::new()),
            ba: RefCell::new(Ring::new()),
        };
        let face_a = LinkFace::new(End {
            bus: &bus,
            is_a: true,
        });
        let face_b = LinkFace::new(End {
            bus: &bus,
            is_a: false,
        });
        let mut a = face_a.turn::<8>(true);
        let mut b = face_b.turn::<8>(false);
        assert_eq!(a.push(b"ping"), 4);
        assert_eq!(b.push(b"pong"), 4);

        let mut got_a = [0u8; 4];
        let mut got_b = [0u8; 4];
        let mut na = 0usize;
        let mut nb = 0usize;
        let mut a_yielded = false;
        let mut b_granted = false;
        let mut b_spoke_before_a_yielded = false;

        for _ in 0..20_000 {
            match a.poll().unwrap() {
                TurnEvent::Received(byte) => {
                    assert!(a_yielded, "reply bytes before our session finished");
                    got_a[na] = byte;
                    na += 1;
                }
                TurnEvent::Yielded => a_yielded = true,
                TurnEvent::Granted | TurnEvent::Idle => {}
            }
            match b.poll().unwrap() {
                TurnEvent::Received(byte) => {
                    got_b[nb] = byte;
                    nb += 1;
                }
                TurnEvent::Yielded => {
                    if !a_yielded {
                        b_spoke_before_a_yielded = true;
                    }
                }
                TurnEvent::Granted => b_granted = true,
                TurnEvent::Idle => {}
            }
            if a_yielded && na == 4 && nb == 4 {
                break;
            }
        }

        assert!(b_granted);
        assert!(!b_spoke_before_a_yielded);
        assert_eq!(&got_b[..nb], b"ping");
        assert_eq!(&got_a[..na], b"pong");
        assert!(a_yielded);
        assert!(!a.speaking());
        assert!(b.speaking() || b.queued() == 0);
    }
}

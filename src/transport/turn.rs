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
use crate::pump::{Pump, PumpEvent, WindowedPump};
use crate::transport::ByteTransport;
use crate::tx::TxState;
use crate::window::MAX_WINDOW;

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

/// Hardware turn: both peers send, and DATA uses the window of 8.
///
/// [`Turn`] is stop-and-wait (one DATA frame, then its ACK). On a UART
/// that costs a full round trip per byte. [`WindowTurn`] still opens one
/// session at a time — so two `START`s never share the wire — but fills
/// the selective-repeat window before `FINISH`. The 4-byte frame is the
/// same. `N` is this peer's outbound queue (stack only).
pub struct WindowTurn<Tx, Rx, const N: usize>
where
    Tx: ByteTransport,
    Rx: ByteTransport<Error = Tx::Error>,
{
    pump: WindowedPump<Tx, Rx, MAX_WINDOW>,
    speak: bool,
    buf: [u8; N],
    len: usize,
    index: usize,
    need_start: bool,
    finish_offered: bool,
}

impl<Tx, Rx, const N: usize> WindowTurn<Tx, Rx, N>
where
    Tx: ByteTransport,
    Rx: ByteTransport<Error = Tx::Error>,
{
    /// `first` sends the opening session. The other peer waits.
    pub fn new(pump: WindowedPump<Tx, Rx, MAX_WINDOW>, first: bool) -> Self {
        WindowTurn {
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

    /// The windowed pump underneath (stats, window limit).
    pub const fn pump(&self) -> &WindowedPump<Tx, Rx, MAX_WINDOW> {
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
    /// current session is already on the wire.
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

    /// One cooperative step. DATA is offered until the window is full.
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
        if self.need_start {
            if self.pump.sender_mut().offer_start().is_ok() {
                self.need_start = false;
                self.index = 0;
                self.finish_offered = false;
            }
            return;
        }
        if self.finish_offered {
            return;
        }
        while self.index < self.len {
            let byte = self.buf[self.index];
            if self.pump.sender_mut().offer(byte).is_err() {
                return;
            }
            self.index += 1;
        }
        if self.pump.sender_mut().offer_finish().is_ok() {
            self.finish_offered = true;
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

    #[test]
    fn window_turn_exchanges_eight_byte_samples() {
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
        // 23.1 °C / 47% / 3710 mV, then 22.0 °C / 40% / 3600 mV.
        let sample_a = [0xA5, 0xE7, 0x00, 0x2F, 0x7E, 0x0E, 0x00, 0x1D];
        let sample_b = [0xA5, 0xDC, 0x00, 0x28, 0x10, 0x0E, 0x01, 0x4E];
        let mut a = face_a.window_turn::<8>(true);
        let mut b = face_b.window_turn::<8>(false);
        assert_eq!(a.push(&sample_a), 8);
        assert_eq!(b.push(&sample_b), 8);

        let mut got_a = [0u8; 8];
        let mut got_b = [0u8; 8];
        let mut na = 0usize;
        let mut nb = 0usize;
        let mut a_yielded = false;

        for _ in 0..40_000 {
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
                TurnEvent::Idle | TurnEvent::Yielded | TurnEvent::Granted => {}
            }
            if a_yielded && na == 8 && nb == 8 {
                break;
            }
        }

        assert!(a_yielded);
        assert_eq!(got_b, sample_a);
        assert_eq!(got_a, sample_b);
    }
}

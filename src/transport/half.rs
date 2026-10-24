//! Half-duplex line in front of a UART, RS-485, or radio.
//!
//! A crossed UART has two wires, so one peer can send while the other
//! sends. A radio or an RS-485 pair has one direction at a time. Bytes
//! from two writers must not share that pair in the middle of a frame.
//!
//! [`HalfDuplex`] holds each frame in a 4-byte stage and only then drives
//! the pin ([`LineDirection::transmit`]), clocks the four bytes out, and
//! drops the pin ([`LineDirection::receive`]). The 4-byte frame is
//! unchanged. Use it under a stop-and-wait [`crate::Turn`]: the windowed
//! turn pipelines DATA and will talk over the peer's ACK on one pair.

use crate::protocol::FRAME_LEN;
use crate::transport::ByteTransport;

/// Drives the external direction pin (RS-485 DE/RE, radio TX/RX switch).
pub trait LineDirection {
    /// The line is ours. Raise DE or key the transmitter.
    fn transmit(&mut self);
    /// The line is the peer's. Drop DE or open the receiver.
    fn receive(&mut self);
}

/// No external pin. Frames are still released four bytes at a time.
#[derive(Debug, Default, Clone, Copy)]
pub struct NopDirection;

impl LineDirection for NopDirection {
    fn transmit(&mut self) {}
    fn receive(&mut self) {}
}

/// Stages one frame, then writes it as a single burst.
///
/// `T` is the physical port (UART FIFO, radio FIFO). `D` toggles the
/// direction pin around that burst.
pub struct HalfDuplex<T, D> {
    port: T,
    dir: D,
    stage: [u8; FRAME_LEN],
    /// Bytes stored for the frame currently being built.
    filled: usize,
    /// How many of those bytes have already been accepted by `port`.
    flushed: usize,
}

impl<T, D> HalfDuplex<T, D> {
    /// Start listening. The pin is dropped before any byte is staged.
    pub fn new(port: T, mut dir: D) -> Self
    where
        D: LineDirection,
    {
        dir.receive();
        HalfDuplex {
            port,
            dir,
            stage: [0; FRAME_LEN],
            filled: 0,
            flushed: 0,
        }
    }

    /// Bytes waiting to form the next on-wire frame (`0..4`).
    pub const fn staged(&self) -> usize {
        self.filled
    }

    /// The direction driver (so a board can read the pin back).
    pub const fn direction(&self) -> &D {
        &self.dir
    }

    /// The physical port.
    pub const fn port(&self) -> &T {
        &self.port
    }
}

impl<T: ByteTransport, D: LineDirection> ByteTransport for HalfDuplex<T, D> {
    type Error = T::Error;

    fn write_byte(&mut self, byte: u8) -> Result<(), Self::Error> {
        if self.filled < FRAME_LEN {
            self.stage[self.filled] = byte;
            self.filled += 1;
        }
        if self.filled < FRAME_LEN {
            return Ok(());
        }
        self.dir.transmit();
        while self.flushed < FRAME_LEN {
            self.port.write_byte(self.stage[self.flushed])?;
            self.flushed += 1;
        }
        self.dir.receive();
        self.filled = 0;
        self.flushed = 0;
        Ok(())
    }

    fn read_byte(&mut self) -> Result<Option<u8>, Self::Error> {
        self.port.read_byte()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::{LinkFace, TurnEvent};
    use core::cell::Cell;

    struct SpyPort {
        out: [u8; 32],
        out_n: usize,
        fail_times: u8,
        incoming: [u8; 32],
        in_n: usize,
        in_i: usize,
    }

    impl SpyPort {
        const fn new() -> Self {
            SpyPort {
                out: [0; 32],
                out_n: 0,
                fail_times: 0,
                incoming: [0; 32],
                in_n: 0,
                in_i: 0,
            }
        }
    }

    impl ByteTransport for SpyPort {
        type Error = &'static str;

        fn write_byte(&mut self, byte: u8) -> Result<(), Self::Error> {
            if self.fail_times > 0 {
                self.fail_times -= 1;
                return Err("fifo");
            }
            self.out[self.out_n] = byte;
            self.out_n += 1;
            Ok(())
        }

        fn read_byte(&mut self) -> Result<Option<u8>, Self::Error> {
            if self.in_i >= self.in_n {
                return Ok(None);
            }
            let byte = self.incoming[self.in_i];
            self.in_i += 1;
            Ok(Some(byte))
        }
    }

    #[derive(Default)]
    struct Pin {
        high: bool,
        rises: u8,
        falls: u8,
    }

    impl LineDirection for Pin {
        fn transmit(&mut self) {
            if !self.high {
                self.rises += 1;
            }
            self.high = true;
        }

        fn receive(&mut self) {
            if self.high {
                self.falls += 1;
            }
            self.high = false;
        }
    }

    #[test]
    fn nothing_hits_the_wire_until_the_frame_is_full() {
        let mut line = HalfDuplex::new(SpyPort::new(), Pin::default());
        line.write_byte(0x01).unwrap();
        line.write_byte(0x02).unwrap();
        line.write_byte(0x03).unwrap();
        assert_eq!(line.port().out_n, 0);
        assert_eq!(line.staged(), 3);
        assert!(!line.direction().high);
        line.write_byte(0x04).unwrap();
        assert_eq!(line.port().out_n, 4);
        assert_eq!(&line.port().out[..4], &[0x01, 0x02, 0x03, 0x04]);
        assert_eq!(line.staged(), 0);
        assert!(!line.direction().high);
        assert_eq!(line.direction().rises, 1);
        assert_eq!(line.direction().falls, 1);
    }

    #[test]
    fn a_fifo_reject_retries_the_same_frame_without_splitting_it() {
        let mut port = SpyPort::new();
        port.fail_times = 1;
        let mut line = HalfDuplex::new(port, NopDirection);
        line.write_byte(0xA).unwrap();
        line.write_byte(0xB).unwrap();
        line.write_byte(0xC).unwrap();
        assert!(line.write_byte(0xD).is_err());
        assert_eq!(line.port().out_n, 0);
        // Pump retries the same fourth byte. The stage must not grow.
        line.write_byte(0xD).unwrap();
        assert_eq!(&line.port().out[..4], &[0xA, 0xB, 0xC, 0xD]);
    }

    /// Shared air. `RefCell` so two [`HalfDuplex`] ends can key the pin.
    struct Air {
        ab: core::cell::RefCell<[u8; 128]>,
        ab_len: Cell<usize>,
        ab_at: Cell<usize>,
        ba: core::cell::RefCell<[u8; 128]>,
        ba_len: Cell<usize>,
        ba_at: Cell<usize>,
        holder: Cell<Option<bool>>,
        collisions: Cell<u32>,
        frames: Cell<u32>,
    }

    impl Air {
        fn new() -> Self {
            Air {
                ab: core::cell::RefCell::new([0; 128]),
                ab_len: Cell::new(0),
                ab_at: Cell::new(0),
                ba: core::cell::RefCell::new([0; 128]),
                ba_len: Cell::new(0),
                ba_at: Cell::new(0),
                holder: Cell::new(None),
                collisions: Cell::new(0),
                frames: Cell::new(0),
            }
        }
    }

    struct AirPort<'a> {
        air: &'a Air,
        is_a: bool,
    }

    struct AirPin<'a> {
        air: &'a Air,
        is_a: bool,
    }

    impl LineDirection for AirPin<'_> {
        fn transmit(&mut self) {
            if let Some(holder) = self.air.holder.get() {
                if holder != self.is_a {
                    self.air.collisions.set(self.air.collisions.get() + 1);
                }
            }
            self.air.holder.set(Some(self.is_a));
        }

        fn receive(&mut self) {
            if self.air.holder.get() == Some(self.is_a) {
                self.air.holder.set(None);
            }
        }
    }

    impl ByteTransport for AirPort<'_> {
        type Error = ();

        fn write_byte(&mut self, byte: u8) -> Result<(), Self::Error> {
            let (buf, len) = if self.is_a {
                (&self.air.ab, &self.air.ab_len)
            } else {
                (&self.air.ba, &self.air.ba_len)
            };
            let n = len.get();
            if n >= 128 {
                return Err(());
            }
            buf.borrow_mut()[n] = byte;
            len.set(n + 1);
            if n % FRAME_LEN == FRAME_LEN - 1 {
                self.air.frames.set(self.air.frames.get() + 1);
            }
            Ok(())
        }

        fn read_byte(&mut self) -> Result<Option<u8>, Self::Error> {
            let (buf, len, at) = if self.is_a {
                (&self.air.ba, &self.air.ba_len, &self.air.ba_at)
            } else {
                (&self.air.ab, &self.air.ab_len, &self.air.ab_at)
            };
            let i = at.get();
            if i >= len.get() {
                return Ok(None);
            }
            let byte = buf.borrow()[i];
            at.set(i + 1);
            Ok(Some(byte))
        }
    }

    #[test]
    fn stop_and_wait_exchange_never_holds_the_pin_together() {
        let air = Air::new();
        let face_a = LinkFace::new(HalfDuplex::new(
            AirPort {
                air: &air,
                is_a: true,
            },
            AirPin {
                air: &air,
                is_a: true,
            },
        ));
        let face_b = LinkFace::new(HalfDuplex::new(
            AirPort {
                air: &air,
                is_a: false,
            },
            AirPin {
                air: &air,
                is_a: false,
            },
        ));
        let mut a = face_a.turn::<4>(true);
        let mut b = face_b.turn::<4>(false);
        assert_eq!(a.push(b"ping"), 4);
        assert_eq!(b.push(b"pong"), 4);

        let mut got_a = [0u8; 4];
        let mut got_b = [0u8; 4];
        let mut na = 0usize;
        let mut nb = 0usize;
        let mut yielded = false;
        for _ in 0..20_000 {
            match a.poll().unwrap() {
                TurnEvent::Received(b) => {
                    got_a[na] = b;
                    na += 1;
                }
                TurnEvent::Yielded => yielded = true,
                _ => {}
            }
            if let TurnEvent::Received(byte) = b.poll().unwrap() {
                got_b[nb] = byte;
                nb += 1;
            }
            if yielded && na == 4 && nb == 4 {
                break;
            }
        }
        assert_eq!(air.collisions.get(), 0);
        assert!(air.frames.get() >= 4);
        assert_eq!(&got_b[..nb], b"ping");
        assert_eq!(&got_a[..na], b"pong");
        assert_eq!(air.ab_len.get() % FRAME_LEN, 0);
        assert_eq!(air.ba_len.get() % FRAME_LEN, 0);
    }
}

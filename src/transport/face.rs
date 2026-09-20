//! One physical link → two [`ByteTransport`] ends for [`crate::Pump`].
//!
//! A UART (or any single RX stream) cannot be read by both the sender and
//! the receiver: ACK/NACK must go to TX, DATA/START/FINISH/ABORT to RX.
//! [`LinkFace`] demultiplexes complete frames so [`crate::Pump::on`] can
//! run on real hardware.
//!
//! ```text
//! application
//!    Pump::on(FaceTx, FaceRx)
//!              │        │
//!              ▼        ▼
//!           LinkFace (demux)
//!              │
//!         ByteTransport   ← your UART impl on ESP32
//! ```
//!
//! In-memory A↔B tests keep using [`crate::Wire`] (`DuplexWire`). Hardware
//! (and any single-port link) uses this type.
//!
//! # Write discipline
//!
//! [`FaceTx`] and [`FaceRx`] share one TX pin. Stop-and-wait keeps them from
//! writing at the same time in the usual cases (sender waits for ACK while
//! the peer only ACKs). Do **not** run two initiators on one link that both
//! stream DATA concurrently — their ACK and DATA bytes would interleave.

use core::cell::RefCell;
use core::fmt;

use crate::protocol::{FrameAssembler, FrameType, FRAME_LEN};
use crate::pump::{Pump, WindowedPump};
use crate::timeout::RetryPolicy;
use crate::transport::ByteTransport;
use crate::window::MAX_WINDOW;

const RING: usize = 64;

#[derive(Clone, Copy)]
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

    fn push_frame(&mut self, bytes: [u8; FRAME_LEN]) -> bool {
        let mut i = 0;
        while i < FRAME_LEN {
            if !self.push(bytes[i]) {
                return false;
            }
            i += 1;
        }
        true
    }
}

struct FaceInner<T> {
    port: T,
    assembler: FrameAssembler,
    control: Ring,
    payload: Ring,
}

impl<T: ByteTransport> FaceInner<T> {
    fn ingest(&mut self) -> Result<(), FaceError<T::Error>> {
        loop {
            match self.port.read_byte() {
                Ok(None) => return Ok(()),
                Ok(Some(byte)) => {
                    let Some(result) = self.assembler.push(byte) else {
                        continue;
                    };
                    let bytes = match result {
                        Ok(frame) => frame.to_bytes(),
                        Err(_) => self.assembler.last_bytes(),
                    };
                    let ok = match result {
                        Ok(frame)
                            if matches!(frame.frame_type(), FrameType::Ack | FrameType::Nack) =>
                        {
                            self.control.push_frame(bytes)
                        }
                        _ => self.payload.push_frame(bytes),
                    };
                    if !ok {
                        return Err(FaceError::Full);
                    }
                }
                Err(e) => return Err(FaceError::Port(e)),
            }
        }
    }
}

/// Demux one physical [`ByteTransport`] into Pump TX/RX ends.
///
/// Interior mutability lets [`FaceTx`] and [`FaceRx`] share the port.
pub struct LinkFace<T> {
    inner: RefCell<FaceInner<T>>,
}

impl<T: ByteTransport> LinkFace<T> {
    /// Wrap a physical port (your UART `ByteTransport`, radio, …).
    pub const fn new(port: T) -> Self {
        LinkFace {
            inner: RefCell::new(FaceInner {
                port,
                assembler: FrameAssembler::new(),
                control: Ring::new(),
                payload: Ring::new(),
            }),
        }
    }

    /// Borrow the sender end and the receiver end.
    pub fn split(&self) -> (FaceTx<'_, T>, FaceRx<'_, T>) {
        (FaceTx { face: self }, FaceRx { face: self })
    }

    /// Stop-and-wait [`Pump`] on this face.
    pub fn pump(&self) -> Pump<FaceTx<'_, T>, FaceRx<'_, T>> {
        let (tx, rx) = self.split();
        Pump::on(tx, rx)
    }

    /// [`Self::pump`] with an explicit [`RetryPolicy`].
    pub fn pump_with(&self, policy: RetryPolicy) -> Pump<FaceTx<'_, T>, FaceRx<'_, T>> {
        let (tx, rx) = self.split();
        Pump::on_with(tx, rx, policy)
    }

    /// Windowed pump for [`crate::PeerLink`] (`N = MAX_WINDOW`, limit 1 until hello).
    pub fn link_pump(&self) -> WindowedPump<FaceTx<'_, T>, FaceRx<'_, T>, MAX_WINDOW> {
        let (tx, rx) = self.split();
        let mut p = WindowedPump::on(tx, rx);
        p.set_window_limit(1);
        p
    }

    /// Access the wrapped port (e.g. to reconfigure baud).
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.inner.get_mut().port
    }
}

/// TX half of a [`LinkFace`]: writes frames out; reads ACK/NACK.
pub struct FaceTx<'a, T> {
    face: &'a LinkFace<T>,
}

/// RX half of a [`LinkFace`]: writes ACK/NACK out; reads DATA/control frames.
pub struct FaceRx<'a, T> {
    face: &'a LinkFace<T>,
}

/// Error from a [`LinkFace`] port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaceError<E> {
    /// Underlying [`ByteTransport`] failed.
    Port(E),
    /// Demux ring had no room (should not happen with a cooperative poller).
    Full,
}

impl<E: fmt::Debug> fmt::Display for FaceError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FaceError::Port(e) => write!(f, "link face: port error {e:?}"),
            FaceError::Full => write!(f, "link face: demux ring full"),
        }
    }
}

impl<T: ByteTransport> ByteTransport for FaceTx<'_, T> {
    type Error = FaceError<T::Error>;

    fn write_byte(&mut self, byte: u8) -> Result<(), Self::Error> {
        let mut g = self.face.inner.borrow_mut();
        g.port.write_byte(byte).map_err(FaceError::Port)
    }

    fn read_byte(&mut self) -> Result<Option<u8>, Self::Error> {
        let mut g = self.face.inner.borrow_mut();
        g.ingest()?;
        Ok(g.control.pop())
    }
}

impl<T: ByteTransport> ByteTransport for FaceRx<'_, T> {
    type Error = FaceError<T::Error>;

    fn write_byte(&mut self, byte: u8) -> Result<(), Self::Error> {
        let mut g = self.face.inner.borrow_mut();
        g.port.write_byte(byte).map_err(FaceError::Port)
    }

    fn read_byte(&mut self) -> Result<Option<u8>, Self::Error> {
        let mut g = self.face.inner.borrow_mut();
        g.ingest()?;
        Ok(g.payload.pop())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Frame;
    use crate::test_support::MockTransport;

    #[test]
    fn ack_goes_to_tx_data_goes_to_rx() {
        let mut incoming = [0u8; 8];
        incoming[..4].copy_from_slice(&Frame::data(0, 0x42).to_bytes());
        incoming[4..].copy_from_slice(&Frame::ack(0).to_bytes());
        let face = LinkFace::new(MockTransport::with_incoming(&incoming));
        let (mut tx, mut rx) = face.split();

        let mut data = [0u8; 4];
        let mut i = 0;
        while i < 4 {
            data[i] = match rx.read_byte() {
                Ok(Some(b)) => b,
                _ => 0,
            };
            i += 1;
        }
        assert_eq!(data, Frame::data(0, 0x42).to_bytes());

        let mut ack = [0u8; 4];
        i = 0;
        while i < 4 {
            ack[i] = match tx.read_byte() {
                Ok(Some(b)) => b,
                _ => 0,
            };
            i += 1;
        }
        assert_eq!(ack, Frame::ack(0).to_bytes());
    }

    #[test]
    fn crc_miss_reaches_rx_not_tx() {
        let mut bad = Frame::data(0, 0x42).to_bytes();
        bad[3] ^= 0xFF;
        let face = LinkFace::new(MockTransport::with_incoming(&bad));
        let (mut tx, mut rx) = face.split();

        let mut got = [0u8; 4];
        let mut i = 0;
        while i < 4 {
            got[i] = match rx.read_byte() {
                Ok(Some(b)) => b,
                _ => 0,
            };
            i += 1;
        }
        assert_eq!(got, bad);
        assert_eq!(tx.read_byte(), Ok(None));
    }

    #[test]
    fn writes_reach_the_physical_port() {
        let mut face = LinkFace::new(MockTransport::with_incoming(&[]));
        {
            let (mut tx, mut rx) = face.split();
            assert_eq!(tx.write_byte(0xAA), Ok(()));
            assert_eq!(rx.write_byte(0xBB), Ok(()));
        }
        assert_eq!(face.get_mut().written(), &[0xAA, 0xBB]);
    }
}

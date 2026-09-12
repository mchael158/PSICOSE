//! One physical duplex wire, two readers on each end.
//!
//! A real UART has one incoming stream. The sender needs ACK/NACK from
//! it; the receiver needs DATA/START/FINISH/ABORT. This type demuxes
//! complete frames so the two readers do not steal each other's bytes.
//!
//! ```text
//! Peer A                         Peer B
//! TX.write ──┐                 ┌── RX.read  (DATA/START/…)
//! RX.write ──┼── a_to_b ───────┤
//!            │                 └── TX.read  (ACK/NACK)
//! TX.read  ◄─┤
//! RX.read  ◄─┼── b_to_a ───────┤
//!            │                 └── TX.write / RX.write
//! ```

use core::cell::RefCell;

use crate::error::Error;
use crate::protocol::{FrameAssembler, FrameType, FRAME_LEN};
use crate::pump::{Pump, PumpEvent, WindowedPump};
use crate::timeout::RetryPolicy;
use crate::transport::{ByteSink, ByteSource, ByteTransport};
use crate::tx::TxState;
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

struct Lane {
    wire: Ring,
    assembler: FrameAssembler,
    control: Ring,
    payload: Ring,
}

impl Lane {
    const fn new() -> Self {
        Lane {
            wire: Ring::new(),
            assembler: FrameAssembler::new(),
            control: Ring::new(),
            payload: Ring::new(),
        }
    }

    fn ingest(&mut self) {
        while let Some(byte) = self.wire.pop() {
            let Some(result) = self.assembler.push(byte) else {
                continue;
            };
            let bytes = match result {
                Ok(frame) => frame.to_bytes(),
                Err(_) => self.assembler.last_bytes(),
            };
            let ok = match result {
                Ok(frame) if matches!(frame.frame_type(), FrameType::Ack | FrameType::Nack) => {
                    self.control.push_frame(bytes)
                }
                _ => self.payload.push_frame(bytes),
            };
            if !ok {
                return;
            }
        }
    }
}

/// Two lanes (A→B and B→A). Interior mutability so four ports share it.
pub struct DuplexWire {
    lanes: RefCell<Lanes>,
}

struct Lanes {
    a_to_b: Lane,
    b_to_a: Lane,
}

impl DuplexWire {
    /// Empty wire.
    pub const fn new() -> Self {
        DuplexWire {
            lanes: RefCell::new(Lanes {
                a_to_b: Lane::new(),
                b_to_a: Lane::new(),
            }),
        }
    }

    /// Four ports: A's TX, A's RX, B's TX, B's RX.
    pub fn ends(
        &self,
    ) -> (
        DuplexPort<'_>,
        DuplexPort<'_>,
        DuplexPort<'_>,
        DuplexPort<'_>,
    ) {
        (
            DuplexPort {
                wire: self,
                is_a: true,
                for_tx: true,
            },
            DuplexPort {
                wire: self,
                is_a: true,
                for_tx: false,
            },
            DuplexPort {
                wire: self,
                is_a: false,
                for_tx: true,
            },
            DuplexPort {
                wire: self,
                is_a: false,
                for_tx: false,
            },
        )
    }

    /// Two stop-and-wait pumps: `let (a, b) = wire.pumps();`
    pub fn pumps(
        &self,
    ) -> (
        Pump<DuplexPort<'_>, DuplexPort<'_>>,
        Pump<DuplexPort<'_>, DuplexPort<'_>>,
    ) {
        let (a_tx, a_rx, b_tx, b_rx) = self.ends();
        (Pump::on(a_tx, a_rx), Pump::on(b_tx, b_rx))
    }

    /// [`Self::pumps`] with an explicit [`RetryPolicy`].
    pub fn pumps_with(
        &self,
        policy: RetryPolicy,
    ) -> (
        Pump<DuplexPort<'_>, DuplexPort<'_>>,
        Pump<DuplexPort<'_>, DuplexPort<'_>>,
    ) {
        let (a_tx, a_rx, b_tx, b_rx) = self.ends();
        (
            Pump::on_with(a_tx, a_rx, policy),
            Pump::on_with(b_tx, b_rx, policy),
        )
    }

    /// Two windowed pumps for [`super::PeerLink`] (`N = MAX_WINDOW`).
    ///
    /// Handshake runs with limit 1; after negotiate + `WINDOW`, the link
    /// raises the runtime limit to the agreed `max_window`.
    pub fn link_pumps(
        &self,
    ) -> (
        WindowedPump<DuplexPort<'_>, DuplexPort<'_>, MAX_WINDOW>,
        WindowedPump<DuplexPort<'_>, DuplexPort<'_>, MAX_WINDOW>,
    ) {
        let (a_tx, a_rx, b_tx, b_rx) = self.ends();
        let mut a = WindowedPump::on(a_tx, a_rx);
        let mut b = WindowedPump::on(b_tx, b_rx);
        a.set_window_limit(1);
        b.set_window_limit(1);
        (a, b)
    }

    /// [`Self::link_pumps`] with an explicit [`RetryPolicy`].
    pub fn link_pumps_with(
        &self,
        policy: RetryPolicy,
    ) -> (
        WindowedPump<DuplexPort<'_>, DuplexPort<'_>, MAX_WINDOW>,
        WindowedPump<DuplexPort<'_>, DuplexPort<'_>, MAX_WINDOW>,
    ) {
        let (a_tx, a_rx, b_tx, b_rx) = self.ends();
        let mut a = WindowedPump::on_with(a_tx, a_rx, policy);
        let mut b = WindowedPump::on_with(b_tx, b_rx, policy);
        a.set_window_limit(1);
        b.set_window_limit(1);
        (a, b)
    }

    /// Framework motor: stop-and-wait copy **A → B** on this wire.
    ///
    /// Examples call this instead of inventing their own UART ring. The
    /// pumps, ACKs, and CRC path are entirely PSICOSE.
    pub fn copy<S, K>(&self, src: &mut S, sink: &mut K) -> Result<usize, WireCopyError>
    where
        S: ByteSource,
        K: ByteSink,
    {
        let (mut a, mut b) = self.pumps();
        let mut hold: Option<u8> = None;
        let mut exhausted = false;
        let mut finish_offered = false;
        let mut n = 0usize;

        for _ in 0..1_000_000 {
            if hold.is_none() && !exhausted {
                match src.read_byte() {
                    Ok(Some(byte)) => hold = Some(byte),
                    Ok(None) => exhausted = true,
                    Err(_) => return Err(WireCopyError::Source),
                }
            }

            if a.sender().state() == TxState::Idle {
                if let Some(byte) = hold.take() {
                    if a.sender_mut().offer(byte).is_err() {
                        hold = Some(byte);
                    }
                } else if exhausted && !finish_offered {
                    match a.sender_mut().offer_finish() {
                        Ok(()) => finish_offered = true,
                        Err(Error::NotIdle) => {}
                        Err(_) => return Err(WireCopyError::Protocol),
                    }
                }
            }

            match a.poll() {
                Ok(PumpEvent::Aborted) => return Err(WireCopyError::Protocol),
                Ok(PumpEvent::Completed) if finish_offered => {}
                Ok(_) => {}
                Err(_) => return Err(WireCopyError::Protocol),
            }

            match b.poll() {
                Ok(PumpEvent::Received(byte)) => {
                    if sink.write_byte(byte).is_err() {
                        return Err(WireCopyError::Sink);
                    }
                    n = n.saturating_add(1);
                }
                Ok(PumpEvent::Completed) => return Ok(n),
                Ok(PumpEvent::Aborted) => return Err(WireCopyError::Protocol),
                Ok(_) => {}
                Err(_) => return Err(WireCopyError::Protocol),
            }
        }

        Err(WireCopyError::Stalled)
    }
}

/// Why [`DuplexWire::copy`] stopped early.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireCopyError {
    /// [`ByteSource`] failed.
    Source,
    /// [`ByteSink`] failed.
    Sink,
    /// Protocol / transport error (ABORT, retries, …).
    Protocol,
    /// Idle budget style stall (too many polls, no progress).
    Stalled,
}

impl core::fmt::Display for WireCopyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            WireCopyError::Source => write!(f, "wire copy: source error"),
            WireCopyError::Sink => write!(f, "wire copy: sink error"),
            WireCopyError::Protocol => write!(f, "wire copy: protocol error"),
            WireCopyError::Stalled => write!(f, "wire copy: stalled"),
        }
    }
}

impl Default for DuplexWire {
    fn default() -> Self {
        Self::new()
    }
}

/// One half of one peer: either the sender port or the receiver port.
pub struct DuplexPort<'a> {
    wire: &'a DuplexWire,
    is_a: bool,
    for_tx: bool,
}

/// The heapless ring had no room.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DuplexFull;

impl ByteTransport for DuplexPort<'_> {
    type Error = DuplexFull;

    fn write_byte(&mut self, byte: u8) -> Result<(), Self::Error> {
        let mut d = self.wire.lanes.borrow_mut();
        let lane = if self.is_a {
            &mut d.a_to_b
        } else {
            &mut d.b_to_a
        };
        if lane.wire.push(byte) {
            Ok(())
        } else {
            Err(DuplexFull)
        }
    }

    fn read_byte(&mut self) -> Result<Option<u8>, Self::Error> {
        let mut d = self.wire.lanes.borrow_mut();
        let lane = if self.is_a {
            &mut d.b_to_a
        } else {
            &mut d.a_to_b
        };
        lane.ingest();
        Ok(if self.for_tx {
            lane.control.pop()
        } else {
            lane.payload.pop()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Frame;

    #[test]
    fn ack_goes_to_tx_data_goes_to_rx() {
        let wire = DuplexWire::new();
        let (mut a_tx, mut a_rx, mut b_tx, mut b_rx) = wire.ends();

        for b in Frame::data(0, 0x42).to_bytes() {
            assert_eq!(a_tx.write_byte(b), Ok(()));
        }
        for b in Frame::ack(0).to_bytes() {
            assert_eq!(b_rx.write_byte(b), Ok(()));
        }

        let mut data = [0u8; 4];
        let mut i = 0;
        while i < 4 {
            data[i] = match b_rx.read_byte() {
                Ok(Some(b)) => b,
                _ => 0,
            };
            i += 1;
        }
        assert_eq!(data, Frame::data(0, 0x42).to_bytes());

        let mut ack = [0u8; 4];
        i = 0;
        while i < 4 {
            ack[i] = match a_tx.read_byte() {
                Ok(Some(b)) => b,
                _ => 0,
            };
            i += 1;
        }
        assert_eq!(ack, Frame::ack(0).to_bytes());

        assert_eq!(a_rx.read_byte(), Ok(None));
        assert_eq!(b_tx.read_byte(), Ok(None));
    }

    #[test]
    fn crc_miss_reaches_rx_not_tx() {
        let wire = DuplexWire::new();
        let (mut a_tx, _a_rx, _b_tx, mut b_rx) = wire.ends();

        let mut bad = Frame::data(0, 0x42).to_bytes();
        bad[3] ^= 0xFF;
        for b in bad {
            assert_eq!(a_tx.write_byte(b), Ok(()));
        }

        let mut got = [0u8; 4];
        let mut i = 0;
        while i < 4 {
            got[i] = match b_rx.read_byte() {
                Ok(Some(b)) => b,
                _ => 0,
            };
            i += 1;
        }
        assert_eq!(got, bad);
        assert_eq!(a_tx.read_byte(), Ok(None));
    }
}

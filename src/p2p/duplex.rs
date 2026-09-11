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

use crate::protocol::{FrameAssembler, FrameType, FRAME_LEN};
use crate::pump::Pump;
use crate::timeout::RetryPolicy;
use crate::transport::ByteTransport;

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
    pub fn ends(&self) -> (DuplexPort<'_>, DuplexPort<'_>, DuplexPort<'_>, DuplexPort<'_>) {
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

    /// Two pumps, default retry policy: `let (a, b) = wire.pumps();`
    pub fn pumps(&self) -> (Pump<DuplexPort<'_>, DuplexPort<'_>>, Pump<DuplexPort<'_>, DuplexPort<'_>>) {
        let (a_tx, a_rx, b_tx, b_rx) = self.ends();
        (Pump::on(a_tx, a_rx), Pump::on(b_tx, b_rx))
    }

    /// [`Self::pumps`] with an explicit [`RetryPolicy`].
    pub fn pumps_with(
        &self,
        policy: RetryPolicy,
    ) -> (Pump<DuplexPort<'_>, DuplexPort<'_>>, Pump<DuplexPort<'_>, DuplexPort<'_>>) {
        let (a_tx, a_rx, b_tx, b_rx) = self.ends();
        (
            Pump::on_with(a_tx, a_rx, policy),
            Pump::on_with(b_tx, b_rx, policy),
        )
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

//! Reassembles a [`Frame`] from a stream of individually-polled bytes.
//!
//! [`ByteTransport::read_byte`](crate::transport::ByteTransport::read_byte)
//! hands back at most one byte per call, so both the sender (waiting for an
//! ACK/NACK) and the receiver (waiting for DATA) need to accumulate
//! [`FRAME_LEN`] bytes before they have anything to interpret. This is that
//! accumulator, factored out once so both sides share the exact same
//! framing logic instead of two hand-rolled, possibly-divergent copies.

use super::frame::{Frame, FrameError, FRAME_LEN};

/// A fixed-size, stack-allocated buffer that fills up one byte at a time
/// and yields a decoded [`Frame`] (or a [`FrameError`]) once full.
#[derive(Debug, Clone, Copy)]
pub struct FrameAssembler {
    buf: [u8; FRAME_LEN],
    filled: usize,
}

impl FrameAssembler {
    /// Creates an empty assembler.
    pub const fn new() -> Self {
        FrameAssembler {
            buf: [0; FRAME_LEN],
            filled: 0,
        }
    }

    /// Discards any partially-accumulated bytes. Used when a frame boundary
    /// is known to be invalid (e.g. after a CRC error) and the assembler
    /// should not try to interpret stale bytes as part of the next frame.
    pub fn reset(&mut self) {
        self.filled = 0;
    }

    /// Feeds one byte into the assembler.
    ///
    /// Returns `None` if the frame is not yet complete. Returns
    /// `Some(Ok(frame))` once [`FRAME_LEN`] bytes have been accumulated and
    /// they decode into a valid frame, or `Some(Err(_))` if they don't —
    /// either way, the assembler is reset and ready for the next frame.
    pub fn push(&mut self, byte: u8) -> Option<Result<Frame, FrameError>> {
        self.buf[self.filled] = byte;
        self.filled += 1;

        if self.filled < FRAME_LEN {
            return None;
        }

        let result = Frame::from_bytes(self.buf);
        self.filled = 0;
        Some(result)
    }
}

impl Default for FrameAssembler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::FrameType;

    #[test]
    fn yields_none_until_full() {
        let mut asm = FrameAssembler::new();
        let bytes = Frame::data_frame(3, 0x55).to_bytes();
        assert!(asm.push(bytes[0]).is_none());
        assert!(asm.push(bytes[1]).is_none());
        assert!(asm.push(bytes[2]).is_none());
        let frame = asm.push(bytes[3]).unwrap().unwrap();
        assert_eq!(frame.frame_type(), FrameType::Data);
        assert_eq!(frame.seq(), 3);
        assert_eq!(frame.payload(), 0x55);
    }

    #[test]
    fn resets_after_completion_and_accepts_next_frame() {
        let mut asm = FrameAssembler::new();
        let first = Frame::ack(1).to_bytes();
        let second = Frame::nack(2).to_bytes();

        for b in first {
            asm.push(b);
        }
        let mut last = None;
        for b in second {
            last = asm.push(b);
        }
        assert_eq!(last.unwrap().unwrap(), Frame::nack(2));
    }

    #[test]
    fn explicit_reset_discards_partial_frame() {
        let mut asm = FrameAssembler::new();
        asm.push(0xFF);
        asm.push(0xFF);
        asm.reset();

        let bytes = Frame::ack(9).to_bytes();
        let mut last = None;
        for b in bytes {
            last = asm.push(b);
        }
        assert_eq!(last.unwrap().unwrap(), Frame::ack(9));
    }

    #[test] 
    fn propagates_decode_errors_and_still_resets() {
        let mut asm = FrameAssembler::new();
        let mut bytes = Frame::data_frame(1, 1).to_bytes();
        bytes[0] = 0x00; // invalid type
        let mut last = None;
        for b in bytes {
            last = asm.push(b);
        }
        assert!(last.unwrap().is_err());

        // Assembler must be usable again after an error.
        let ok = Frame::ack(5).to_bytes();
        let mut last2 = None;
        for b in ok {
            last2 = asm.push(b);
        }
        assert_eq!(last2.unwrap().unwrap(), Frame::ack(5));
    }
}

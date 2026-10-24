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
    /// The window that just completed, kept after the buffer slides.
    last: [u8; FRAME_LEN],
    filled: usize,
    /// When set, a window whose first byte is not a frame type drops that
    /// byte and retries. A CRC miss keeps the boundary: a bit flip must
    /// not shift every later frame.
    resync: bool,
}

impl FrameAssembler {
    /// Creates an empty assembler.
    pub const fn new() -> Self {
        FrameAssembler {
            buf: [0; FRAME_LEN],
            last: [0; FRAME_LEN],
            filled: 0,
            resync: false,
        }
    }

    /// Like [`new`](Self::new), but an unknown type byte is discarded and
    /// the other three stay. The next byte tries the boundary one step later.
    ///
    /// Use this on the raw byte stream only. A CRC miss still consumes the
    /// whole window, so a bit flip does not move the next frame.
    pub const fn resyncing() -> Self {
        FrameAssembler {
            buf: [0; FRAME_LEN],
            last: [0; FRAME_LEN],
            filled: 0,
            resync: true,
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
    /// they decode into a valid frame, or `Some(Err(_))` if they don't.
    ///
    /// A CRC or semantic failure clears the window. With
    /// [`resyncing`](Self::resyncing), an unknown type keeps the last three
    /// bytes instead.
    pub fn push(&mut self, byte: u8) -> Option<Result<Frame, FrameError>> {
        if self.filled >= FRAME_LEN {
            self.filled = 0;
        }
        self.buf[self.filled] = byte;
        self.filled += 1;

        if self.filled < FRAME_LEN {
            return None;
        }

        let window = self.buf;
        self.last = window;
        let result = Frame::from_bytes(window);
        let slide = self.resync && matches!(result, Err(FrameError::InvalidType(_)));
        if slide {
            self.buf[0] = window[1];
            self.buf[1] = window[2];
            self.buf[2] = window[3];
            self.filled = FRAME_LEN - 1;
        } else {
            self.filled = 0;
        }
        Some(result)
    }

    /// The four bytes of the last completed candidate (valid or not).
    ///
    /// Meaningful only immediately after [`Self::push`] returned `Some`.
    /// A resync slide does not overwrite this: the demux can still hand
    /// the rejected window to RX.
    pub const fn last_bytes(&self) -> [u8; FRAME_LEN] {
        self.last
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
        let bytes = Frame::data(3, 0x55).to_bytes();
        assert!(asm.push(bytes[0]).is_none());
        assert!(asm.push(bytes[1]).is_none());
        assert!(asm.push(bytes[2]).is_none());
        assert_eq!(asm.push(bytes[3]), Some(Ok(Frame::data(3, 0x55))));
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
        assert_eq!(last, Some(Ok(Frame::nack(2))));
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
        assert_eq!(last, Some(Ok(Frame::ack(9))));
    }

    #[test]
    fn propagates_decode_errors_and_still_resets() {
        let mut asm = FrameAssembler::new();
        let mut bytes = Frame::data(1, 1).to_bytes();
        bytes[0] = 0x00; // invalid type
        let mut last = None;
        for b in bytes {
            last = asm.push(b);
        }
        assert!(matches!(last, Some(Err(_))));

        // Assembler must be usable again after an error.
        let ok = Frame::ack(5).to_bytes();
        let mut last2 = None;
        for b in ok {
            last2 = asm.push(b);
        }
        assert_eq!(last2, Some(Ok(Frame::ack(5))));
    }

    #[test]
    fn a_dropped_byte_does_not_pin_the_next_frame() {
        let mut asm = FrameAssembler::resyncing();
        let good = Frame::data(1, 0x42).to_bytes();
        let mut stream = [0u8; 5];
        stream[0] = 0x00;
        stream[1..].copy_from_slice(&good);

        let mut got = None;
        for byte in stream {
            if let Some(Ok(frame)) = asm.push(byte) {
                got = Some(frame);
            }
        }
        assert_eq!(got, Some(Frame::data(1, 0x42)));
    }

    #[test]
    fn a_semantic_reject_keeps_the_next_boundary() {
        let header = [FrameType::Ack as u8, 1, 0xFF];
        let bad = [
            header[0],
            header[1],
            header[2],
            crate::protocol::crc8(&header),
        ];
        let next = Frame::ack(2).to_bytes();
        let mut asm = FrameAssembler::resyncing();
        let mut saw_semantics = false;
        let mut got = None;
        for byte in bad.into_iter().chain(next) {
            match asm.push(byte) {
                Some(Err(FrameError::InvalidSemantics)) => saw_semantics = true,
                Some(Ok(frame)) => got = Some(frame),
                _ => {}
            }
        }
        assert!(saw_semantics);
        assert_eq!(got, Some(Frame::ack(2)));
    }

    #[test]
    fn a_crc_miss_does_not_shift_the_next_frame() {
        let mut bad = Frame::data(1, 0x11).to_bytes();
        bad[3] ^= 0xFF;
        let good = Frame::data(2, 0x22).to_bytes();
        let mut asm = FrameAssembler::resyncing();
        let mut saw_crc = false;
        let mut got = None;
        for byte in bad.into_iter().chain(good) {
            match asm.push(byte) {
                Some(Err(FrameError::CrcMismatch { .. })) => saw_crc = true,
                Some(Ok(frame)) => got = Some(frame),
                _ => {}
            }
        }
        assert!(saw_crc);
        assert_eq!(got, Some(Frame::data(2, 0x22)));
    }
}

//! One-byte-at-a-time outgoing frame buffer.
//!
//! Hardware links (`ByteTransport`) accept at most one byte per call.
//! A 4-byte frame is therefore four polls, not one. [`OutBuf`] holds the
//! frame and yields the next byte only after the caller has successfully
//! written the previous one — a failed `write_byte` does not lose a byte.

use super::frame::{Frame, FRAME_LEN};

/// Stack buffer for a frame that is being written a byte at a time.
#[derive(Debug, Clone, Copy)]
pub struct OutBuf {
    buf: [u8; FRAME_LEN],
    pos: usize,
}

impl OutBuf {
    /// Empty: nothing to write.
    pub const fn empty() -> Self {
        OutBuf {
            buf: [0; FRAME_LEN],
            pos: FRAME_LEN,
        }
    }

    /// True when no frame is in progress.
    pub const fn is_idle(&self) -> bool {
        self.pos >= FRAME_LEN
    }

    /// Loads `frame` and starts at the first byte.
    pub fn load(&mut self, frame: &Frame) {
        self.buf = frame.to_bytes();
        self.pos = 0;
    }

    /// The next byte to write, if any. Does not advance.
    pub const fn peek(&self) -> Option<u8> {
        if self.pos >= FRAME_LEN {
            None
        } else {
            Some(self.buf[self.pos])
        }
    }

    /// Call only after a successful `write_byte` of [`OutBuf::peek`].
    pub fn commit(&mut self) {
        if self.pos < FRAME_LEN {
            self.pos += 1;
        }
    }
}

impl Default for OutBuf {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Frame;

    #[test]
    fn empty_has_nothing_to_write() {
        assert!(OutBuf::empty().is_idle());
        assert_eq!(OutBuf::empty().peek(), None);
    }

    #[test]
    fn yields_four_bytes_then_goes_idle() {
        let frame = Frame::ack(7);
        let expected = frame.to_bytes();
        let mut out = OutBuf::empty();
        out.load(&frame);
        assert!(!out.is_idle());
        for (i, &want) in expected.iter().enumerate() {
            assert_eq!(out.peek(), Some(want), "byte {i}");
            out.commit();
        }
        assert!(out.is_idle());
        assert_eq!(out.peek(), None);
    }

    #[test]
    fn peek_without_commit_repeats_the_same_byte() {
        let mut out = OutBuf::empty();
        out.load(&Frame::nack(1));
        let first = out.peek();
        assert_eq!(out.peek(), first);
        out.commit();
        assert_ne!(out.peek(), first);
    }
}

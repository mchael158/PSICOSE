//! Heapless test doubles. Compiled only under `#[cfg(test)]`.
//! Stack arrays only — no `Vec`, no `VecDeque`, no allocator.

use crate::transport::ByteTransport;

const CAP: usize = 64;

/// Concatenate two 4-byte frames into one stack buffer.
pub fn concat2(a: [u8; 4], b: [u8; 4]) -> [u8; 8] {
    let mut out = [0u8; 8];
    out[..4].copy_from_slice(&a);
    out[4..].copy_from_slice(&b);
    out
}

/// Concatenate three 4-byte frames into one stack buffer.
pub fn concat3(a: [u8; 4], b: [u8; 4], c: [u8; 4]) -> [u8; 12] {
    let mut out = [0u8; 12];
    out[..4].copy_from_slice(&a);
    out[4..8].copy_from_slice(&b);
    out[8..].copy_from_slice(&c);
    out
}

/// Outgoing script buffer is full ([`CAP`] bytes). Matches `DuplexFull` /
/// `LinkFull`: overflow is an error, never a silent drop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MockFull;

/// A scripted, single-ended in-memory transport backed by fixed arrays.
pub struct MockTransport {
    incoming: [u8; CAP],
    incoming_len: usize,
    incoming_pos: usize,
    written_buf: [u8; CAP],
    written_len: usize,
}

impl MockTransport {
    /// Yields exactly `bytes` from `read_byte`, then `Ok(None)`.
    ///
    /// If `bytes.len() > CAP`, only the first [`CAP`] bytes are kept.
    pub fn with_incoming(bytes: &[u8]) -> Self {
        let n = core::cmp::min(bytes.len(), CAP);
        let mut incoming = [0u8; CAP];
        incoming[..n].copy_from_slice(&bytes[..n]);
        MockTransport {
            incoming,
            incoming_len: n,
            incoming_pos: 0,
            written_buf: [0u8; CAP],
            written_len: 0,
        }
    }

    /// Bytes written so far.
    pub fn written(&self) -> &[u8] {
        &self.written_buf[..self.written_len]
    }
}

impl ByteTransport for MockTransport {
    type Error = MockFull;

    fn write_byte(&mut self, byte: u8) -> Result<(), Self::Error> {
        if self.written_len >= CAP {
            return Err(MockFull);
        }
        self.written_buf[self.written_len] = byte;
        self.written_len += 1;
        Ok(())
    }

    fn read_byte(&mut self) -> Result<Option<u8>, Self::Error> {
        if self.incoming_pos >= self.incoming_len {
            return Ok(None);
        }
        let byte = self.incoming[self.incoming_pos];
        self.incoming_pos += 1;
        Ok(Some(byte))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_full_is_an_error_not_a_silent_drop() {
        let mut t = MockTransport::with_incoming(&[]);
        for i in 0..CAP {
            assert_eq!(t.write_byte(i as u8), Ok(()));
        }
        assert_eq!(t.written().len(), CAP);
        assert_eq!(t.write_byte(0xFF), Err(MockFull));
        assert_eq!(t.written().len(), CAP);
        assert_eq!(t.written()[CAP - 1], (CAP - 1) as u8);
    }
}

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

/// Concatenate four 4-byte frames into one stack buffer.
#[allow(dead_code)]
pub fn concat4(a: [u8; 4], b: [u8; 4], c: [u8; 4], d: [u8; 4]) -> [u8; 16] {
    let mut out = [0u8; 16];
    out[..4].copy_from_slice(&a);
    out[4..8].copy_from_slice(&b);
    out[8..12].copy_from_slice(&c);
    out[12..].copy_from_slice(&d);
    out
}

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
    type Error = core::convert::Infallible;

    fn write_byte(&mut self, byte: u8) -> Result<(), Self::Error> {
        if self.written_len < CAP {
            self.written_buf[self.written_len] = byte;
            self.written_len += 1;
        }
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

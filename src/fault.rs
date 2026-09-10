//! Fault injection: a heapless wrapper that deliberately damages the wire.
//!
//! This is not a production transport. It exists so the protocol can be
//! shown to survive drop, corruption, and replay *before* anyone points
//! it at UART/SPI/radio.

use crate::transport::ByteTransport;

/// When to drop or corrupt bytes/frames. `0` on a period means "never".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FaultPolicy {
    /// Drop the first `n` complete frames (or bytes, if not
    /// [`FaultPolicy::frame_granularity`]), then stop dropping for this reason.
    pub drop_first: u32,
    /// Drop every Nth frame/byte (1-based count after each emit).
    pub drop_every: u32,
    /// Corrupt the first `n` frames/bytes, then stop corrupting for this
    /// reason.
    pub corrupt_first: u32,
    /// Corrupt every Nth frame/byte (XOR on the CRC byte of a frame, or
    /// on the byte itself).
    pub corrupt_every: u32,
    /// Count in units of 4-byte frames. Always `true` for write-side
    /// policies used with PSICOSE (the protocol writes whole frames).
    pub frame_granularity: bool,
}

impl FaultPolicy {
    /// A clean wire.
    pub const fn none() -> Self {
        FaultPolicy {
            drop_first: 0,
            drop_every: 0,
            corrupt_first: 0,
            corrupt_every: 0,
            frame_granularity: true,
        }
    }

    /// Drop the first `n` frames, then pass everything.
    pub const fn drop_first(n: u32) -> Self {
        FaultPolicy {
            drop_first: n,
            drop_every: 0,
            corrupt_first: 0,
            corrupt_every: 0,
            frame_granularity: true,
        }
    }

    /// Drop every `n`th frame (2 = drop 2, 4, 6, …).
    pub const fn drop_every(n: u32) -> Self {
        FaultPolicy {
            drop_first: 0,
            drop_every: n,
            corrupt_first: 0,
            corrupt_every: 0,
            frame_granularity: true,
        }
    }

    /// Break the CRC of the first `n` frames, then pass clean frames.
    pub const fn corrupt_first(n: u32) -> Self {
        FaultPolicy {
            drop_first: 0,
            drop_every: 0,
            corrupt_first: n,
            corrupt_every: 0,
            frame_granularity: true,
        }
    }

    /// Break the CRC of every `n`th frame.
    pub const fn corrupt_every(n: u32) -> Self {
        FaultPolicy {
            drop_first: 0,
            drop_every: 0,
            corrupt_first: 0,
            corrupt_every: n,
            frame_granularity: true,
        }
    }

    fn hits(count: u32, every: u32) -> bool {
        every != 0 && count != 0 && count % every == 0
    }

    fn should_drop(self, count: u32) -> bool {
        (self.drop_first != 0 && count <= self.drop_first) || Self::hits(count, self.drop_every)
    }

    fn should_corrupt(self, count: u32) -> bool {
        (self.corrupt_first != 0 && count <= self.corrupt_first)
            || Self::hits(count, self.corrupt_every)
    }
}

impl Default for FaultPolicy {
    fn default() -> Self {
        Self::none()
    }
}

/// Wraps a [`ByteTransport`] and applies [`FaultPolicy`] on write and/or
/// read. Write faults are frame-buffered (4 bytes) so a "lost DATA" is a
/// lost *frame*, not a torn one — unless [`FaultPolicy::frame_granularity`] is `false`.
pub struct FaultyTransport<T> {
    inner: T,
    write_policy: FaultPolicy,
    read_policy: FaultPolicy,
    writes: u32,
    reads: u32,
    write_buf: [u8; 4],
    write_fill: usize,
    /// First `n` `read_byte` calls return `Ok(None)` without consuming
    /// the inner stream. Models a delayed frame, not a lost one.
    hold_reads: u32,
    held: u32,
}

impl<T> FaultyTransport<T> {
    /// Apply `write_policy` to outgoing bytes and `read_policy` to incoming.
    pub const fn new(inner: T, write_policy: FaultPolicy, read_policy: FaultPolicy) -> Self {
        FaultyTransport {
            inner,
            write_policy,
            read_policy,
            writes: 0,
            reads: 0,
            write_buf: [0; 4],
            write_fill: 0,
            hold_reads: 0,
            held: 0,
        }
    }

    /// Delay the first `n` reads (`Ok(None)`), then pass the inner stream.
    pub const fn with_read_hold(mut self, n: u32) -> Self {
        self.hold_reads = n;
        self
    }

    /// Faults only on writes (DATA from a sender, ACK from a receiver).
    pub const fn on_write(inner: T, write_policy: FaultPolicy) -> Self {
        Self::new(inner, write_policy, FaultPolicy::none())
    }

    /// The wrapped transport.
    pub fn inner(&self) -> &T {
        &self.inner
    }

    /// The wrapped transport, mutably.
    pub fn inner_mut(&mut self) -> &mut T {
        &mut self.inner
    }

    /// How many write-units (frames or bytes) have been counted.
    pub const fn writes(&self) -> u32 {
        self.writes
    }
}

impl<T: ByteTransport> ByteTransport for FaultyTransport<T> {
    type Error = T::Error;

    fn write_byte(&mut self, byte: u8) -> Result<(), Self::Error> {
        if !self.write_policy.frame_granularity {
            self.writes = self.writes.saturating_add(1);
            if self.write_policy.should_drop(self.writes) {
                return Ok(());
            }
            let out = if self.write_policy.should_corrupt(self.writes) {
                byte ^ 0x01
            } else {
                byte
            };
            return self.inner.write_byte(out);
        }

        self.write_buf[self.write_fill] = byte;
        self.write_fill += 1;
        if self.write_fill < 4 {
            return Ok(());
        }
        self.write_fill = 0;
        self.writes = self.writes.saturating_add(1);
        if self.write_policy.should_drop(self.writes) {
            return Ok(());
        }
        if self.write_policy.should_corrupt(self.writes) {
            self.write_buf[3] ^= 0xFF;
        }
        let frame = self.write_buf;
        for b in frame {
            self.inner.write_byte(b)?;
        }
        Ok(())
    }

    fn read_byte(&mut self) -> Result<Option<u8>, Self::Error> {
        if self.held < self.hold_reads {
            self.held = self.held.saturating_add(1);
            return Ok(None);
        }
        let Some(byte) = self.inner.read_byte()? else {
            return Ok(None);
        };
        self.reads = self.reads.saturating_add(1);
        if self.read_policy.should_drop(self.reads) {
            return Ok(None);
        }
        if self.read_policy.should_corrupt(self.reads) {
            return Ok(Some(byte ^ 0x01));
        }
        Ok(Some(byte))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::MockTransport;

    #[test]
    fn drop_first_write_frame_eats_four_bytes() {
        let inner = MockTransport::with_incoming(&[]);
        let mut f = FaultyTransport::on_write(inner, FaultPolicy::drop_first(1));
        for b in [1u8, 2, 3, 4] {
            f.write_byte(b).unwrap();
        }
        assert_eq!(f.writes(), 1);
        assert!(f.inner().written().is_empty());
        for b in [5u8, 6, 7, 8] {
            f.write_byte(b).unwrap();
        }
        assert_eq!(f.inner().written(), [5, 6, 7, 8]);
    }

    #[test]
    fn corrupt_every_flips_crc_byte() {
        let inner = MockTransport::with_incoming(&[]);
        let mut f = FaultyTransport::on_write(inner, FaultPolicy::corrupt_every(1));
        f.write_byte(0x01).unwrap();
        f.write_byte(0x00).unwrap();
        f.write_byte(0x42).unwrap();
        f.write_byte(0x00).unwrap();
        let w = f.inner().written();
        assert_eq!(w.len(), 4);
        assert_eq!(w[3], 0xFF);
    }
}

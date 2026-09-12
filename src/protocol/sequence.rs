//! A one-byte sequence counter with explicit, tested wraparound behavior.
//!
//! Because `SEQ` is a `u8`, it wraps every 256 frames. That wraparound is
//! not a bug to guard against — it's the intended, load-bearing behavior
//! that lets a stop-and-wait link run forever on constant memory. This
//! type exists so the wraparound happens in exactly one place, with a test
//! pinned to it, instead of being re-derived (and possibly re-broken) at
//! every call site.

/// A monotonically increasing (mod 256) sequence counter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Sequence(u8);

impl Sequence {
    /// Starts a new sequence counter at `0`.
    pub const fn new() -> Self {
        Sequence(0)
    }

    /// Starts a sequence counter at an explicit value (e.g. to test the
    /// `255 → 0` wrap as the *expected* number, not only as `advance`).
    pub const fn from_raw(value: u8) -> Self {
        Sequence(value)
    }

    /// Returns the current sequence number without advancing it.
    pub const fn current(&self) -> u8 {
        self.0
    }

    /// Advances to the next sequence number (wrapping `255 -> 0`) and
    /// returns the value that was current *before* advancing — i.e. the
    /// sequence number that should be stamped on the frame just sent.
    pub fn advance(&mut self) -> u8 {
        let previous = self.0;
        self.0 = self.0.wrapping_add(1);
        previous
    }

    /// The sequence number immediately preceding the current one (wrapping
    /// `0 -> 255`). Used to recognize a retransmitted duplicate of the last
    /// frame that was already accepted and ACKed.
    pub const fn previous(&self) -> u8 {
        self.0.wrapping_sub(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_at_zero() {
        assert_eq!(Sequence::new().current(), 0);
    }

    #[test]
    fn advance_returns_pre_increment_value() {
        let mut seq = Sequence::new();
        assert_eq!(seq.advance(), 0);
        assert_eq!(seq.advance(), 1);
        assert_eq!(seq.current(), 2);
    }

    #[test]
    fn wraps_at_255_to_0() {
        let mut seq = Sequence::new();
        for _ in 0..255 {
            seq.advance();
        }
        assert_eq!(seq.current(), 255);
        assert_eq!(seq.advance(), 255);
        assert_eq!(
            seq.current(),
            0,
            "sequence must wrap 255 -> 0, not overflow"
        );
    }

    #[test]
    fn previous_wraps_0_to_255() {
        let seq = Sequence::new();
        assert_eq!(seq.previous(), 255);
    }

    #[test]
    fn from_raw_preserves_value() {
        assert_eq!(Sequence::from_raw(255).current(), 255);
        assert_eq!(Sequence::from_raw(255).previous(), 254);
    }
}

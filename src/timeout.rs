//! Retry and timeout policy.
//!
//! PSICOSE-1B deliberately does not depend on any notion of wall-clock
//! time, because `no_std` targets don't agree on one (some have a
//! `SysTick`, some have a RTC, some have nothing). Instead, a "timeout" is
//! defined as a number of **poll ticks** — i.e. how many times
//! [`ByteTransport::read_byte`](crate::transport::ByteTransport) was
//! called and returned `Ok(None)` in a row while waiting for a response.
//!
//! This pushes the actual time unit out to the caller: a bare-metal loop
//! polling at 10 kHz and one polling at 10 Hz get very different real-world
//! timeouts from the same `timeout_ticks` value, and that's intentional —
//! the caller is in the best position to know its own poll rate.

/// Governs how long a [`Sender`](crate::tx::Sender) waits for an ACK before
/// retransmitting, and how many times it will retry before giving up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Number of consecutive empty polls to tolerate before declaring a
    /// timeout and retransmitting.
    pub timeout_ticks: u16,
    /// Maximum number of retransmissions for a single frame before the
    /// transfer is abandoned as failed. `0` means "send once, never retry".
    pub max_retries: u8,
}

impl RetryPolicy {
    /// Builds a policy with explicit values.
    pub const fn new(timeout_ticks: u16, max_retries: u8) -> Self {
        RetryPolicy {
            timeout_ticks,
            max_retries,
        }
    }
}

impl Default for RetryPolicy {
    /// A conservative default: 1000 empty polls before timing out, up to
    /// 5 retransmissions. Tune this per link — a slow radio link and a
    /// fast SPI bus have very different sensible defaults.
    fn default() -> Self {
        RetryPolicy {
            timeout_ticks: 1000,
            max_retries: 5,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_conservative_but_bounded() {
        let policy = RetryPolicy::default();
        assert!(policy.timeout_ticks > 0);
        assert!(policy.max_retries > 0);
    }

    #[test]
    fn zero_retries_means_send_once() {
        let policy = RetryPolicy::new(10, 0);
        assert_eq!(policy.max_retries, 0);
    }
}

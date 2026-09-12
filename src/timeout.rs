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

use crate::error::Error;

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
    /// Conservative default: 1000 empty polls, up to 5 retransmissions.
    /// Tune per link — a slow radio and a fast SPI disagree on time.
    pub const DEFAULT: Self = Self::new(1000, 5);

    /// Builds a policy with explicit values.
    pub const fn new(timeout_ticks: u16, max_retries: u8) -> Self {
        RetryPolicy {
            timeout_ticks,
            max_retries,
        }
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Ceiling on consecutive polls with no application-level progress.
///
/// [`RetryPolicy`] bounds **one in-flight frame** (empty reads → retransmit).
/// [`IdleBudget`] bounds an **outer** cooperative loop (`Pump::send_all`,
/// `recv_all`, …) so a silent peer cannot spin forever between those
/// per-frame retries.
///
/// Still tick-based — never wall-clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdleBudget {
    /// `None` means never exhaust (legacy unbounded helpers).
    limit: Option<u32>,
    idle: u32,
}

impl IdleBudget {
    /// Default outer ceiling for cooperative drain loops on live links:
    /// one million consecutive idle polls without application progress.
    ///
    /// Tune with [`Self::new`]. Use [`Self::unbounded`] only for scripted
    /// pairs that cannot hang (unit tests with pre-loaded replies).
    pub const DEFAULT: Self = Self::new(1_000_000);

    /// Never returns [`Error::IdleBudgetExhausted`]. Prefer [`Self::DEFAULT`]
    /// or [`Self::new`] on live links.
    pub const fn unbounded() -> Self {
        IdleBudget {
            limit: None,
            idle: 0,
        }
    }

    /// Fail once more than `max_idle_ticks` consecutive idle polls occur.
    ///
    /// `IdleBudget::new(0)` fails on the first idle observation.
    pub const fn new(max_idle_ticks: u32) -> Self {
        IdleBudget {
            limit: Some(max_idle_ticks),
            idle: 0,
        }
    }

    /// Application progress (byte ACKed / delivered / session event).
    pub fn reset(&mut self) {
        self.idle = 0;
    }

    /// One poll without application progress.
    pub fn tick<E>(&mut self) -> Result<(), Error<E>> {
        let Some(limit) = self.limit else {
            return Ok(());
        };
        self.idle = self.idle.saturating_add(1);
        if self.idle > limit {
            Err(Error::IdleBudgetExhausted)
        } else {
            Ok(())
        }
    }
}

impl Default for IdleBudget {
    fn default() -> Self {
        Self::DEFAULT
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

    #[test]
    fn idle_budget_trips_after_limit() {
        let mut b = IdleBudget::new(2);
        assert_eq!(b.tick::<()>(), Ok(()));
        assert_eq!(b.tick::<()>(), Ok(()));
        assert_eq!(b.tick::<()>(), Err(Error::IdleBudgetExhausted));
    }

    #[test]
    fn idle_budget_reset_clears_streak() {
        let mut b = IdleBudget::new(1);
        assert_eq!(b.tick::<()>(), Ok(()));
        b.reset();
        assert_eq!(b.tick::<()>(), Ok(()));
        assert_eq!(b.tick::<()>(), Err(Error::IdleBudgetExhausted));
    }

    #[test]
    fn unbounded_idle_budget_never_trips() {
        let mut b = IdleBudget::unbounded();
        for _ in 0..10_000 {
            assert_eq!(b.tick::<()>(), Ok(()));
        }
    }
}

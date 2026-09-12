//! Crate-wide error type.

use crate::protocol::FrameError;

/// Errors produced by PSICOSE-1B's TX/RX state machines.
///
/// Generic over `E`, the error type of the underlying
/// [`ByteTransport`](crate::transport::ByteTransport), so a transport
/// failure (e.g. a UART overrun) is never silently swallowed or converted
/// to `()` — it's carried through as-is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error<E> {
    /// The underlying transport returned an error.
    Transport(E),
    /// A received frame failed CRC validation.
    Crc,
    /// A received frame's `TYPE` byte was not a recognized [`FrameType`].
    ///
    /// [`FrameType`]: crate::protocol::FrameType
    InvalidFrameType(u8),
    /// A CRC-valid frame broke the control-frame contract.
    InvalidSemantics,
    /// No valid response was seen within the configured
    /// [`RetryPolicy::timeout_ticks`](crate::timeout::RetryPolicy).
    Timeout,
    /// A frame was retransmitted [`RetryPolicy::max_retries`] times without
    /// a positive acknowledgement; the transfer has been abandoned.
    ///
    /// [`RetryPolicy::max_retries`]: crate::timeout::RetryPolicy::max_retries
    RetriesExhausted,
    /// An outer cooperative loop saw too many consecutive polls with no
    /// application progress ([`IdleBudget`](crate::timeout::IdleBudget)).
    IdleBudgetExhausted,
    /// `offer` / `offer_start` / `offer_finish` was called while the
    /// sender was not in a state that can accept a new flight.
    NotIdle,
    /// The send window already holds `N` unacknowledged frames.
    /// Poll until an ACK frees a slot, then offer again.
    WindowFull,
    /// The session was cancelled by [`FrameType::Abort`](crate::protocol::FrameType::Abort).
    Aborted,
}

impl<E> From<FrameError> for Error<E> {
    fn from(err: FrameError) -> Self {
        match err {
            FrameError::CrcMismatch { .. } => Error::Crc,
            FrameError::InvalidType(byte) => Error::InvalidFrameType(byte),
            FrameError::InvalidSemantics => Error::InvalidSemantics,
        }
    }
}

impl<E: core::fmt::Debug> core::fmt::Display for Error<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::Transport(e) => write!(f, "transport error: {e:?}"),
            Error::Crc => write!(f, "CRC mismatch on received frame"),
            Error::InvalidFrameType(byte) => write!(f, "invalid frame type byte: 0x{byte:02X}"),
            Error::InvalidSemantics => write!(f, "semantically invalid control frame"),
            Error::Timeout => write!(f, "timed out waiting for a response"),
            Error::RetriesExhausted => write!(f, "retransmission budget exhausted"),
            Error::IdleBudgetExhausted => {
                write!(f, "idle budget exhausted (no application progress)")
            }
            Error::NotIdle => write!(f, "sender is not idle"),
            Error::WindowFull => write!(f, "send window is full"),
            Error::Aborted => write!(f, "session aborted"),
        }
    }
}

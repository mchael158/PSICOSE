//! The abstraction boundary between PSICOSE-1B and physical hardware.
//!
//! Everything above this line (frames, state machines, retry policy) is
//! hardware-agnostic. Everything below it is a concrete implementation for
//! UART, SPI, a radio, an in-memory test double, or anything else that can
//! move one byte at a time. The protocol never needs to know which.

/// A byte-oriented, non-blocking, bidirectional link.
///
/// Implementations are expected to be **non-blocking**: [`read_byte`] must
/// return `Ok(None)` immediately if no byte is currently available, rather
/// than blocking the caller. This is what lets [`crate::tx::Sender`] and
/// [`crate::rx::Receiver`] implement their own timeout/retry logic on top,
/// independent of whatever clock or scheduler the host system uses.
///
/// [`read_byte`]: ByteTransport::read_byte
pub trait ByteTransport {
    /// The error type produced by the underlying hardware/IO layer.
    type Error;

    /// Writes a single byte to the link. Implementations may block until
    /// the byte has been accepted by the underlying hardware (e.g. a UART
    /// FIFO), but must not silently drop it.
    fn write_byte(&mut self, byte: u8) -> Result<(), Self::Error>;

    /// Polls for a single received byte. Must return `Ok(None)` rather
    /// than blocking if nothing has arrived yet.
    fn read_byte(&mut self) -> Result<Option<u8>, Self::Error>;
}

/// A read-only source of bytes — e.g. a file, a flash region, or a sensor
/// buffer being streamed out one byte at a time.
///
/// Decoupled from [`ByteTransport`] on purpose: the thing PSICOSE reads
/// application data *from* is a separate concern from the link it sends
/// wire frames *over*, and the two are frequently different types (a file
/// on one side, a UART on the other).
pub trait ByteSource {
    /// The error type produced by the underlying data source.
    type Error;

    /// Returns the next application byte, or `Ok(None)` when exhausted.
    ///
    /// File, flash, and a camera buffer are all just implementations of
    /// this trait. The protocol core does not know which.
    fn read_byte(&mut self) -> Result<Option<u8>, Self::Error>;
}

/// A write-only destination for bytes — the mirror image of [`ByteSource`].
pub trait ByteSink {
    /// The error type produced by the underlying data destination.
    type Error;

    /// Writes one application byte (file, flash, RAM — the core does not
    /// care).
    fn write_byte(&mut self, byte: u8) -> Result<(), Self::Error>;
}

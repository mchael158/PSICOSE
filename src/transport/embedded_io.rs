//! # Feature `embedded-io` — glue into the rest of PSICOSE
//!
//! This module is **not** a UART driver and **not** the protocol. It only
//! converts [`embedded_io`] traits into the traits every other module already
//! uses ([`ByteTransport`], [`ByteSource`], [`ByteSink`]).
//!
//! Enable with:
//!
//! ```toml
//! psicose = { version = "0.3", features = ["embedded-io"] }
//! ```
//!
//! ## Where it sits (whole crate)
//!
//! ```text
//! application bytes
//!        │
//!        ├─ optional: aead          (feature = "aead")
//!        ├─ optional: Fragmenter
//!        ▼
//! PeerLink / Pump / Sender|Receiver|Windowed*
//!        │
//!        ▼
//! ByteTransport  ◄── IoTransport   (THIS MODULE, feature = "embedded-io")
//!        │                 ▲
//!        │                 │ wraps
//!        │         embedded_io::Read + Write + ReadReady
//!        ▼                 │
//!   your UART / SPI ───────┘
//! ```
//!
//! | You have | Wrap with | Then use |
//! | --- | --- | --- |
//! | Duplex port (`Read`+`Write`+`ReadReady`) | [`IoTransport`] | [`crate::Pump::on`], [`crate::WindowedPump::on`], [`crate::PeerLink`] |
//! | Reader only | [`IoSource`] | [`crate::send_all`] / `SliceSource`-style paths |
//! | Writer only | [`IoSink`] | [`crate::recv_all`] sinks |
//!
//! ## Mapping rules
//!
//! - **Read:** [`ReadReady`] → `Ok(None)` when empty (PSICOSE must not block).
//! - **Write:** [`Write::write_all`] of one byte (may wait for FIFO space).
//! - Dependency is **`embedded-io` 0.6** (MSRV 1.75). `0.7` needs rustc 1.81+.
//!
//! ## Field sketch
//!
//! ```ignore
//! use psicose::{IoTransport, Pump};
//!
//! // each half of a split UART (or demuxed duplex end)
//! let mut pump = Pump::on(
//!     IoTransport::new(uart_tx),
//!     IoTransport::new(uart_rx),
//! );
//! ```
//!
//! In-repo examples still use the heapless `End` ring in `examples/common/link.rs`.
//! On hardware, replace that `End` with [`IoTransport`] around your driver.

use embedded_io::{ErrorType, Read, ReadReady, Write};

use crate::transport::traits::{ByteSink, ByteSource, ByteTransport};

/// Wire adapter: [`embedded_io`] duplex → [`ByteTransport`].
///
/// Requires [`Read`] + [`Write`] + [`ReadReady`]. Pass into
/// [`crate::Pump::on`] / [`crate::WindowedPump::on`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct IoTransport<T> {
    inner: T,
}

impl<T> IoTransport<T> {
    /// Wrap `inner`.
    pub const fn new(inner: T) -> Self {
        Self { inner }
    }

    /// Borrow the inner port.
    pub const fn get_ref(&self) -> &T {
        &self.inner
    }

    /// Borrow the inner port mutably.
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.inner
    }

    /// Unwrap the inner port.
    pub fn into_inner(self) -> T {
        self.inner
    }
}

impl<T> ByteTransport for IoTransport<T>
where
    T: Read + Write + ReadReady,
{
    type Error = <T as ErrorType>::Error;

    fn write_byte(&mut self, byte: u8) -> Result<(), Self::Error> {
        self.inner.write_all(&[byte])
    }

    fn read_byte(&mut self) -> Result<Option<u8>, Self::Error> {
        if !self.inner.read_ready()? {
            return Ok(None);
        }
        let mut buf = [0u8; 1];
        match self.inner.read(&mut buf)? {
            0 => Ok(None),
            _ => Ok(Some(buf[0])),
        }
    }
}

/// Application-byte reader: [`embedded_io::Read`] + [`ReadReady`] → [`ByteSource`].
///
/// For payloads (flash / camera / file behind a reader). The **wire** uses
/// [`IoTransport`], not this type.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct IoSource<T> {
    inner: T,
}

impl<T> IoSource<T> {
    /// Wrap `inner`.
    pub const fn new(inner: T) -> Self {
        Self { inner }
    }

    /// Borrow the inner reader.
    pub const fn get_ref(&self) -> &T {
        &self.inner
    }

    /// Borrow the inner reader mutably.
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.inner
    }

    /// Unwrap the inner reader.
    pub fn into_inner(self) -> T {
        self.inner
    }
}

impl<T> ByteSource for IoSource<T>
where
    T: Read + ReadReady,
{
    type Error = <T as ErrorType>::Error;

    fn read_byte(&mut self) -> Result<Option<u8>, Self::Error> {
        if !self.inner.read_ready()? {
            return Ok(None);
        }
        let mut buf = [0u8; 1];
        match self.inner.read(&mut buf)? {
            0 => Ok(None),
            _ => Ok(Some(buf[0])),
        }
    }
}

/// Application-byte writer: [`embedded_io::Write`] → [`ByteSink`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct IoSink<T> {
    inner: T,
}

impl<T> IoSink<T> {
    /// Wrap `inner`.
    pub const fn new(inner: T) -> Self {
        Self { inner }
    }

    /// Borrow the inner writer.
    pub const fn get_ref(&self) -> &T {
        &self.inner
    }

    /// Borrow the inner writer mutably.
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.inner
    }

    /// Unwrap the inner writer.
    pub fn into_inner(self) -> T {
        self.inner
    }
}

impl<T> ByteSink for IoSink<T>
where
    T: Write,
{
    type Error = <T as ErrorType>::Error;

    fn write_byte(&mut self, byte: u8) -> Result<(), Self::Error> {
        self.inner.write_all(&[byte])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pump::{Pump, PumpEvent};
    use embedded_io::{Error, ErrorKind, ErrorType, Read, ReadReady, Write};

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct E;

    impl Error for E {
        fn kind(&self) -> ErrorKind {
            ErrorKind::Other
        }
    }

    impl core::fmt::Display for E {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            write!(f, "e")
        }
    }

    /// Tiny ring: enough to exercise ready / empty / full paths.
    struct Port {
        rx: [u8; 8],
        rx_len: usize,
        tx: [u8; 8],
        tx_len: usize,
    }

    impl Port {
        const fn new() -> Self {
            Self {
                rx: [0; 8],
                rx_len: 0,
                tx: [0; 8],
                tx_len: 0,
            }
        }

        fn push_rx(&mut self, b: u8) {
            self.rx[self.rx_len] = b;
            self.rx_len += 1;
        }
    }

    impl ErrorType for Port {
        type Error = E;
    }

    impl Read for Port {
        fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
            if self.rx_len == 0 || buf.is_empty() {
                return Ok(0);
            }
            buf[0] = self.rx[0];
            self.rx.copy_within(1..self.rx_len, 0);
            self.rx_len -= 1;
            Ok(1)
        }
    }

    impl ReadReady for Port {
        fn read_ready(&mut self) -> Result<bool, Self::Error> {
            Ok(self.rx_len > 0)
        }
    }

    impl Write for Port {
        fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
            if buf.is_empty() {
                return Ok(0);
            }
            if self.tx_len >= self.tx.len() {
                return Err(E);
            }
            self.tx[self.tx_len] = buf[0];
            self.tx_len += 1;
            Ok(1)
        }

        fn flush(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    #[test]
    fn transport_read_none_when_empty() {
        let mut t = IoTransport::new(Port::new());
        assert_eq!(t.read_byte(), Ok(None));
    }

    #[test]
    fn transport_round_trip_one_byte() {
        let mut port = Port::new();
        port.push_rx(0xA5);
        let mut t = IoTransport::new(port);
        assert_eq!(t.read_byte(), Ok(Some(0xA5)));
        assert_eq!(t.read_byte(), Ok(None));
        assert_eq!(t.write_byte(0x5A), Ok(()));
        assert_eq!(t.get_ref().tx_len, 1);
        assert_eq!(t.get_ref().tx[0], 0x5A);
    }

    #[test]
    fn source_and_sink() {
        let mut port = Port::new();
        port.push_rx(7);
        let mut src = IoSource::new(port);
        assert_eq!(src.read_byte(), Ok(Some(7)));

        let mut sink = IoSink::new(Port::new());
        assert_eq!(sink.write_byte(9), Ok(()));
        assert_eq!(sink.get_ref().tx[0], 9);
    }

    /// Proves the adapter plugs into the same `Pump` the rest of the crate uses.
    #[test]
    fn pump_accepts_io_transport_ends() {
        let mut pump = Pump::on(IoTransport::new(Port::new()), IoTransport::new(Port::new()));
        assert_eq!(pump.poll(), Ok(PumpEvent::Idle));
    }
}

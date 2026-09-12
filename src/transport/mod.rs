//! Transport-layer traits and optional hardware bridges.
//!
//! - Always: [`traits`] — [`ByteTransport`] / [`ByteSource`] / [`ByteSink`].
//! - Feature `embedded-io`: [`embedded_io`] — [`IoTransport`] / [`IoSource`] /
//!   [`IoSink`] so UART/SPI drivers that speak `embedded-io` 0.6 plug into
//!   [`crate::Pump`] without a hand-rolled `ByteTransport`.

pub mod traits;

#[cfg(feature = "embedded-io")]
#[cfg_attr(docsrs, doc(cfg(feature = "embedded-io")))]
pub mod embedded_io;

pub use traits::{ByteSink, ByteSource, ByteTransport};

#[cfg(feature = "embedded-io")]
#[cfg_attr(docsrs, doc(cfg(feature = "embedded-io")))]
pub use embedded_io::{IoSink, IoSource, IoTransport};

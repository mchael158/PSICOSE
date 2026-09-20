//! Transport-layer traits and the hardware demux.
//!
//! - [`traits`] — [`ByteTransport`] / [`ByteSource`] / [`ByteSink`].
//! - [`face`] — [`LinkFace`] demuxes one physical port into Pump TX/RX.
//!
//! Hardware path: implement [`ByteTransport`] on your UART, then
//! `LinkFace::new(port) → Pump / Node`. No third-party I/O crates.

pub mod face;
pub mod traits;

pub use face::{FaceError, FaceRx, FaceTx, LinkFace};
pub use traits::{ByteSink, ByteSource, ByteTransport};

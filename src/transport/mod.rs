//! Transport-layer traits and the hardware demux.
//!
//! - [`traits`] — [`ByteTransport`] / [`ByteSource`] / [`ByteSink`].
//! - [`face`] — [`LinkFace`] demuxes one physical port into Pump TX/RX.
//!
//! Hardware path: implement [`ByteTransport`] on your UART, then
//! `LinkFace::new(port) → Pump / Node`. No third-party I/O crates.

pub mod face;
pub mod half;
pub mod page;
pub mod traits;
pub mod turn;

pub use face::{FaceError, FaceRx, FaceTx, LinkFace};
pub use half::{HalfDuplex, LineDirection, NopDirection};
pub use page::{PageEvent, PageLink};
pub use traits::{ByteSink, ByteSource, ByteTransport};
pub use turn::{Turn, TurnEvent, WindowTurn};

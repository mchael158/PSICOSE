//! The P2P layer — **part of the PSICOSE motor**, not an external stack.
//!
//! ```text
//!        application / examples
//!                 │
//!                 ▼
//!        psicose::Node  →  PeerLink  →  Wire / UART
//! ```
//!
//! Open links through [`Node`]; do not create a separate P2P product and
//! pass connections into this crate.
//!
//! Rules this module never breaks:
//!
//! - **Nothing here enters the 4-byte frame.** [`PeerId`], the hello,
//!   message headers, and fragments are ordinary payload bytes carried
//!   one at a time by DATA frames.
//! - `SEQ` (`u8`, transport) and [`MessageId`] (`u16`, application) are
//!   different counters for different problems. They never mix.
//! - No heap, no `std`, no `unsafe`. Every type is `Copy` or borrows a
//!   caller-owned buffer, with a size known at compile time.

pub mod drive;
pub mod duplex;
pub mod link;
pub mod node;
pub mod peer;
pub mod session;
pub mod stream;
pub mod table;

pub use drive::{establish, send_message};
pub use duplex::{DuplexFull, DuplexPort, DuplexWire, WireCopyError};
pub use link::{LinkError, LinkEvent, PeerLink};
pub use node::Node;
pub use peer::PeerId;
pub use session::{
    Capabilities, HandshakeError, PeerSession, SessionConfig, SessionState, HELLO_LEN,
    PROTOCOL_VERSION,
};
pub use stream::{
    DefragError, Defragmenter, Fragmenter, HeaderError, MessageHeader, MessageId, StreamId,
    HEADER_LEN,
};
pub use table::{PeerEntry, PeerTable, TableError, MAX_PEERS};
/// In-memory duplex harness (`DuplexWire`) for tests and examples.
///
/// **Not** the 4-byte on-wire frame ([`crate::Frame`]). Use
/// [`DuplexWire::copy`], [`DuplexWire::pumps`], or
/// [`DuplexWire::link_pumps`].
pub use DuplexWire as Wire;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_session_stays_a_register_machine() {
        let size = core::mem::size_of::<PeerSession>();
        assert!(size <= 128, "PeerSession is {size} bytes — keep it tiny");
    }
}

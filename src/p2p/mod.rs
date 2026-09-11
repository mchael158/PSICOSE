//! The P2P layer, in the same crate, above the same 4-byte frame.
//!
//! ```text
//!                     APPLICATION
//!                          │
//!               ┌──────────▼──────────┐
//!               │        p2p          │
//!               │ PeerId / Session    │
//!               │ StreamId / Message  │
//!               └──────────┬──────────┘
//!                          │ payload bytes
//!               ┌──────────▼──────────┐
//!               │      transport      │
//!               │  TYPE|SEQ|DATA|CRC  │
//!               └──────────┬──────────┘
//!                          │
//!                    ByteTransport
//! ```
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

pub mod duplex;
pub mod link;
pub mod peer;
pub mod session;
pub mod stream;
pub mod table;

pub use duplex::{DuplexFull, DuplexPort, DuplexWire};
/// In-memory duplex. Same type as [`DuplexWire`].
pub use DuplexWire as Wire;
pub use link::{LinkError, LinkEvent, PeerLink};
pub use peer::PeerId;
pub use session::{
    Capabilities, HandshakeError, PeerSession, SessionConfig, SessionState, HELLO_LEN,
    PROTOCOL_VERSION,
};
pub use stream::{Fragmenter, HeaderError, MessageHeader, MessageId, StreamId, HEADER_LEN};
pub use table::{PeerEntry, PeerTable, TableError, MAX_PEERS};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_session_stays_a_register_machine() {
        let size = core::mem::size_of::<PeerSession>();
        assert!(size <= 128, "PeerSession is {size} bytes — keep it tiny");
    }
}

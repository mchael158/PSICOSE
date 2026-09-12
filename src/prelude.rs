//! The names you write: `use psicose::prelude::*;`
//!
//! ```
//! use psicose::prelude::*;
//!
//! let wire = Wire::new();
//! let (pump_a, pump_b) = wire.pumps();
//!
//! let mut alice = PeerTable::<4>::new(PeerId::from_label(b"alice"));
//! let mut bob = PeerTable::<4>::new(PeerId::from_label(b"bob"));
//!
//! let mut a = match PeerLink::connect(&mut alice, pump_a) {
//!     Ok(link) => link,
//!     Err(_) => return,
//! };
//! let mut b = PeerLink::accept(&bob, pump_b);
//! let _ = (a.poll(&mut alice), b.poll(&mut bob));
//! ```

pub use crate::{
    ByteSink, ByteSource, ByteTransport, Capabilities, DefragError, Defragmenter, DuplexPort,
    DuplexWire, Error, Fragmenter, Frame, IdleBudget, LinkEvent, MessageHeader, MessageId, PeerId,
    PeerLink, PeerSession, PeerTable, PollOutcome, Pump, PumpEvent, Receiver, RetryPolicy, Sender,
    SessionConfig, SessionState, SessionStats, SliceSink, SliceSource, StreamId, TableError,
    TxState, W8Receiver, W8Sender, WindowedReceiver, WindowedSender, Wire, HEADER_LEN, HELLO_LEN,
};

#[cfg(feature = "aead")]
pub use crate::{
    open, open_from, seal, seal_to, sealed_len, AeadError, KEY_LEN, NONCE_LEN, TAG_LEN,
};

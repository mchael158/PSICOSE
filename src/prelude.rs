//! The names you write.
//!
//! ```
//! use psicose::prelude::*;
//!
//! let wire = Wire::new();
//! let (pump_a, pump_b) = wire.pumps();
//!
//! let mut alice = PeerTable::<4>::new(PeerId::from([0xAA; 8]));
//! let mut bob = PeerTable::<4>::new(PeerId::from([0xBB; 8]));
//!
//! let mut a = match PeerLink::connect(&mut alice, pump_a) {
//!     Ok(link) => link,
//!     Err(_) => return,
//! };
//! let mut b = PeerLink::accept(&bob, pump_b);
//! let _ = (a.poll(&mut alice), b.poll(&mut bob));
//! ```

pub use crate::{
    ByteSink, ByteSource, ByteTransport, Capabilities, DuplexPort, DuplexWire, Error, Frame,
    Fragmenter, HEADER_LEN, HELLO_LEN, LinkEvent, MessageHeader, MessageId, PeerId, PeerLink,
    PeerSession, PeerTable, PollOutcome, Pump, PumpEvent, Receiver, RetryPolicy, Sender,
    SessionConfig, SessionState, SliceSink, SliceSource, StreamId, TableError, TxState,
    W8Receiver, W8Sender, WindowedReceiver, WindowedSender, Wire,
};

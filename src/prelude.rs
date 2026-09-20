//! The names you write: `use psicose::prelude::*;`
//!
//! **Only PSICOSE types** — this crate has zero dependencies.
//! Full catalog: `docs/API.md` / `docs/API.pt-BR.md`.
//!
//! ## Included here
//!
//! | Group | Names |
//! | --- | --- |
//! | Motor | [`Node`], [`Wire`], [`LinkFace`], [`establish`], [`send_message`] |
//! | P2P | [`PeerId`], [`PeerLink`], [`PeerTable`], [`PeerSession`], … |
//! | Session | [`SessionConfig`], [`SessionState`], [`Capabilities`] |
//! | Messages | [`Fragmenter`], [`Defragmenter`], [`MessageId`], [`StreamId`], … |
//! | Transport | [`Pump`], [`WindowedPump`], [`ByteTransport`], [`Sender`], … |
//! | Bytes | [`SliceSource`], [`SliceSink`], [`Frame`], [`IdleBudget`], … |
//! | Errors | [`Error`], [`WireCopyError`], [`TableError`], [`LinkEvent`], … |
//!
//! ```
//! use psicose::prelude::*;
//!
//! let wire = Wire::new();
//! let (pump_a, pump_b) = wire.link_pumps();
//!
//! let mut alice = Node::<4>::new(PeerId::from_label(b"alice"));
//! let mut bob = Node::<4>::new(PeerId::from_label(b"bob"));
//!
//! let mut a = match alice.connect(pump_a) {
//!     Ok(link) => link,
//!     Err(_) => return,
//! };
//! let mut b = bob.accept(pump_b);
//! assert!(establish(&mut a, &mut alice, &mut b, &mut bob));
//! ```

pub use crate::{
    establish, send_message, ByteSink, ByteSource, ByteTransport, Capabilities, DefragError,
    Defragmenter, DuplexPort, DuplexWire, Error, FaceError, FaceRx, FaceTx, Fragmenter, Frame,
    IdleBudget, LinkEvent, LinkFace, MessageHeader, MessageId, Node, PeerId, PeerLink, PeerSession,
    PeerTable, PollOutcome, Pump, PumpEvent, Receiver, RetryPolicy, Sender, SessionConfig,
    SessionState, SessionStats, SliceSink, SliceSource, StreamId, TableError, TxState, W8Receiver,
    W8Sender, WindowedPump, WindowedReceiver, WindowedSender, Wire, WireCopyError, HEADER_LEN,
    HELLO_LEN,
};

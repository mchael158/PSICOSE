//! The names you write: `use psicose::prelude::*;`
//!
//! Full catalog (every root export + what it does): `docs/API.md` /
//! `docs/API.pt-BR.md`.
//!
//! ## Included here
//!
//! | Group | Names |
//! | --- | --- |
//! | Motor | [`Node`], [`Wire`], [`establish`], [`send_message`] |
//! | P2P | [`PeerId`], [`PeerLink`], [`PeerTable`], [`PeerSession`], … |
//! | Session | [`SessionConfig`], [`SessionState`], [`Capabilities`] |
//! | Messages | [`Fragmenter`], [`Defragmenter`], [`MessageId`], [`StreamId`], … |
//! | Transport | [`Pump`], [`WindowedPump`], [`ByteTransport`], [`Sender`], … |
//! | Bytes | [`SliceSource`], [`SliceSink`], [`Frame`], [`IdleBudget`], … |
//! | Errors | [`Error`], [`WireCopyError`], [`TableError`], [`LinkEvent`], … |
//!
//! Feature `aead` also reexports seal/open helpers. Feature `embedded-io`
//! reexports [`IoTransport`] / [`IoSource`] / [`IoSink`].
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
    Defragmenter, DuplexPort, DuplexWire, Error, Fragmenter, Frame, IdleBudget, LinkEvent,
    MessageHeader, MessageId, Node, PeerId, PeerLink, PeerSession, PeerTable, PollOutcome, Pump,
    PumpEvent, Receiver, RetryPolicy, Sender, SessionConfig, SessionState, SessionStats, SliceSink,
    SliceSource, StreamId, TableError, TxState, W8Receiver, W8Sender, WindowedPump,
    WindowedReceiver, WindowedSender, Wire, WireCopyError, HEADER_LEN, HELLO_LEN,
};

#[cfg(feature = "aead")]
pub use crate::{
    open, open_from, seal, seal_to, sealed_len, AeadError, KEY_LEN, NONCE_LEN, TAG_LEN,
};

#[cfg(feature = "embedded-io")]
pub use crate::{IoSink, IoSource, IoTransport};

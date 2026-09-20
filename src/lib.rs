//! # PSICOSE-1B
//!
//! A `no_std`, heapless **framework** for reliable byte links: wire protocol,
//! pump, windowing, and **P2P sessions**. You compose the layers; nothing
//! allocates. **Zero crate dependencies** — `use psicose::…` is only PSICOSE.
//!
//! P2P is **not** a separate crate. [`Node`], [`PeerId`], [`PeerTable`],
//! [`PeerLink`], hello negotiation, and fragmentation live in this package
//! on top of the same 4-byte frame. Examples call into this motor — they
//! do not invent an external P2P and pass connections in.
//!
//! ## Framework layers
//!
//! ```text
//! exemplos / application / boards/*
//!        │
//!        ▼
//! psicose::Node                         ← motor entry
//!        ├─ PeerLink / PeerTable / hello
//!        ├─ Fragmenter / Defragmenter
//!        ▼
//! LinkFace / Wire::copy / Pump / WindowedPump
//!        │  DATA 1 byte/frame + CRC-8 + ACK
//!        ▼
//! ByteTransport  (you implement on UART, or Wire harness)
//! ```
//!
//! | Layer | Entry points |
//! | --- | --- |
//! | Wire frame / CRC / SEQ | [`Frame`], [`crc8`] |
//! | Stop-and-wait / window | [`Pump`], [`WindowedPump`], [`Wire::copy`] |
//! | Hardware demux | [`LinkFace`] |
//! | P2P session + peers | [`Node`], [`PeerLink`], [`establish`] |
//!
//! ## Non-negotiable properties
//!
//! - **`#![no_std]`, `#![forbid(unsafe_code)]`, zero dependencies.**
//!   No heap, no `Vec`, no third-party crates in the dependency graph.
//! - **Constant memory regardless of transfer size.**
//! - **The wire frame is fixed at [`protocol::FRAME_LEN`] = 4 bytes:**
//!   `TYPE(1) | SEQ(1) | DATA(1) | CRC(1)`.
//! - **Reliability is the protocol's job.** ACK, NACK, SEQ, CRC-8, retry.
//!
//! Start with [`prelude`]. Map: `docs/API.md`. Hardware: `docs/HARDWARE.md`.
//! Spec: `docs/PROTOCOL.md`.
//!
//! ## What you import (`use psicose::…`)
//!
//! Prefer [`prelude`]. Everything below is a **psicose** type — no foreign
//! crates appear in the public API.
//!
//! | Reach for | When |
//! | --- | --- |
//! | [`Node`] | P2P entry — identity, neighbors, `connect` / `accept` |
//! | [`LinkFace`] | Hardware demux: one UART → Pump TX/RX ends |
//! | [`Wire`] | In-memory A↔B harness (tests). Not the 4-byte frame. |
//! | [`Wire::copy`] | Stop-and-wait copy through the motor |
//! | [`establish`] / [`send_message`] | Cooperative P2P helpers |
//! | [`PeerLink`] | Live link (prefer [`Node`]) |
//! | [`SessionConfig`] | Hello offer — `DEFAULT`, `FORUM`, `SECURE` |
//! | [`Pump`] / [`WindowedPump`] | Reliability over your [`ByteTransport`] |
//! | [`Frame`] / [`crc8`] | Speak the 4-byte envelope yourself |
//! | [`SliceSource`] / [`SliceSink`] | `&[u8]` / `&mut [u8]` as byte ends |
//! | [`ByteTransport`] | **You** implement this for UART/SPI/radio |
//!
//! ## Minimal P2P example
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
//!
//! assert!(establish(&mut a, &mut alice, &mut b, &mut bob));
//! assert_eq!(alice.established(), 1);
//! assert_eq!(bob.established(), 1);
//! ```
//!
//! ## Threat model (short)
//!
//! CRC-8 detects accidental corruption. It does **not** authenticate.
//! Confidentiality/authenticity are **outside** this crate — seal messages
//! in your application before they enter the transport if needed.
#![no_std]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]

pub mod actors;
pub mod error;
pub mod fault;
pub mod p2p;
pub mod prelude;
pub mod protocol;
pub mod pump;
pub mod rx;
pub mod stream;
pub mod timeout;
pub mod transport;
pub mod tx;
pub mod window;

#[doc(inline)]
pub use error::Error;
#[doc(inline)]
pub use p2p::{
    establish, send_message, Capabilities, DefragError, Defragmenter, DuplexFull, DuplexPort,
    DuplexWire, Fragmenter, HandshakeError, HeaderError, LinkError, LinkEvent, MessageHeader,
    MessageId, Node, PeerEntry, PeerId, PeerLink, PeerSession, PeerTable, SessionConfig,
    SessionState, StreamId, TableError, Wire, WireCopyError, HEADER_LEN, HELLO_LEN, MAX_PEERS,
    PROTOCOL_VERSION,
};
#[doc(inline)]
pub use protocol::{
    crc8, Frame, FrameAssembler, FrameError, FrameType, Sequence, CRC8_POLY, FRAME_LEN, PAYLOAD_LEN,
};
#[doc(inline)]
pub use pump::{Pump, PumpEvent, SessionStats, WindowedPump};
#[doc(inline)]
pub use rx::{PollOutcome, Receiver, RxState};
#[doc(inline)]
pub use stream::{
    recv_all, recv_all_budgeted, recv_all_windowed, recv_all_windowed_budgeted, recv_bytes,
    send_all, send_all_windowed, send_all_windowed_budgeted, send_bytes, SliceFull, SliceSink,
    SliceSource, StreamError,
};
#[doc(inline)]
pub use timeout::{IdleBudget, RetryPolicy};
#[doc(inline)]
pub use transport::{ByteSink, ByteSource, ByteTransport, FaceError, FaceRx, FaceTx, LinkFace};
#[doc(inline)]
pub use tx::{Sender, TxPoll, TxState};
#[doc(inline)]
pub use window::{W8Receiver, W8Sender, WindowedReceiver, WindowedSender, MAX_WINDOW};

#[cfg(test)]
mod test_support;

//! # PSICOSE-1B
//!
//! A `no_std`, heapless **framework** for reliable byte links: wire protocol,
//! pump, windowing, **P2P sessions**, plus optional AEAD and `embedded-io`.
//! You compose the layers; nothing allocates.
//!
//! P2P is **not** a separate crate. [`Node`], [`PeerId`], [`PeerTable`],
//! [`PeerLink`], hello negotiation, and fragmentation live in this package
//! on top of the same 4-byte frame. Examples call into this motor — they
//! do not invent an external P2P and pass connections in.
//!
//! ## Framework layers
//!
//! ```text
//! exemplos / application
//!        │
//!        ▼
//! psicose::Node                         ← motor entry
//!        ├─ PeerLink / PeerTable / hello
//!        ├─ Fragmenter / Defragmenter
//!        ├─ optional aead::seal_to / open_from
//!        ▼
//! Wire::copy / Pump / WindowedPump
//!        │  DATA 1 byte/frame + CRC-8 + ACK
//!        ▼
//! ByteTransport  (yours, or IoTransport)
//! ```
//!
//! | Layer | Always in the crate? | Entry points |
//! | --- | --- | --- |
//! | Wire frame / CRC / SEQ | yes | [`Frame`], [`crc8`] |
//! | Stop-and-wait / window | yes | [`Pump`], [`WindowedPump`], [`Wire::copy`] |
//! | P2P session + peers | yes | [`Node`], [`PeerLink`], [`establish`] |
//! | AEAD | feature `aead` | [`seal_to`], [`open_from`] |
//! | `embedded-io` bridge | feature `embedded-io` | [`IoTransport`] |
//!
//! ## Optional Cargo features
//!
//! | Feature | What it adds |
//! | --- | --- |
//! | *(none)* | Full framework except crypto + embedded-io adapters (**zero deps**) |
//! | `aead` | ChaCha20-Poly1305 above the transport |
//! | `embedded-io` | [`IoTransport`] / [`IoSource`] / [`IoSink`] (embedded-io 0.6) |
//!
//! ## Non-negotiable properties
//!
//! - **`#![no_std]`, `#![forbid(unsafe_code)]`.** No heap, no `Vec`, no
//!   `String`, no `Box`, no `Rc`/`Arc`, no async runtime. Every type in
//!   this crate is stack-allocated and has a size known at compile time.
//! - **Constant memory regardless of transfer size.** A 4-byte config blob
//!   and a multi-gigabyte file are transferred through the exact same
//!   `Sender`/`Receiver` (1 in flight) or [`window::WindowedSender`]
//!   (`N ≤ 8` in flight) pair. The protocol never owns the file — only
//!   the window of payload bytes currently on the wire.
//! - **The wire frame is fixed at [`protocol::FRAME_LEN`] = 4 bytes:**
//!   `TYPE(1) | SEQ(1) | DATA(1) | CRC(1)`.
//! - **Reliability is the protocol's job.** ACK, NACK, SEQ, CRC-8, and
//!   tick-based retransmission live in [`tx`] / [`rx`] / [`pump`].
//!
//! Start with [`prelude`]. Full name-by-name map: `docs/API.md`
//! (Português: `docs/API.pt-BR.md`). Wire format: `docs/PROTOCOL.md`.
//!
//! ## What you import (`use psicose::…`)
//!
//! Prefer [`prelude`] for day-to-day work. Almost every type below is also
//! at the crate root.
//!
//! | Reach for | When |
//! | --- | --- |
//! | [`Node`] | P2P entry — identity, neighbors, `connect` / `accept` |
//! | [`Wire`] | In-memory A↔B harness (`DuplexWire`). Not the 4-byte frame. |
//! | [`Wire::copy`] | Stop-and-wait copy source→sink through the motor |
//! | [`establish`] / [`send_message`] | Cooperative P2P helpers (busy-loop) |
//! | [`PeerLink`] | Live link (prefer opening via [`Node`]) |
//! | [`PeerTable`] | Neighbor slots (usually via [`Node::table_mut`]) |
//! | [`SessionConfig`] | Hello offer — presets `DEFAULT`, `FORUM`, `SECURE` |
//! | [`Capabilities`] | Hello bits — `WINDOW`, `STREAM`, `FORUM`, `FRAGMENTATION`, … |
//! | [`Fragmenter`] / [`Defragmenter`] | Cut / rebuild application messages |
//! | [`Pump`] / [`WindowedPump`] | Reliability over your [`ByteTransport`] |
//! | [`Frame`] / [`crc8`] | Speak the 4-byte envelope yourself |
//! | [`SliceSource`] / [`SliceSink`] | `&[u8]` / `&mut [u8]` as byte ends |
//! | [`seal_to`] / [`open_from`] | Feature `aead` — crypto above the transport |
//! | [`IoTransport`] | Feature `embedded-io` — wrap embedded-io ports |
//!
//! Modules for deeper paths: [`p2p`], [`protocol`], [`pump`], [`tx`],
//! [`rx`], [`window`], [`transport`], [`stream`], [`timeout`], [`actors`],
//! [`fault`], and feature-gated [`aead`].
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
//! CRC-8 detects accidental corruption. It does not authenticate. Enable
//! feature `aead` and call [`aead::seal_to`] / [`aead::open_from`] on
//! application messages **before** they enter the transport.
#![no_std]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]

pub mod actors;
#[cfg(feature = "aead")]
#[cfg_attr(docsrs, doc(cfg(feature = "aead")))]
pub mod aead;
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

#[cfg(feature = "aead")]
#[cfg_attr(docsrs, doc(cfg(feature = "aead")))]
#[doc(inline)]
pub use aead::{
    open, open_from, seal, seal_to, sealed_len, AeadError, KEY_LEN, NONCE_LEN, TAG_LEN,
};
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
pub use transport::{ByteSink, ByteSource, ByteTransport};
#[cfg(feature = "embedded-io")]
#[cfg_attr(docsrs, doc(cfg(feature = "embedded-io")))]
#[doc(inline)]
pub use transport::{IoSink, IoSource, IoTransport};
#[doc(inline)]
pub use tx::{Sender, TxPoll, TxState};
#[doc(inline)]
pub use window::{W8Receiver, W8Sender, WindowedReceiver, WindowedSender, MAX_WINDOW};

#[cfg(test)]
mod test_support;

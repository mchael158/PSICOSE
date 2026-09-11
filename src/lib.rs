//! # PSICOSE-1B
//!
//! A `no_std`, heapless, deterministic, byte-oriented transport protocol.
//! The **payload** is exactly one byte; the **frame** is always four.
//!
//! ```text
//!                     APPLICATION
//!                          │
//!                          ▼
//!                  ┌────────────────┐
//!                  │  byte source   │
//!                  └────────┬───────┘
//!                           │ byte by byte
//!                           ▼
//!                 ┌──────────────────┐
//!                 │  PSICOSE-1B      │
//!                 │  tx::Sender      │
//!                 └────────┬─────────┘
//!                          │
//!                   ┌──────┴──────┐
//!                   │             │
//!                  DATA        ACK/NACK
//!                   │             │
//!                   └──────┬──────┘
//!                          ▼
//!                   ByteTransport
//!                          │
//!            ┌─────────────┼─────────────┐
//!            ▼             ▼             ▼
//!          UART           SPI           RADIO
//! ```
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
//!   `TYPE(1) | SEQ(1) | DATA(1) | CRC(1)`. `DATA` is always exactly one
//!   byte, by design — see the module docs on [`protocol::frame`] for why.
//! - **Reliability is the protocol's job, not the application's.** ACK,
//!   NACK, sequence numbers, CRC-8, and tick-based retransmission are all
//!   handled by [`tx::Sender`] / [`rx::Receiver`] so the application only
//!   ever sees clean, in-order bytes.
//!
//! ## Any data, not just frames
//!
//! The 4-byte [`Frame`] is the **envelope**, not the application type.
//! JPEG, a file, flash, a sensor sample, or `struct` bytes all become a
//! [`ByteSource`]. PSICOSE never sees the type — only the next byte.
//! [`stream`] drains a source and rebuilds it on a [`ByteSink`].
//! [`Pump`] interleaves TX and RX without an internal loop.
//!
//! ## P2P above the transport
//!
//! The [`p2p`] module adds identity ([`PeerId`]), sessions ([`PeerSession`]),
//! a compile-time table ([`PeerTable`]), a live [`PeerLink`], and streams
//! ([`StreamId`], [`Fragmenter`]) — all as **payload bytes**. Start from
//! [`prelude`]: `Wire::new().pumps()`.
//!
//! ## Minimal example
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
//!
//! let mut i = 0;
//! while i < 10_000 {
//!     i += 1;
//!     let _ = a.poll(&mut alice);
//!     let _ = b.poll(&mut bob);
//!     if a.session().state() == SessionState::Established
//!         && b.session().state() == SessionState::Established
//!     {
//!         break;
//!     }
//! }
//! assert_eq!(alice.established(), 1);
//! assert_eq!(bob.established(), 1);
//! ```
//!
//! ## What's here vs. what's next
//!
//! This is **0.2.3**: stop-and-wait (1B),
//! selective-repeat [`window`] (`N ≤ 8`), [`stream`], cooperative
//! [`Pump`] / [`SessionStats`], `ABORT` on the same 4-byte frame, and
//! [`p2p`] (`PeerId` / [`PeerSession`] / [`PeerTable`] / [`PeerLink`] /
//! [`StreamId`] / [`Fragmenter`]) as payload bytes only.
//! File and UART/SPI stay out — you implement [`ByteSource`] on top of
//! them. Runnable stand-ins live in `examples/` (`jpeg_over_uart`,
//! `firmware_flash`, `sensor_telemetry`, `radio_windowed`). The formal
//! wire spec ships as `PROTOCOL.md` (English) and `PROTOCOL.pt-BR.md`
//! in the crate.
//!
//! **Payload is 1 byte. The frame is 4 bytes.** `DATA` is the only
//! application bit; `TYPE|SEQ|CRC` are overhead (25% before ACKs, 12.5%
//! stop-and-wait). [`actors`] schedules N receivers on one thread.

#![no_std]
#![forbid(unsafe_code)]
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
    Capabilities, DuplexFull, DuplexPort, DuplexWire, Fragmenter, HandshakeError, HeaderError,
    LinkError, LinkEvent, MessageHeader, MessageId, PeerEntry, PeerId, PeerLink, PeerSession,
    PeerTable, SessionConfig, SessionState, StreamId, TableError, Wire, HEADER_LEN, HELLO_LEN,
    MAX_PEERS, PROTOCOL_VERSION,
};
#[doc(inline)]
pub use protocol::{
    crc8, Frame, FrameAssembler, FrameError, FrameType, Sequence, CRC8_POLY, FRAME_LEN,
    PAYLOAD_LEN,
};
#[doc(inline)]
pub use pump::{Pump, PumpEvent, SessionStats};
#[doc(inline)]
pub use rx::{PollOutcome, Receiver, RxState};
#[doc(inline)]
pub use stream::{
    recv_all, recv_all_windowed, recv_bytes, send_all, send_all_windowed, send_bytes, SliceFull,
    SliceSink, SliceSource, StreamError,
};
#[doc(inline)]
pub use timeout::RetryPolicy;
#[doc(inline)]
pub use transport::{ByteSink, ByteSource, ByteTransport};
#[doc(inline)]
pub use tx::{Sender, TxPoll, TxState};
#[doc(inline)]
pub use window::{WindowedReceiver, WindowedSender, MAX_WINDOW, W8Receiver, W8Sender};

#[cfg(test)]
mod test_support;

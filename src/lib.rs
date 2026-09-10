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
//!
//! ## Minimal example
//!
//! ```
//! use psicose::Frame;
//! let frame = Frame::data(0, 0xAA);
//! assert_eq!(Frame::from_bytes(frame.to_bytes()).unwrap(), frame);
//! ```
//!
//! ## What's here vs. what's next
//!
//! This is **0.2.1**: stop-and-wait (1B) plus selective-repeat
//! [`window`] (`N ≤ 8`, `[Option<T>; N]`, no heap, no `std`), plus
//! [`stream`] (`SliceSource` / `SliceSink` / `send_all` / `recv_all`).
//! File and UART/SPI stay out — you implement [`ByteSource`] on top of
//! them. The formal wire spec ships as `PROTOCOL.md` (English) and
//! `PROTOCOL.pt-BR.md` in the crate.
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
pub mod protocol;
pub mod rx;
pub mod stream;
pub mod timeout;
pub mod transport;
pub mod tx;
pub mod window;

pub use error::Error;
pub use protocol::{
    crc8, Frame, FrameAssembler, FrameError, FrameType, Sequence, CRC8_POLY, FRAME_LEN,
    PAYLOAD_LEN,
};
pub use rx::{PollOutcome, Receiver, RxState};
pub use stream::{
    recv_all, recv_all_windowed, recv_bytes, send_all, send_all_windowed, send_bytes, SliceFull,
    SliceSink, SliceSource, StreamError,
};
pub use timeout::RetryPolicy;
pub use transport::{ByteSink, ByteSource, ByteTransport};
pub use tx::{Sender, TxPoll, TxState};
pub use window::{WindowedReceiver, WindowedSender, MAX_WINDOW, W8Receiver, W8Sender};

#[cfg(test)]
mod test_support;

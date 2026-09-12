//! Application data is **bytes**. The 4-byte frame is only the envelope.
//!
//! ```text
//! JPEG  File  Flash  Sensor  firmware.bin  [u8]
//!   │     │     │       │         │         │
//!   └─────┴─────┴───────┴─────────┴─────────┘
//!                     │
//!                ByteSource          ← you implement this
//!                     │ 1 byte
//!                     ▼
//!                  PSICOSE           ← never owns the file
//!                     │ Frame
//!                     ▼
//!                ByteTransport
//!                     │
//!                ByteSink            ← you implement this
//!                     │
//!              File  Flash  RAM
//! ```
//!
//! A JPEG, a 4 GB file, and a 4-byte config blob use the same path.
//! PSICOSE does not know the type. It only ever holds the current
//! payload byte (or `N` of them, in a window).

use crate::error::Error;
use crate::rx::{PollOutcome, Receiver};
use crate::timeout::IdleBudget;
use crate::transport::{ByteSink, ByteSource, ByteTransport};
use crate::tx::{Sender, TxPoll};
use crate::window::{WindowedReceiver, WindowedSender};

/// Error from a source-to-sink transfer: either the link or the
/// application reader/writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamError<Link, App> {
    /// ACK/NACK/CRC/retry failure on the wire.
    Protocol(Error<Link>),
    /// [`ByteSource`] or [`ByteSink`] failed.
    Application(App),
}

impl<Link: core::fmt::Debug, App: core::fmt::Debug> core::fmt::Display for StreamError<Link, App> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            StreamError::Protocol(e) => write!(f, "protocol: {e}"),
            StreamError::Application(e) => write!(f, "application: {e:?}"),
        }
    }
}

/// The destination slice had no room for the next byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SliceFull;

/// Borrowed byte source. The caller owns the buffer; PSICOSE does not.
pub struct SliceSource<'a> {
    rest: &'a [u8],
}

impl<'a> SliceSource<'a> {
    /// Streams `bytes` from the start.
    pub const fn new(bytes: &'a [u8]) -> Self {
        SliceSource { rest: bytes }
    }

    /// Bytes not yet offered to the sender.
    pub const fn remaining(&self) -> usize {
        self.rest.len()
    }
}

impl ByteSource for SliceSource<'_> {
    type Error = core::convert::Infallible;

    fn read_byte(&mut self) -> Result<Option<u8>, Self::Error> {
        match self.rest.split_first() {
            Some((byte, rest)) => {
                self.rest = rest;
                Ok(Some(*byte))
            }
            None => Ok(None),
        }
    }
}

impl<'a> From<&'a [u8]> for SliceSource<'a> {
    fn from(bytes: &'a [u8]) -> Self {
        SliceSource::new(bytes)
    }
}

/// Borrowed byte sink. The caller owns the buffer; PSICOSE does not.
pub struct SliceSink<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

impl<'a> SliceSink<'a> {
    /// Writes into `buf` from index 0.
    pub fn new(buf: &'a mut [u8]) -> Self {
        SliceSink { buf, pos: 0 }
    }

    /// How many bytes were written.
    pub const fn filled(&self) -> usize {
        self.pos
    }

    /// The prefix that was written.
    pub fn written(&self) -> &[u8] {
        &self.buf[..self.pos]
    }
}

impl ByteSink for SliceSink<'_> {
    type Error = SliceFull;

    fn write_byte(&mut self, byte: u8) -> Result<(), Self::Error> {
        if self.pos >= self.buf.len() {
            return Err(SliceFull);
        }
        self.buf[self.pos] = byte;
        self.pos += 1;
        Ok(())
    }
}

impl<'a> From<&'a mut [u8]> for SliceSink<'a> {
    fn from(buf: &'a mut [u8]) -> Self {
        SliceSink::new(buf)
    }
}

/// Drain `src` through a stop-and-wait sender, then FINISH.
///
/// The source can be a file, flash, camera buffer, or [`SliceSource`].
/// Returns how many payload bytes were accepted by the peer.
///
/// This path expects ACKs on the sender's own transport (scripted
/// replies). On a live pair, use [`crate::Pump::send_all`] so RX is
/// polled in the same cooperative step.
///
/// In-flight waits use [`crate::timeout::RetryPolicy`] inside
/// [`Sender::send_byte`](crate::tx::Sender::send_byte) — not an idle budget.
pub fn send_all<T, S>(
    tx: &mut Sender<T>,
    src: &mut S,
) -> Result<u64, StreamError<T::Error, S::Error>>
where
    T: ByteTransport,
    S: ByteSource,
{
    let mut n = 0u64;
    loop {
        let byte = src.read_byte().map_err(StreamError::Application)?;
        match byte {
            Some(byte) => {
                tx.send_byte(byte).map_err(StreamError::Protocol)?;
                n = n.saturating_add(1);
            }
            None => {
                tx.send_finish().map_err(StreamError::Protocol)?;
                return Ok(n);
            }
        }
    }
}

/// Drain `src` through a windowed sender, then FINISH.
pub fn send_all_windowed<T, S, const N: usize>(
    tx: &mut WindowedSender<T, N>,
    src: &mut S,
) -> Result<u64, StreamError<T::Error, S::Error>>
where
    T: ByteTransport,
    S: ByteSource,
{
    send_all_windowed_budgeted(tx, src, IdleBudget::DEFAULT)
}

/// [`send_all_windowed`] with an [`IdleBudget`] on the `WindowFull` spin.
pub fn send_all_windowed_budgeted<T, S, const N: usize>(
    tx: &mut WindowedSender<T, N>,
    src: &mut S,
    mut budget: IdleBudget,
) -> Result<u64, StreamError<T::Error, S::Error>>
where
    T: ByteTransport,
    S: ByteSource,
{
    let mut n = 0u64;
    loop {
        let byte = src.read_byte().map_err(StreamError::Application)?;
        match byte {
            Some(byte) => loop {
                match tx.offer(byte) {
                    Ok(()) => {
                        n = n.saturating_add(1);
                        budget.reset();
                        break;
                    }
                    Err(Error::WindowFull) => match tx.poll().map_err(StreamError::Protocol)? {
                        TxPoll::Acked => budget.reset(),
                        TxPoll::Pending => budget.tick().map_err(StreamError::Protocol)?,
                        TxPoll::Aborted => {
                            return Err(StreamError::Protocol(Error::Aborted));
                        }
                        TxPoll::SessionReady | TxPoll::TransferDone => {
                            budget.tick().map_err(StreamError::Protocol)?;
                        }
                    },
                    Err(e) => return Err(StreamError::Protocol(e)),
                }
            },
            None => break,
        }
    }
    while tx.outstanding() > 0 {
        match tx.poll().map_err(StreamError::Protocol)? {
            TxPoll::Acked => budget.reset(),
            TxPoll::Pending => budget.tick().map_err(StreamError::Protocol)?,
            TxPoll::Aborted => return Err(StreamError::Protocol(Error::Aborted)),
            TxPoll::SessionReady | TxPoll::TransferDone => break,
        }
    }
    tx.send_finish().map_err(StreamError::Protocol)?;
    Ok(n)
}

/// Write every delivered payload byte into `sink` until FINISH.
pub fn recv_all<T, K>(
    rx: &mut Receiver<T>,
    sink: &mut K,
) -> Result<u64, StreamError<T::Error, K::Error>>
where
    T: ByteTransport,
    K: ByteSink,
{
    recv_all_budgeted(rx, sink, IdleBudget::DEFAULT)
}

/// [`recv_all`] with an [`IdleBudget`] so a silent peer cannot hang the loop.
pub fn recv_all_budgeted<T, K>(
    rx: &mut Receiver<T>,
    sink: &mut K,
    budget: IdleBudget,
) -> Result<u64, StreamError<T::Error, K::Error>>
where
    T: ByteTransport,
    K: ByteSink,
{
    drain_recv(|| rx.poll(), sink, budget)
}

/// Windowed counterpart of [`recv_all`].
pub fn recv_all_windowed<T, K, const N: usize>(
    rx: &mut WindowedReceiver<T, N>,
    sink: &mut K,
) -> Result<u64, StreamError<T::Error, K::Error>>
where
    T: ByteTransport,
    K: ByteSink,
{
    recv_all_windowed_budgeted(rx, sink, IdleBudget::DEFAULT)
}

/// [`recv_all_windowed`] with an [`IdleBudget`].
pub fn recv_all_windowed_budgeted<T, K, const N: usize>(
    rx: &mut WindowedReceiver<T, N>,
    sink: &mut K,
    budget: IdleBudget,
) -> Result<u64, StreamError<T::Error, K::Error>>
where
    T: ByteTransport,
    K: ByteSink,
{
    drain_recv(|| rx.poll(), sink, budget)
}

fn drain_recv<E, K, F>(
    mut poll: F,
    sink: &mut K,
    mut budget: IdleBudget,
) -> Result<u64, StreamError<E, K::Error>>
where
    F: FnMut() -> Result<PollOutcome, Error<E>>,
    K: ByteSink,
{
    let mut n = 0u64;
    loop {
        match poll().map_err(StreamError::Protocol)? {
            PollOutcome::Delivered(byte) => {
                sink.write_byte(byte).map_err(StreamError::Application)?;
                n = n.saturating_add(1);
                budget.reset();
            }
            PollOutcome::TransferFinished => return Ok(n),
            PollOutcome::Aborted => return Err(StreamError::Protocol(Error::Aborted)),
            PollOutcome::Started
            | PollOutcome::DuplicateIgnored
            | PollOutcome::Rejected
            | PollOutcome::CrcRejected => budget.reset(),
            PollOutcome::Pending => budget.tick().map_err(StreamError::Protocol)?,
        }
    }
}

/// Send a borrowed slice. The slice stays with the caller.
pub fn send_bytes<T: ByteTransport>(
    tx: &mut Sender<T>,
    bytes: &[u8],
) -> Result<u64, Error<T::Error>> {
    let mut src = SliceSource::new(bytes);
    match send_all(tx, &mut src) {
        Ok(n) => Ok(n),
        Err(StreamError::Protocol(e)) => Err(e),
        Err(StreamError::Application(never)) => match never {},
    }
}

/// Receive into a borrowed slice until FINISH.
pub fn recv_bytes<'a, T: ByteTransport>(
    rx: &mut Receiver<T>,
    buf: &'a mut [u8],
) -> Result<&'a [u8], StreamError<T::Error, SliceFull>> {
    let mut sink = SliceSink::new(buf);
    recv_all(rx, &mut sink)?;
    let n = sink.filled();
    Ok(&buf[..n])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Frame;
    use crate::test_support::MockTransport;
    use crate::timeout::RetryPolicy;
    use crate::transport::ByteSource;
    use crate::window::{WindowedReceiver, WindowedSender};

    fn ack_run(data_bytes: u8) -> [u8; 36] {
        let data_bytes = core::cmp::min(data_bytes, 8);
        let mut out = [0u8; 36];
        let frames = data_bytes as usize + 1;
        for i in 0..frames {
            let bytes = Frame::ack(i as u8).to_bytes();
            out[i * 4..i * 4 + 4].copy_from_slice(&bytes);
        }
        out
    }

    #[test]
    fn slice_source_is_just_bytes() {
        let jpeg_so_i_like = [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        let mut src = SliceSource::new(&jpeg_so_i_like);
        assert_eq!(src.read_byte(), Ok(Some(0xFF)));
        assert_eq!(src.read_byte(), Ok(Some(0xD8)));
        assert_eq!(src.remaining(), 4);
    }

    #[test]
    fn send_bytes_moves_any_payload_then_finish() {
        let payload = [b'P', b'S', b'I', 0x00];
        let incoming = ack_run(4);
        let mut tx = Sender::with_policy(
            MockTransport::with_incoming(&incoming[..20]),
            RetryPolicy::new(50, 3),
        );
        assert_eq!(send_bytes(&mut tx, &payload), Ok(4));
        assert_eq!(tx.state(), crate::tx::TxState::Finished);
    }

    #[test]
    fn recv_bytes_rebuilds_the_payload() {
        let incoming = crate::test_support::concat3(
            Frame::data(0, 0xFF).to_bytes(),
            Frame::data(1, 0xD8).to_bytes(),
            Frame::finish(2).to_bytes(),
        );
        let mut rx = Receiver::new(MockTransport::with_incoming(&incoming));
        let mut buf = [0u8; 8];
        assert_eq!(recv_bytes(&mut rx, &mut buf), Ok(&[0xFF, 0xD8][..]));
    }

    #[test]
    fn sink_full_is_an_application_error() {
        let incoming = crate::test_support::concat2(
            Frame::data(0, 1).to_bytes(),
            Frame::data(1, 2).to_bytes(),
        );
        let mut rx = Receiver::new(MockTransport::with_incoming(&incoming));
        let mut buf = [0u8; 1];
        assert_eq!(
            recv_bytes(&mut rx, &mut buf),
            Err(StreamError::Application(SliceFull))
        );
    }

    #[test]
    fn send_all_windowed_then_finish() {
        let payload = [0xDE, 0xAD, 0xBE, 0xEF];
        let incoming = ack_run(4);
        let mut tx = WindowedSender::<_, 4>::with_policy(
            MockTransport::with_incoming(&incoming[..20]),
            RetryPolicy::new(50, 3),
        );
        let mut src = SliceSource::new(&payload);
        assert_eq!(send_all_windowed(&mut tx, &mut src), Ok(4));
        assert_eq!(tx.state(), crate::tx::TxState::Finished);
    }

    #[test]
    fn recv_all_windowed_rebuilds_any_blob() {
        let incoming = crate::test_support::concat3(
            Frame::data(0, b'{').to_bytes(),
            Frame::data(1, b'}').to_bytes(),
            Frame::finish(2).to_bytes(),
        );
        let mut rx = WindowedReceiver::<_, 4>::new(MockTransport::with_incoming(&incoming));
        let mut buf = [0u8; 8];
        let mut sink = SliceSink::new(&mut buf);
        assert_eq!(recv_all_windowed(&mut rx, &mut sink), Ok(2));
        assert_eq!(sink.written(), b"{}");
    }

    #[test]
    fn recv_all_budgeted_returns_when_peer_disappears() {
        let mut rx = Receiver::new(MockTransport::with_incoming(&[]));
        let mut buf = [0u8; 4];
        let mut sink = SliceSink::new(&mut buf);
        assert_eq!(
            recv_all_budgeted(&mut rx, &mut sink, IdleBudget::new(8)),
            Err(StreamError::Protocol(Error::IdleBudgetExhausted))
        );
    }
}

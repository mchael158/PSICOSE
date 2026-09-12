//! Heapless selective-repeat receiver: reorder buffer `[Option<u8>; N]`.

use crate::error::Error;
use crate::protocol::{Frame, FrameAssembler, FrameError, FrameType, OutBuf, Sequence};
use crate::rx::{PollOutcome, RxState};
use crate::transport::ByteTransport;

use super::check_window;

#[derive(Clone, Copy)]
enum PendingAction {
    Drain,
    Duplicate,
    Quiet,
    Started,
    Finished,
    Aborted,
    Rejected,
    CrcRejected,
}

/// Selective-repeat receiver with a compile-time window of `N` frames.
///
/// Accepts `SEQ` in `[expected, expected+N)` and re-ACKs
/// `[expected-N, expected)`. Delivers only in order. No heap.
pub struct WindowedReceiver<T: ByteTransport, const N: usize> {
    transport: T,
    expected_seq: Sequence,
    buf: [Option<u8>; N],
    emit: [u8; N],
    emit_at: usize,
    emit_len: usize,
    state: RxState,
    assembler: FrameAssembler,
    out: OutBuf,
    pending: Option<PendingAction>,
    finish_seq: Option<u8>,
    abort_latched: bool,
}

impl<T: ByteTransport, const N: usize> WindowedReceiver<T, N> {
    /// Expects the first DATA at seq `0`.
    pub fn new(transport: T) -> Self {
        check_window::<N>();
        WindowedReceiver {
            transport,
            expected_seq: Sequence::new(),
            buf: [None; N],
            emit: [0; N],
            emit_at: 0,
            emit_len: 0,
            state: RxState::Idle,
            assembler: FrameAssembler::new(),
            out: OutBuf::empty(),
            pending: None,
            finish_seq: None,
            abort_latched: false,
        }
    }

    /// Current coarse state.
    pub fn state(&self) -> RxState {
        self.state
    }

    /// Next in-order DATA sequence (also the SEQ a valid FINISH must carry).
    pub fn expected_seq(&self) -> u8 {
        self.expected_seq.current()
    }

    /// Compile-time window length.
    pub const fn window_size(&self) -> usize {
        N
    }

    fn settle_idle(&mut self) {
        self.state = if self.abort_latched {
            RxState::Aborted
        } else if self.finish_seq.is_some() {
            RxState::Finished
        } else {
            RxState::Idle
        };
    }

    fn is_closed(&self) -> bool {
        self.abort_latched || self.finish_seq.is_some()
    }

    fn clear_window(&mut self) {
        self.buf = [None; N];
        self.emit_at = 0;
        self.emit_len = 0;
    }

    fn offset(&self, seq: u8) -> Option<usize> {
        let delta = seq.wrapping_sub(self.expected_seq.current());
        if (delta as usize) < N {
            Some(delta as usize)
        } else {
            None
        }
    }

    fn is_duplicate(&self, seq: u8) -> bool {
        let back = self.expected_seq.current().wrapping_sub(seq);
        back >= 1 && (back as usize) <= N
    }

    fn shift_buf(&mut self) {
        for i in 0..N - 1 {
            self.buf[i] = self.buf[i + 1];
        }
        if N > 0 {
            self.buf[N - 1] = None;
        }
    }

    fn push_emit(&mut self, byte: u8) {
        if self.emit_len >= N {
            return;
        }
        self.emit[self.emit_len] = byte;
        self.emit_len += 1;
    }

    fn pop_emit(&mut self) -> Option<u8> {
        if self.emit_at >= self.emit_len {
            self.emit_at = 0;
            self.emit_len = 0;
            return None;
        }
        let byte = self.emit[self.emit_at];
        self.emit_at += 1;
        if self.emit_at >= self.emit_len {
            self.emit_at = 0;
            self.emit_len = 0;
        }
        Some(byte)
    }

    fn drain_in_order(&mut self) {
        while let Some(byte) = self.buf[0] {
            self.buf[0] = None;
            self.shift_buf();
            self.expected_seq.advance();
            self.push_emit(byte);
        }
    }

    fn begin_reply(&mut self, frame: &Frame, action: PendingAction) {
        self.out.load(frame);
        self.pending = Some(action);
        self.state = RxState::Acknowledging;
    }

    fn pump_reply(&mut self) -> Result<Option<PollOutcome>, Error<T::Error>> {
        let Some(byte) = self.out.peek() else {
            return Ok(self.take_pending());
        };
        self.transport.write_byte(byte).map_err(Error::Transport)?;
        self.out.commit();
        if self.out.is_idle() {
            Ok(self.take_pending())
        } else {
            self.state = RxState::Acknowledging;
            Ok(None)
        }
    }

    fn take_pending(&mut self) -> Option<PollOutcome> {
        let action = self.pending.take()?;
        match action {
            PendingAction::Drain => {
                self.drain_in_order();
                self.state = RxState::Idle;
                self.pop_emit().map(PollOutcome::Delivered)
            }
            PendingAction::Duplicate => {
                self.settle_idle();
                Some(PollOutcome::DuplicateIgnored)
            }
            PendingAction::Quiet => {
                self.settle_idle();
                Some(PollOutcome::Pending)
            }
            PendingAction::Started => {
                self.finish_seq = None;
                self.abort_latched = false;
                self.state = RxState::Idle;
                Some(PollOutcome::Started)
            }
            PendingAction::Finished => {
                self.state = RxState::Finished;
                Some(PollOutcome::TransferFinished)
            }
            PendingAction::Aborted => {
                self.state = RxState::Aborted;
                Some(PollOutcome::Aborted)
            }
            PendingAction::Rejected => {
                self.settle_idle();
                Some(PollOutcome::Rejected)
            }
            PendingAction::CrcRejected => {
                self.settle_idle();
                Some(PollOutcome::CrcRejected)
            }
        }
    }

    /// Writes at most one reply byte, or reads at most one incoming byte.
    pub fn poll(&mut self) -> Result<PollOutcome, Error<T::Error>> {
        if let Some(byte) = self.pop_emit() {
            return Ok(PollOutcome::Delivered(byte));
        }
        if !self.out.is_idle() {
            return Ok(match self.pump_reply()? {
                Some(outcome) => outcome,
                None => PollOutcome::Pending,
            });
        }

        self.state = RxState::Receiving;
        let Some(byte) = self.transport.read_byte().map_err(Error::Transport)? else {
            self.settle_idle();
            return Ok(PollOutcome::Pending);
        };

        let Some(decode_result) = self.assembler.push(byte) else {
            return Ok(PollOutcome::Pending);
        };

        self.state = RxState::Validating;
        let frame = match decode_result {
            Ok(frame) => frame,
            Err(err) => {
                if self.is_closed() {
                    self.settle_idle();
                    return Ok(PollOutcome::Pending);
                }
                let action = match err {
                    FrameError::CrcMismatch { .. } => PendingAction::CrcRejected,
                    _ => PendingAction::Rejected,
                };
                self.begin_reply(&Frame::nack(self.expected_seq.current()), action);
                return Ok(match self.pump_reply()? {
                    Some(outcome) => outcome,
                    None => PollOutcome::Pending,
                });
            }
        };

        match frame.frame_type() {
            FrameType::Data => self.handle_data(frame)?,
            FrameType::Start => self.handle_start()?,
            FrameType::Finish => self.handle_finish(frame.seq())?,
            FrameType::Abort => self.handle_abort()?,
            FrameType::Ack | FrameType::Nack => {
                self.settle_idle();
                return Ok(PollOutcome::Pending);
            }
        }

        Ok(match self.pump_reply()? {
            Some(outcome) => outcome,
            None => PollOutcome::Pending,
        })
    }

    fn handle_start(&mut self) -> Result<(), Error<T::Error>> {
        self.expected_seq = Sequence::new();
        self.finish_seq = None;
        self.abort_latched = false;
        self.clear_window();
        self.begin_reply(&Frame::ack(0), PendingAction::Started);
        Ok(())
    }

    fn handle_abort(&mut self) -> Result<(), Error<T::Error>> {
        if self.finish_seq.is_some() {
            self.state = RxState::Finished;
            return Ok(());
        }
        self.abort_latched = true;
        self.expected_seq = Sequence::new();
        self.clear_window();
        self.begin_reply(&Frame::ack(0), PendingAction::Aborted);
        Ok(())
    }

    fn handle_finish(&mut self, seq: u8) -> Result<(), Error<T::Error>> {
        if self.abort_latched {
            self.state = RxState::Aborted;
            return Ok(());
        }
        if let Some(closed) = self.finish_seq {
            if seq == closed {
                self.begin_reply(&Frame::ack(seq), PendingAction::Finished);
            } else {
                self.state = RxState::Finished;
            }
            return Ok(());
        }

        if seq != self.expected_seq.current() {
            self.begin_reply(&Frame::nack(seq), PendingAction::Rejected);
            return Ok(());
        }

        self.finish_seq = Some(seq);
        self.clear_window();
        self.begin_reply(&Frame::ack(seq), PendingAction::Finished);
        Ok(())
    }

    fn handle_data(&mut self, frame: Frame) -> Result<(), Error<T::Error>> {
        if self.abort_latched {
            self.state = RxState::Aborted;
            return Ok(());
        }
        if self.finish_seq.is_some() {
            self.state = RxState::Finished;
            return Ok(());
        }

        let seq = frame.seq();

        if let Some(off) = self.offset(seq) {
            if self.buf[off].is_none() {
                self.buf[off] = Some(frame.payload());
            }
            let action = if off == 0 {
                PendingAction::Drain
            } else {
                PendingAction::Quiet
            };
            self.begin_reply(&Frame::ack(seq), action);
            return Ok(());
        }

        if self.is_duplicate(seq) {
            self.begin_reply(&Frame::ack(seq), PendingAction::Duplicate);
            return Ok(());
        }

        self.begin_reply(&Frame::nack(seq), PendingAction::Rejected);
        Ok(())
    }

    /// Unwraps the transport.
    pub fn release(self) -> T {
        self.transport
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Frame;
    use crate::test_support::MockTransport;

    fn poll_until_settled<T: ByteTransport, const N: usize>(
        rx: &mut WindowedReceiver<T, N>,
    ) -> Result<PollOutcome, Error<T::Error>> {
        loop {
            match rx.poll()? {
                PollOutcome::Pending => continue,
                other => return Ok(other),
            }
        }
    }

    #[test]
    fn in_order_byte_is_acked_and_delivered() {
        let mut rx = WindowedReceiver::<_, 4>::new(MockTransport::with_incoming(
            &Frame::data(0, 0x42).to_bytes(),
        ));
        assert_eq!(
            poll_until_settled(&mut rx),
            Ok(PollOutcome::Delivered(0x42))
        );
        assert_eq!(rx.expected_seq(), 1);
        assert_eq!(rx.transport.written(), Frame::ack(0).to_bytes());
    }

    #[test]
    fn out_of_order_is_buffered_then_delivered_in_order() {
        let incoming = crate::test_support::concat2(
            Frame::data(1, 0xB1).to_bytes(),
            Frame::data(0, 0xB0).to_bytes(),
        );
        let mut rx = WindowedReceiver::<_, 4>::new(MockTransport::with_incoming(&incoming));

        assert_eq!(
            poll_until_settled(&mut rx),
            Ok(PollOutcome::Delivered(0xB0))
        );
        assert_eq!(rx.poll(), Ok(PollOutcome::Delivered(0xB1)));
        assert_eq!(rx.expected_seq(), 2);
        assert_eq!(
            rx.transport.written(),
            crate::test_support::concat2(Frame::ack(1).to_bytes(), Frame::ack(0).to_bytes())
        );
    }

    #[test]
    fn duplicate_inside_the_left_window_is_reacked() {
        let incoming = crate::test_support::concat2(
            Frame::data(0, 0xAA).to_bytes(),
            Frame::data(0, 0xAA).to_bytes(),
        );
        let mut rx = WindowedReceiver::<_, 4>::new(MockTransport::with_incoming(&incoming));
        assert_eq!(
            poll_until_settled(&mut rx),
            Ok(PollOutcome::Delivered(0xAA))
        );
        assert_eq!(
            poll_until_settled(&mut rx),
            Ok(PollOutcome::DuplicateIgnored)
        );
        assert_eq!(rx.expected_seq(), 1);
    }

    #[test]
    fn seq_outside_window_is_rejected() {
        let mut rx = WindowedReceiver::<_, 4>::new(MockTransport::with_incoming(
            &Frame::data(5, 0xFF).to_bytes(),
        ));
        assert_eq!(poll_until_settled(&mut rx), Ok(PollOutcome::Rejected));
        assert_eq!(rx.expected_seq(), 0);
        assert_eq!(rx.transport.written(), Frame::nack(5).to_bytes());
    }

    #[test]
    fn finish_with_a_hole_is_rejected() {
        let incoming = crate::test_support::concat2(
            Frame::data(1, 0x01).to_bytes(),
            Frame::finish(2).to_bytes(),
        );
        let mut rx = WindowedReceiver::<_, 4>::new(MockTransport::with_incoming(&incoming));
        assert_eq!(poll_until_settled(&mut rx), Ok(PollOutcome::Rejected));
        assert_eq!(rx.expected_seq(), 0);
        assert_ne!(rx.state(), RxState::Finished);
    }

    #[test]
    fn wraparound_window_accepts_255_then_0() {
        let incoming = crate::test_support::concat2(
            Frame::data(255, 0xFE).to_bytes(),
            Frame::data(0, 0x00).to_bytes(),
        );
        let mut rx = WindowedReceiver::<_, 4>::new(MockTransport::with_incoming(&incoming));
        // Force expected to 255 without exposing a setter on the windowed type.
        rx.expected_seq = Sequence::from_raw(255);
        assert_eq!(
            poll_until_settled(&mut rx),
            Ok(PollOutcome::Delivered(0xFE))
        );
        assert_eq!(rx.expected_seq(), 0);
        assert_eq!(
            poll_until_settled(&mut rx),
            Ok(PollOutcome::Delivered(0x00))
        );
        assert_eq!(rx.expected_seq(), 1);
    }
}

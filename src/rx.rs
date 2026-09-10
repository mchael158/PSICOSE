//! The receive side: validates incoming DATA frames, ACKs the in-order
//! ones, NACKs the corrupted ones, and transparently re-ACKs duplicate
//! retransmissions without re-delivering the byte to the application.
//!
//! Control replies are written one byte per poll. The application sees
//! [`PollOutcome::Delivered`] only after the matching ACK is fully on
//! the wire — so a failed write cannot deliver a byte the peer never
//! learned about.

use crate::error::Error;
use crate::protocol::{Frame, FrameAssembler, FrameType, OutBuf, Sequence};
use crate::transport::ByteTransport;

/// The receiver's current state. Exposed via [`Receiver::state`] for
/// diagnostics/logging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RxState {
    /// No frame currently being assembled.
    Idle,
    /// Bytes are being accumulated into a candidate frame.
    Receiving,
    /// A complete frame has been assembled and is being checked.
    Validating,
    /// An ACK or NACK is being written back, one byte per poll.
    Acknowledging,
    /// A FINISH frame has been accepted; the transfer is complete.
    Finished,
}

/// A stop-and-wait PSICOSE-1B receiver, symmetric to [`Sender`](crate::tx::Sender).
pub struct Receiver<T: ByteTransport> {
    transport: T,
    expected_seq: Sequence,
    state: RxState,
    assembler: FrameAssembler,
    out: OutBuf,
    pending: Option<PendingAction>,
    /// Set when FINISH(expected) was accepted. Survives assembler noise.
    finish_seq: Option<u8>,
}

#[derive(Clone, Copy)]
enum PendingAction {
    Deliver(u8),
    Duplicate,
    Started,
    Finished,
    Rejected,
}

/// What happened as a result of one [`Receiver::poll`] call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PollOutcome {
    /// No complete frame was available yet; call [`Receiver::poll`] again.
    Pending,
    /// A new, in-order data byte was received and ACKed. This is the only
    /// variant that represents application-visible data.
    Delivered(u8),
    /// A retransmitted duplicate of the last-accepted frame was received
    /// and re-ACKed, but not re-delivered to the application.
    DuplicateIgnored,
    /// START was accepted; `expected_seq` is 0 and the session was ACKed.
    Started,
    /// FINISH was accepted and ACKed. The transfer is closed.
    TransferFinished,
    /// A frame was rejected (CRC/type failure, or unexpected `SEQ`). A
    /// NACK was sent. This is a wire event, not an application failure:
    /// keep polling.
    Rejected,
}

impl<T: ByteTransport> Receiver<T> {
    /// Creates a new receiver, expecting the first DATA frame at seq `0`.
    pub fn new(transport: T) -> Self {
        Receiver {
            transport,
            expected_seq: Sequence::new(),
            state: RxState::Idle,
            assembler: FrameAssembler::new(),
            out: OutBuf::empty(),
            pending: None,
            finish_seq: None,
        }
    }

    /// The receiver's current state.
    pub fn state(&self) -> RxState {
        self.state
    }

    /// The sequence number the receiver expects the *next* DATA frame to
    /// carry. Also the SEQ a valid FINISH must carry.
    pub fn expected_seq(&self) -> u8 {
        self.expected_seq.current()
    }

    #[cfg(test)]
    pub(crate) fn set_expected_seq(&mut self, seq: u8) {
        self.expected_seq = Sequence::from_raw(seq);
    }

    fn settle_idle(&mut self) {
        self.state = if self.finish_seq.is_some() {
            RxState::Finished
        } else {
            RxState::Idle
        };
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
            PendingAction::Deliver(byte) => {
                self.expected_seq.advance();
                self.state = RxState::Idle;
                Some(PollOutcome::Delivered(byte))
            }
            PendingAction::Duplicate => {
                self.settle_idle();
                Some(PollOutcome::DuplicateIgnored)
            }
            PendingAction::Started => {
                self.finish_seq = None;
                self.state = RxState::Idle;
                Some(PollOutcome::Started)
            }
            PendingAction::Finished => {
                self.state = RxState::Finished;
                Some(PollOutcome::TransferFinished)
            }
            PendingAction::Rejected => {
                self.settle_idle();
                Some(PollOutcome::Rejected)
            }
        }
    }

    /// Polls the transport once: writes at most one reply byte, or reads
    /// at most one incoming byte. Non-blocking.
    ///
    /// Callers loop on this, e.g.:
    ///
    /// ```ignore
    /// loop {
    ///     match receiver.poll()? {
    ///         PollOutcome::Delivered(byte) => sink.write_byte(byte)?,
    ///         PollOutcome::TransferFinished => break,
    ///         PollOutcome::Started | PollOutcome::Pending | PollOutcome::DuplicateIgnored | PollOutcome::Rejected => {}
    ///     }
    /// }
    /// ```
    pub fn poll(&mut self) -> Result<PollOutcome, Error<T::Error>> {
        if !self.out.is_idle() {
            return Ok(self.pump_reply()?.unwrap_or(PollOutcome::Pending));
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
            Err(_) => {
                if self.finish_seq.is_some() {
                    self.state = RxState::Finished;
                    return Ok(PollOutcome::Pending);
                }
                self.begin_reply(&Frame::nack(self.expected_seq.current()), PendingAction::Rejected);
                return Ok(self.pump_reply()?.unwrap_or(PollOutcome::Pending));
            }
        };

        match frame.frame_type() {
            FrameType::Data => self.handle_data(frame)?,
            FrameType::Start => self.handle_start()?,
            FrameType::Finish => self.handle_finish(frame.seq())?,
            FrameType::Ack | FrameType::Nack => {
                self.settle_idle();
                return Ok(PollOutcome::Pending);
            }
        }

        Ok(self.pump_reply()?.unwrap_or(PollOutcome::Pending))
    }

    fn handle_start(&mut self) -> Result<(), Error<T::Error>> {
        self.expected_seq = Sequence::new();
        self.finish_seq = None;
        self.begin_reply(&Frame::ack(0), PendingAction::Started);
        Ok(())
    }

    fn handle_finish(&mut self, seq: u8) -> Result<(), Error<T::Error>> {
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
        self.begin_reply(&Frame::ack(seq), PendingAction::Finished);
        Ok(())
    }

    fn handle_data(&mut self, frame: Frame) -> Result<(), Error<T::Error>> {
        if self.finish_seq.is_some() {
            self.state = RxState::Finished;
            return Ok(());
        }

        let expected = self.expected_seq.current();

        if frame.seq() == expected {
            self.begin_reply(&Frame::ack(expected), PendingAction::Deliver(frame.payload()));
            return Ok(());
        }

        if frame.seq() == self.expected_seq.previous() {
            self.begin_reply(&Frame::ack(frame.seq()), PendingAction::Duplicate);
            return Ok(());
        }

        self.begin_reply(&Frame::nack(frame.seq()), PendingAction::Rejected);
        Ok(())
    }

    /// Consumes the receiver, returning the underlying transport.
    pub fn release(self) -> T {
        self.transport
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::MockTransport;

    fn poll_until_settled<T: ByteTransport>(
        rx: &mut Receiver<T>,
    ) -> Result<PollOutcome, Error<T::Error>> {
        loop {
            match rx.poll()? {
                PollOutcome::Pending => continue,
                other => return Ok(other),
            }
        }
    }

    #[test]
    fn delivers_in_order_byte_and_acks_it() {
        let incoming = Frame::data_frame(0, 0x42).to_bytes();
        let transport = MockTransport::with_incoming(&incoming);
        let mut rx = Receiver::new(transport);

        let outcome = poll_until_settled(&mut rx).unwrap();
        assert_eq!(outcome, PollOutcome::Delivered(0x42));
        assert_eq!(rx.expected_seq(), 1);
        assert_eq!(rx.transport.written(), Frame::ack(0).to_bytes());
    }

    #[test]
    fn delivered_only_after_ack_is_fully_written() {
        let incoming = Frame::data(0, 0x42).to_bytes();
        let mut rx = Receiver::new(MockTransport::with_incoming(&incoming));

        for _ in 0..3 {
            assert_eq!(rx.poll().unwrap(), PollOutcome::Pending);
            assert!(rx.transport.written().is_empty());
        }
        assert_eq!(rx.poll().unwrap(), PollOutcome::Pending);
        assert_eq!(rx.transport.written().len(), 1);
        assert_eq!(rx.expected_seq(), 0, "must not advance before ACK is out");
        assert_eq!(rx.poll().unwrap(), PollOutcome::Pending);
        assert_eq!(rx.poll().unwrap(), PollOutcome::Pending);
        assert_eq!(rx.poll().unwrap(), PollOutcome::Delivered(0x42));
        assert_eq!(rx.expected_seq(), 1);
        assert_eq!(rx.transport.written(), Frame::ack(0).to_bytes());
    }

    #[test]
    fn pending_when_frame_incomplete() {
        let bytes = Frame::data_frame(0, 1).to_bytes();
        let transport = MockTransport::with_incoming(&bytes[..2]);
        let mut rx = Receiver::new(transport);

        assert_eq!(rx.poll().unwrap(), PollOutcome::Pending);
        assert_eq!(rx.poll().unwrap(), PollOutcome::Pending);
        assert_eq!(rx.poll().unwrap(), PollOutcome::Pending);
    }

    #[test]
    fn duplicate_retransmission_is_reacked_not_redelivered() {
        let incoming = crate::test_support::concat2(
            Frame::data_frame(0, 0xAA).to_bytes(),
            Frame::data_frame(0, 0xAA).to_bytes(),
        );
        let transport = MockTransport::with_incoming(&incoming);
        let mut rx = Receiver::new(transport);

        assert_eq!(
            poll_until_settled(&mut rx).unwrap(),
            PollOutcome::Delivered(0xAA)
        );
        assert_eq!(
            poll_until_settled(&mut rx).unwrap(),
            PollOutcome::DuplicateIgnored
        );
        assert_eq!(rx.expected_seq(), 1);
    }

    #[test]
    fn corrupted_frame_triggers_nack_and_propagates_error() {
        let mut bytes = Frame::data_frame(0, 1).to_bytes();
        bytes[2] ^= 0xFF;
        let transport = MockTransport::with_incoming(&bytes);
        let mut rx = Receiver::new(transport);

        assert_eq!(poll_until_settled(&mut rx).unwrap(), PollOutcome::Rejected);
        assert_eq!(rx.expected_seq(), 0);
        assert_eq!(rx.transport.written(), Frame::nack(0).to_bytes());
    }

    #[test]
    fn start_resets_expected_seq_and_acks() {
        let incoming = crate::test_support::concat3(
            Frame::data(0, 0x10).to_bytes(),
            Frame::data(1, 0x11).to_bytes(),
            Frame::start().to_bytes(),
        );
        let transport = MockTransport::with_incoming(&incoming);
        let mut rx = Receiver::new(transport);

        assert_eq!(poll_until_settled(&mut rx).unwrap(), PollOutcome::Delivered(0x10));
        assert_eq!(poll_until_settled(&mut rx).unwrap(), PollOutcome::Delivered(0x11));
        assert_eq!(rx.expected_seq(), 2);
        assert_eq!(poll_until_settled(&mut rx).unwrap(), PollOutcome::Started);
        assert_eq!(rx.expected_seq(), 0);
        assert_eq!(rx.state(), RxState::Idle);
    }

    #[test]
    fn finish_is_accepted_only_at_expected_seq() {
        let incoming = crate::test_support::concat2(
            Frame::data(0, 0x10).to_bytes(),
            Frame::finish(0).to_bytes(),
        );
        let mut rx = Receiver::new(MockTransport::with_incoming(&incoming));

        assert_eq!(poll_until_settled(&mut rx).unwrap(), PollOutcome::Delivered(0x10));
        assert_eq!(rx.expected_seq(), 1);
        assert_eq!(poll_until_settled(&mut rx).unwrap(), PollOutcome::Rejected);
        assert_eq!(rx.state(), RxState::Idle);
        assert_eq!(rx.expected_seq(), 1);
    }

    #[test]
    fn finish_at_expected_seq_is_acked_then_closed() {
        let transport = MockTransport::with_incoming(&Frame::finish(0).to_bytes());
        let mut rx = Receiver::new(transport);

        assert_eq!(
            poll_until_settled(&mut rx).unwrap(),
            PollOutcome::TransferFinished
        );
        assert_eq!(rx.state(), RxState::Finished);
        assert_eq!(rx.transport.written(), Frame::ack(0).to_bytes());
    }

    #[test]
    fn duplicate_finish_is_reacked_not_an_error() {
        let incoming = crate::test_support::concat2(
            Frame::finish(0).to_bytes(),
            Frame::finish(0).to_bytes(),
        );
        let transport = MockTransport::with_incoming(&incoming);
        let mut rx = Receiver::new(transport);

        assert_eq!(
            poll_until_settled(&mut rx).unwrap(),
            PollOutcome::TransferFinished
        );
        assert_eq!(
            poll_until_settled(&mut rx).unwrap(),
            PollOutcome::TransferFinished
        );
        assert_eq!(
            rx.transport.written(),
            crate::test_support::concat2(Frame::ack(0).to_bytes(), Frame::ack(0).to_bytes())
        );
        assert_eq!(rx.state(), RxState::Finished);
    }

    #[test]
    fn data_after_finish_is_not_delivered() {
        let incoming = crate::test_support::concat2(
            Frame::finish(0).to_bytes(),
            Frame::data(0, 0x99).to_bytes(),
        );
        let mut rx = Receiver::new(MockTransport::with_incoming(&incoming));

        assert_eq!(
            poll_until_settled(&mut rx).unwrap(),
            PollOutcome::TransferFinished
        );
        for _ in 0..8 {
            assert_eq!(rx.poll().unwrap(), PollOutcome::Pending);
        }
        assert_eq!(rx.state(), RxState::Finished);
        assert_eq!(rx.transport.written(), Frame::ack(0).to_bytes());
    }

    #[test]
    fn corrupt_after_finish_does_not_nack_or_leave_finished() {
        let finish = Frame::finish(0).to_bytes();
        let mut bad = Frame::data(0, 1).to_bytes();
        bad[3] ^= 0xFF;
        let incoming = crate::test_support::concat2(finish, bad);
        let mut rx = Receiver::new(MockTransport::with_incoming(&incoming));

        assert_eq!(
            poll_until_settled(&mut rx).unwrap(),
            PollOutcome::TransferFinished
        );
        for _ in 0..8 {
            assert_eq!(rx.poll().unwrap(), PollOutcome::Pending);
        }
        assert_eq!(rx.state(), RxState::Finished);
        assert_eq!(rx.transport.written(), Frame::ack(0).to_bytes());
    }

    #[test]
    fn start_after_finish_reopens_the_session() {
        let incoming = crate::test_support::concat3(
            Frame::finish(0).to_bytes(),
            Frame::start().to_bytes(),
            Frame::data(0, 0xAB).to_bytes(),
        );
        let mut rx = Receiver::new(MockTransport::with_incoming(&incoming));

        assert_eq!(
            poll_until_settled(&mut rx).unwrap(),
            PollOutcome::TransferFinished
        );
        assert_eq!(poll_until_settled(&mut rx).unwrap(), PollOutcome::Started);
        assert_eq!(rx.expected_seq(), 0);
        assert_eq!(
            poll_until_settled(&mut rx).unwrap(),
            PollOutcome::Delivered(0xAB)
        );
        assert_eq!(rx.expected_seq(), 1);
    }

    #[test]
    fn reordered_data_is_rejected_and_not_delivered() {
        let incoming = crate::test_support::concat3(
            Frame::data_frame(0, 0x10).to_bytes(),
            Frame::data_frame(2, 0x12).to_bytes(),
            Frame::data_frame(1, 0x11).to_bytes(),
        );
        let transport = MockTransport::with_incoming(&incoming);
        let mut rx = Receiver::new(transport);

        assert_eq!(poll_until_settled(&mut rx).unwrap(), PollOutcome::Delivered(0x10));
        assert_eq!(poll_until_settled(&mut rx).unwrap(), PollOutcome::Rejected);
        assert_eq!(rx.expected_seq(), 1);
        assert_eq!(poll_until_settled(&mut rx).unwrap(), PollOutcome::Delivered(0x11));
        assert_eq!(rx.expected_seq(), 2);
    }

    #[test]
    fn wraparound_duplicate_of_255_is_not_redelivered() {
        let incoming = crate::test_support::concat2(
            Frame::data_frame(255, 0xFE).to_bytes(),
            Frame::data_frame(0, 0x00).to_bytes(),
        );
        let transport = MockTransport::with_incoming(&incoming);
        let mut rx = Receiver::new(transport);
        rx.set_expected_seq(0);

        assert_eq!(
            poll_until_settled(&mut rx).unwrap(),
            PollOutcome::DuplicateIgnored
        );
        assert_eq!(rx.expected_seq(), 0);
        assert_eq!(
            poll_until_settled(&mut rx).unwrap(),
            PollOutcome::Delivered(0x00)
        );
        assert_eq!(rx.expected_seq(), 1);
    }

    #[test]
    fn wraparound_255_then_0_delivers_both() {
        let incoming = crate::test_support::concat2(
            Frame::data_frame(255, 0xFE).to_bytes(),
            Frame::data_frame(0, 0x00).to_bytes(),
        );
        let transport = MockTransport::with_incoming(&incoming);
        let mut rx = Receiver::new(transport);
        rx.set_expected_seq(255);

        assert_eq!(
            poll_until_settled(&mut rx).unwrap(),
            PollOutcome::Delivered(0xFE)
        );
        assert_eq!(rx.expected_seq(), 0);
        assert_eq!(
            poll_until_settled(&mut rx).unwrap(),
            PollOutcome::Delivered(0x00)
        );
        assert_eq!(rx.expected_seq(), 1);
    }
}

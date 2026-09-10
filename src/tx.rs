//! Stop-and-wait sender.
//!
//! ```text
//!                 ┌─────────┐
//!                 │  IDLE   │
//!                 └────┬────┘
//!                      │ offer / offer_start / offer_finish
//!                      ▼
//!                 ┌─────────┐
//!                 │ SENDING │  ← one wire byte per poll
//!                 └────┬────┘
//!                      ▼
//!                ┌───────────┐
//!                │ WAIT_ACK  │
//!                └─────┬─────┘
//!                      │
//!             ┌────────┼────────┐
//!             │        │        │
//!         ACK(cur)  NACK(cur)  TIMEOUT
//!             │        │        │
//!             ▼        └────┬───┘
//!           IDLE            │
//!      (or Finished)        ▼
//!                       RETRYING
//!                           │
//!                           ▼
//!                       SENDING
//! ```
//!
//! ACK/NACK for any *other* sequence, and an invalid frame, are ignored
//! until the tick timeout fires. A frame is not "on the wire" until all
//! four bytes have been written.

use crate::error::Error;
use crate::protocol::{Frame, FrameAssembler, FrameType, OutBuf, Sequence};
use crate::timeout::RetryPolicy;
use crate::transport::ByteTransport;

/// Transmitter state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxState {
    /// Ready for the next offer.
    Idle,
    /// Writing a frame, one byte per poll.
    Sending,
    /// Frame on the wire; waiting for ACK/NACK/timeout.
    WaitingAck,
    /// About to retransmit.
    Retrying,
    /// FINISH was ACKed.
    Finished,
    /// Retry budget exhausted.
    Failed,
}

/// Outcome of one [`Sender::poll`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxPoll {
    /// Still working.
    Pending,
    /// A DATA byte was ACKed; sequence advanced.
    Acked,
    /// START was ACKed; receiver has reset to seq 0.
    SessionReady,
    /// FINISH was ACKed; transfer is closed.
    TransferDone,
}

#[derive(Clone, Copy)]
enum FlightKind {
    Data(u8),
    Start,
    Finish,
}

struct InFlight {
    kind: FlightKind,
    seq: u8,
    retries_left: u8,
    waiting: bool,
    empty_ticks: u16,
}

/// A stop-and-wait PSICOSE-1B sender.
pub struct Sender<T: ByteTransport> {
    transport: T,
    seq: Sequence,
    state: TxState,
    retry_policy: RetryPolicy,
    assembler: FrameAssembler,
    inflight: Option<InFlight>,
    out: OutBuf,
}

impl<T: ByteTransport> Sender<T> {
    /// Default [`RetryPolicy`].
    pub fn new(transport: T) -> Self {
        Self::with_policy(transport, RetryPolicy::default())
    }

    /// Explicit [`RetryPolicy`].
    pub fn with_policy(transport: T, retry_policy: RetryPolicy) -> Self {
        Sender {
            transport,
            seq: Sequence::new(),
            state: TxState::Idle,
            retry_policy,
            assembler: FrameAssembler::new(),
            inflight: None,
            out: OutBuf::empty(),
        }
    }

    /// Current state.
    pub fn state(&self) -> TxState {
        self.state
    }

    /// Sequence that the next DATA byte will carry.
    pub fn next_seq(&self) -> u8 {
        self.seq.current()
    }

    fn can_offer_data(&self) -> bool {
        self.state == TxState::Idle && self.inflight.is_none() && self.out.is_idle()
    }

    fn begin(&mut self, kind: FlightKind, seq: u8) {
        self.inflight = Some(InFlight {
            kind,
            seq,
            retries_left: self.retry_policy.max_retries,
            waiting: false,
            empty_ticks: 0,
        });
        self.state = TxState::Sending;
    }

    /// Opens a session. Allowed from Idle, Finished, or Failed (recovery).
    /// The receiver resets `expected_seq` to 0 and ACKs.
    pub fn offer_start(&mut self) -> Result<(), Error<T::Error>> {
        match self.state {
            TxState::Idle | TxState::Finished | TxState::Failed => {}
            _ => return Err(Error::NotIdle),
        }
        if !self.out.is_idle() {
            return Err(Error::NotIdle);
        }
        self.inflight = None;
        self.seq = Sequence::new();
        self.begin(FlightKind::Start, 0);
        Ok(())
    }

    /// Begins a stop-and-wait send of one payload byte. Must be Idle.
    pub fn offer(&mut self, data: u8) -> Result<(), Error<T::Error>> {
        if !self.can_offer_data() {
            return Err(Error::NotIdle);
        }
        self.begin(FlightKind::Data(data), self.seq.current());
        Ok(())
    }

    /// Begins FINISH at the current (next unused) sequence. Must be Idle.
    pub fn offer_finish(&mut self) -> Result<(), Error<T::Error>> {
        if !self.can_offer_data() {
            return Err(Error::NotIdle);
        }
        self.begin(FlightKind::Finish, self.seq.current());
        Ok(())
    }

    fn pump_out(&mut self) -> Result<bool, Error<T::Error>> {
        let Some(byte) = self.out.peek() else {
            return Ok(true);
        };
        self.transport.write_byte(byte).map_err(Error::Transport)?;
        self.out.commit();
        Ok(self.out.is_idle())
    }

    fn load_flight_frame(&mut self) {
        let Some(flight) = self.inflight.as_ref() else {
            return;
        };
        let frame = match flight.kind {
            FlightKind::Data(byte) => Frame::data(flight.seq, byte),
            FlightKind::Start => Frame::start(),
            FlightKind::Finish => Frame::finish(flight.seq),
        };
        self.out.load(&frame);
        self.state = TxState::Sending;
    }

    /// One non-blocking step: write at most one byte, or read at most one.
    pub fn poll(&mut self) -> Result<TxPoll, Error<T::Error>> {
        if self.state == TxState::Failed {
            return Err(Error::RetriesExhausted);
        }
        if self.state == TxState::Finished && self.inflight.is_none() {
            return Ok(TxPoll::TransferDone);
        }

        if !self.out.is_idle() {
            let done = self.pump_out()?;
            if !done {
                self.state = TxState::Sending;
                return Ok(TxPoll::Pending);
            }
            if let Some(flight) = self.inflight.as_mut() {
                flight.waiting = true;
                flight.empty_ticks = 0;
            }
            self.assembler.reset();
            self.state = TxState::WaitingAck;
            return Ok(TxPoll::Pending);
        }

        let Some(flight) = self.inflight.as_ref() else {
            return Ok(TxPoll::Pending);
        };

        if !flight.waiting {
            self.load_flight_frame();
            let done = self.pump_out()?;
            if !done {
                return Ok(TxPoll::Pending);
            }
            if let Some(flight) = self.inflight.as_mut() {
                flight.waiting = true;
                flight.empty_ticks = 0;
            }
            self.assembler.reset();
            self.state = TxState::WaitingAck;
            return Ok(TxPoll::Pending);
        }

        match self.transport.read_byte().map_err(Error::Transport)? {
            Some(byte) => {
                let Some(result) = self.assembler.push(byte) else {
                    return Ok(TxPoll::Pending);
                };
                let frame = match result {
                    Ok(frame) => frame,
                    Err(_) => return Ok(TxPoll::Pending),
                };
                self.on_control(frame)
            }
            None => {
                let timeout = self.retry_policy.timeout_ticks;
                let flight = self.inflight.as_mut().unwrap();
                flight.empty_ticks = flight.empty_ticks.saturating_add(1);
                if flight.empty_ticks >= timeout {
                    self.on_attempt_failed()
                } else {
                    Ok(TxPoll::Pending)
                }
            }
        }
    }

    fn on_control(&mut self, frame: Frame) -> Result<TxPoll, Error<T::Error>> {
        let seq = self.inflight.as_ref().map(|f| f.seq).unwrap_or(0);
        match (frame.frame_type(), frame.seq()) {
            (FrameType::Ack, s) if s == seq => self.on_ack(),
            (FrameType::Nack, s) if s == seq => self.on_attempt_failed(),
            (FrameType::Ack | FrameType::Nack, _) => {
                if let Some(flight) = self.inflight.as_mut() {
                    flight.empty_ticks = 0;
                }
                Ok(TxPoll::Pending)
            }
            _ => Ok(TxPoll::Pending),
        }
    }

    fn on_ack(&mut self) -> Result<TxPoll, Error<T::Error>> {
        let kind = self.inflight.as_ref().map(|f| f.kind).unwrap();
        self.inflight = None;
        match kind {
            FlightKind::Data(_) => {
                self.seq.advance();
                self.state = TxState::Idle;
                Ok(TxPoll::Acked)
            }
            FlightKind::Start => {
                self.seq = Sequence::new();
                self.state = TxState::Idle;
                Ok(TxPoll::SessionReady)
            }
            FlightKind::Finish => {
                self.state = TxState::Finished;
                Ok(TxPoll::TransferDone)
            }
        }
    }

    fn on_attempt_failed(&mut self) -> Result<TxPoll, Error<T::Error>> {
        let flight = self.inflight.as_mut().unwrap();
        if flight.retries_left == 0 {
            self.inflight = None;
            self.state = TxState::Failed;
            return Err(Error::RetriesExhausted);
        }
        flight.retries_left -= 1;
        flight.waiting = false;
        flight.empty_ticks = 0;
        self.state = TxState::Retrying;
        Ok(TxPoll::Pending)
    }

    /// START + wait for ACK.
    pub fn send_start(&mut self) -> Result<(), Error<T::Error>> {
        self.offer_start()?;
        loop {
            match self.poll()? {
                TxPoll::Pending => {}
                TxPoll::SessionReady => return Ok(()),
                TxPoll::Acked | TxPoll::TransferDone => return Ok(()),
            }
        }
    }

    /// DATA + wait for ACK.
    pub fn send_byte(&mut self, data: u8) -> Result<(), Error<T::Error>> {
        self.offer(data)?;
        loop {
            match self.poll()? {
                TxPoll::Pending => {}
                TxPoll::Acked => return Ok(()),
                TxPoll::SessionReady | TxPoll::TransferDone => return Ok(()),
            }
        }
    }

    /// FINISH + wait for ACK.
    pub fn send_finish(&mut self) -> Result<(), Error<T::Error>> {
        self.offer_finish()?;
        loop {
            match self.poll()? {
                TxPoll::Pending => {}
                TxPoll::TransferDone => return Ok(()),
                TxPoll::Acked | TxPoll::SessionReady => return Ok(()),
            }
        }
    }

    /// Unwraps the transport.
    pub fn release(self) -> T {
        self.transport
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{concat2, MockTransport};

    #[test]
    fn writes_one_byte_per_poll() {
        let transport = MockTransport::with_incoming(&Frame::ack(0).to_bytes());
        let mut sender = Sender::new(transport);
        sender.offer(0x7E).unwrap();
        for n in 1..=3 {
            assert_eq!(sender.poll().unwrap(), TxPoll::Pending);
            assert_eq!(sender.transport.written().len(), n);
            assert_eq!(sender.state(), TxState::Sending);
        }
        assert_eq!(sender.poll().unwrap(), TxPoll::Pending);
        assert_eq!(sender.transport.written().len(), 4);
        assert_eq!(sender.state(), TxState::WaitingAck);
    }

    #[test]
    fn offer_while_sending_is_not_idle() {
        let transport = MockTransport::with_incoming(&[]);
        let mut sender = Sender::new(transport);
        sender.offer(0x01).unwrap();
        assert_eq!(sender.offer(0x02), Err(Error::NotIdle));
        assert_eq!(sender.offer_finish(), Err(Error::NotIdle));
    }

    #[test]
    fn happy_path_single_byte_is_acked_and_seq_advances() {
        let transport = MockTransport::with_incoming(&Frame::ack(0).to_bytes());
        let mut sender = Sender::new(transport);
        sender.send_byte(0x7E).unwrap();
        assert_eq!(sender.next_seq(), 1);
        assert_eq!(sender.transport.written(), Frame::data(0, 0x7E).to_bytes());
    }

    #[test]
    fn nack_of_current_seq_retransmits() {
        let incoming = concat2(Frame::nack(0).to_bytes(), Frame::ack(0).to_bytes());
        let transport = MockTransport::with_incoming(&incoming);
        let mut sender = Sender::with_policy(transport, RetryPolicy::new(50, 3));
        sender.send_byte(0x11).unwrap();
        assert_eq!(
            sender.transport.written(),
            concat2(Frame::data(0, 0x11).to_bytes(), Frame::data(0, 0x11).to_bytes())
        );
    }

    #[test]
    fn ack_for_wrong_sequence_is_ignored_until_timeout() {
        let transport = MockTransport::with_incoming(&Frame::ack(99).to_bytes());
        let mut sender = Sender::with_policy(transport, RetryPolicy::new(5, 0));
        let err = sender.send_byte(0x01).unwrap_err();
        assert_eq!(err, Error::RetriesExhausted);
    }

    #[test]
    fn nack_for_wrong_sequence_is_ignored_until_timeout() {
        let transport = MockTransport::with_incoming(&Frame::nack(99).to_bytes());
        let mut sender = Sender::with_policy(transport, RetryPolicy::new(5, 0));
        assert_eq!(sender.send_byte(0x01).unwrap_err(), Error::RetriesExhausted);
        assert_eq!(sender.transport.written(), Frame::data(0, 0x01).to_bytes());
    }

    #[test]
    fn corrupted_ack_is_ignored_then_real_ack_succeeds() {
        let mut bad = Frame::ack(0).to_bytes();
        bad[3] ^= 0xFF;
        let incoming = concat2(bad, Frame::ack(0).to_bytes());
        let transport = MockTransport::with_incoming(&incoming);
        let mut sender = Sender::with_policy(transport, RetryPolicy::new(50, 3));
        sender.send_byte(0x42).unwrap();
        assert_eq!(sender.next_seq(), 1);
        assert_eq!(sender.transport.written(), Frame::data(0, 0x42).to_bytes());
    }

    #[test]
    fn timeout_with_no_response_exhausts_retries_and_fails() {
        let transport = MockTransport::with_incoming(&[]);
        let mut sender = Sender::with_policy(transport, RetryPolicy::new(2, 1));
        assert_eq!(sender.send_byte(0xAB).unwrap_err(), Error::RetriesExhausted);
        assert_eq!(sender.state(), TxState::Failed);
    }

    #[test]
    fn start_waits_for_ack() {
        let transport = MockTransport::with_incoming(&Frame::ack(0).to_bytes());
        let mut sender = Sender::new(transport);
        sender.send_start().unwrap();
        assert_eq!(sender.state(), TxState::Idle);
        assert_eq!(sender.next_seq(), 0);
        assert_eq!(sender.transport.written(), Frame::start().to_bytes());
    }

    #[test]
    fn finish_waits_for_ack() {
        let transport = MockTransport::with_incoming(&Frame::ack(0).to_bytes());
        let mut sender = Sender::new(transport);
        sender.send_finish().unwrap();
        assert_eq!(sender.state(), TxState::Finished);
        assert_eq!(sender.transport.written(), Frame::finish(0).to_bytes());
    }

    #[test]
    fn offer_after_finish_is_not_idle_until_start() {
        let transport = MockTransport::with_incoming(&Frame::ack(0).to_bytes());
        let mut sender = Sender::new(transport);
        sender.send_finish().unwrap();
        assert_eq!(sender.offer(0x01), Err(Error::NotIdle));
        sender.offer_start().unwrap();
        assert_eq!(sender.state(), TxState::Sending);
    }

    #[test]
    fn semantically_invalid_ack_is_ignored() {
        let header = [FrameType::Ack as u8, 0, 0xFF];
        let crc = crate::protocol::crc8(&header);
        let bad = [header[0], header[1], header[2], crc];
        let incoming = concat2(bad, Frame::ack(0).to_bytes());
        let transport = MockTransport::with_incoming(&incoming);
        let mut sender = Sender::with_policy(transport, RetryPolicy::new(50, 3));
        sender.send_byte(0x01).unwrap();
        assert_eq!(sender.transport.written(), Frame::data(0, 0x01).to_bytes());
    }
}

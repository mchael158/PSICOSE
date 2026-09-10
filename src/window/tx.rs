//! Heapless selective-repeat sender: `[Option<TxSlot>; N]`, `N ≤ 8`.

use crate::error::Error;
use crate::protocol::{Frame, FrameAssembler, FrameType, OutBuf, Sequence};
use crate::timeout::RetryPolicy;
use crate::transport::ByteTransport;
use crate::tx::{TxPoll, TxState};

use super::check_window;

#[derive(Clone, Copy)]
enum FlightKind {
    Data,
    Start,
    Finish,
}

#[derive(Clone, Copy)]
struct TxSlot {
    kind: FlightKind,
    seq: u8,
    payload: u8,
    retries_left: u8,
    written: bool,
    needs_retransmit: bool,
    empty_ticks: u16,
}

/// Selective-repeat sender with a compile-time window of `N` frames.
///
/// `N` is checked at compile time: `1 ≤ N ≤ 8`. No heap. One wire byte
/// per [`WindowedSender::poll`].
pub struct WindowedSender<T: ByteTransport, const N: usize> {
    transport: T,
    next_seq: Sequence,
    slots: [Option<TxSlot>; N],
    state: TxState,
    retry_policy: RetryPolicy,
    assembler: FrameAssembler,
    out: OutBuf,
    writing: Option<usize>,
}

impl<T: ByteTransport, const N: usize> WindowedSender<T, N> {
    /// Default [`RetryPolicy`].
    pub fn new(transport: T) -> Self {
        Self::with_policy(transport, RetryPolicy::default())
    }

    /// Explicit [`RetryPolicy`].
    pub fn with_policy(transport: T, retry_policy: RetryPolicy) -> Self {
        let () = check_window::<N>();
        WindowedSender {
            transport,
            next_seq: Sequence::new(),
            slots: [None; N],
            state: TxState::Idle,
            retry_policy,
            assembler: FrameAssembler::new(),
            out: OutBuf::empty(),
            writing: None,
        }
    }

    /// Current coarse state.
    pub fn state(&self) -> TxState {
        self.state
    }

    /// Sequence that the next offered DATA byte will carry.
    pub fn next_seq(&self) -> u8 {
        self.next_seq.current()
    }

    /// How many frames are in the window (unacked or not yet written).
    pub fn outstanding(&self) -> usize {
        let mut n = 0;
        for slot in &self.slots {
            if slot.is_some() {
                n += 1;
            }
        }
        n
    }

    /// Compile-time window length.
    pub const fn window_size(&self) -> usize {
        N
    }

    fn find_seq(&self, seq: u8) -> Option<usize> {
        for (i, slot) in self.slots.iter().enumerate() {
            if slot.as_ref().map(|s| s.seq) == Some(seq) {
                return Some(i);
            }
        }
        None
    }

    fn find_free(&self) -> Option<usize> {
        self.slots.iter().position(|s| s.is_none())
    }

    fn oldest_unacked(&self) -> Option<u8> {
        let mut oldest = None;
        for slot in self.slots.iter().flatten() {
            oldest = Some(match oldest {
                None => slot.seq,
                Some(base) => {
                    if slot.seq.wrapping_sub(base) as usize <= N {
                        base
                    } else {
                        slot.seq
                    }
                }
            });
        }
        oldest
    }

    fn seq_in_send_window(&self, seq: u8) -> bool {
        match self.oldest_unacked() {
            None => true,
            Some(base) => (seq.wrapping_sub(base) as usize) < N,
        }
    }

    fn has_session_flight(&self) -> bool {
        for slot in self.slots.iter().flatten() {
            if matches!(slot.kind, FlightKind::Start | FlightKind::Finish) {
                return true;
            }
        }
        false
    }

    fn insert(&mut self, kind: FlightKind, seq: u8, payload: u8) -> Result<(), Error<T::Error>> {
        let i = self.find_free().ok_or(Error::WindowFull)?;
        self.slots[i] = Some(TxSlot {
            kind,
            seq,
            payload,
            retries_left: self.retry_policy.max_retries,
            written: false,
            needs_retransmit: false,
            empty_ticks: 0,
        });
        if self.state != TxState::Sending {
            self.state = TxState::Sending;
        }
        Ok(())
    }

    /// Opens a session. Allowed from Idle / Finished / Failed when the
    /// window is empty.
    pub fn offer_start(&mut self) -> Result<(), Error<T::Error>> {
        match self.state {
            TxState::Idle | TxState::Finished | TxState::Failed => {}
            _ if self.outstanding() == 0 && self.out.is_idle() => {}
            _ => return Err(Error::NotIdle),
        }
        if self.outstanding() != 0 || !self.out.is_idle() {
            return Err(Error::NotIdle);
        }
        self.slots = [None; N];
        self.next_seq = Sequence::new();
        self.state = TxState::Sending;
        self.insert(FlightKind::Start, 0, 0)
    }

    /// Queues one DATA byte. Succeeds while the send window
    /// `[oldest_unacked, oldest_unacked+N)` has room, even if other
    /// frames are still in flight.
    pub fn offer(&mut self, data: u8) -> Result<(), Error<T::Error>> {
        if matches!(self.state, TxState::Finished | TxState::Failed) {
            return Err(Error::NotIdle);
        }
        if self.has_session_flight() {
            return Err(Error::NotIdle);
        }
        let seq = self.next_seq.current();
        if !self.seq_in_send_window(seq) || self.find_free().is_none() {
            return Err(Error::WindowFull);
        }
        self.insert(FlightKind::Data, seq, data)?;
        self.next_seq.advance();
        Ok(())
    }

    /// Queues FINISH at the next unused sequence. The window must be empty.
    pub fn offer_finish(&mut self) -> Result<(), Error<T::Error>> {
        if matches!(self.state, TxState::Finished | TxState::Failed) {
            return Err(Error::NotIdle);
        }
        if self.outstanding() != 0 || self.has_session_flight() {
            return Err(Error::NotIdle);
        }
        self.insert(FlightKind::Finish, self.next_seq.current(), 0)
    }

    fn pump_out(&mut self) -> Result<bool, Error<T::Error>> {
        let Some(byte) = self.out.peek() else {
            return Ok(true);
        };
        self.transport.write_byte(byte).map_err(Error::Transport)?;
        self.out.commit();
        Ok(self.out.is_idle())
    }

    fn next_to_write(&self) -> Option<usize> {
        let mut first_new = None;
        for (i, slot) in self.slots.iter().enumerate() {
            let Some(s) = slot else {
                continue;
            };
            if s.needs_retransmit {
                return Some(i);
            }
            if !s.written && first_new.is_none() {
                first_new = Some(i);
            }
        }
        first_new
    }

    fn load_slot(&mut self, i: usize) -> bool {
        let Some(slot) = self.slots[i] else {
            return false;
        };
        let frame = match slot.kind {
            FlightKind::Data => Frame::data(slot.seq, slot.payload),
            FlightKind::Start => Frame::start(),
            FlightKind::Finish => Frame::finish(slot.seq),
        };
        self.out.load(&frame);
        self.writing = Some(i);
        self.state = TxState::Sending;
        true
    }

    fn finish_write(&mut self) {
        if let Some(i) = self.writing.take() {
            if let Some(slot) = self.slots[i].as_mut() {
                slot.written = true;
                slot.needs_retransmit = false;
                slot.empty_ticks = 0;
            }
        }
        self.assembler.reset();
        self.state = if self.outstanding() == 0 {
            TxState::Idle
        } else {
            TxState::WaitingAck
        };
    }

    /// One non-blocking step: write at most one byte, or read at most one.
    pub fn poll(&mut self) -> Result<TxPoll, Error<T::Error>> {
        if self.state == TxState::Failed {
            return Err(Error::RetriesExhausted);
        }
        if self.state == TxState::Finished && self.outstanding() == 0 {
            return Ok(TxPoll::TransferDone);
        }

        if !self.out.is_idle() {
            let done = self.pump_out()?;
            if done {
                self.finish_write();
            }
            return Ok(TxPoll::Pending);
        }

        if let Some(i) = self.next_to_write() {
            if self.load_slot(i) {
                let done = self.pump_out()?;
                if done {
                    self.finish_write();
                }
            }
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
            None => self.tick_timeouts(),
        }
    }

    fn on_control(&mut self, frame: Frame) -> Result<TxPoll, Error<T::Error>> {
        match frame.frame_type() {
            FrameType::Ack => self.on_ack(frame.seq()),
            FrameType::Nack => self.on_nack(frame.seq()),
            _ => Ok(TxPoll::Pending),
        }
    }

    fn on_ack(&mut self, seq: u8) -> Result<TxPoll, Error<T::Error>> {
        let Some(i) = self.find_seq(seq) else {
            return Ok(TxPoll::Pending);
        };
        let Some(slot) = self.slots[i] else {
            return Ok(TxPoll::Pending);
        };
        if !slot.written {
            return Ok(TxPoll::Pending);
        }
        let kind = slot.kind;
        self.slots[i] = None;
        match kind {
            FlightKind::Data => {
                self.state = if self.outstanding() == 0 {
                    TxState::Idle
                } else {
                    TxState::WaitingAck
                };
                Ok(TxPoll::Acked)
            }
            FlightKind::Start => {
                self.next_seq = Sequence::new();
                self.state = TxState::Idle;
                Ok(TxPoll::SessionReady)
            }
            FlightKind::Finish => {
                self.state = TxState::Finished;
                Ok(TxPoll::TransferDone)
            }
        }
    }

    fn on_nack(&mut self, seq: u8) -> Result<TxPoll, Error<T::Error>> {
        let Some(i) = self.find_seq(seq) else {
            return Ok(TxPoll::Pending);
        };
        let Some(slot) = self.slots[i].as_mut() else {
            return Ok(TxPoll::Pending);
        };
        if !slot.written {
            return Ok(TxPoll::Pending);
        }
        if slot.retries_left == 0 {
            self.slots = [None; N];
            self.state = TxState::Failed;
            return Err(Error::RetriesExhausted);
        }
        slot.retries_left -= 1;
        slot.needs_retransmit = true;
        slot.empty_ticks = 0;
        self.state = TxState::Retrying;
        Ok(TxPoll::Pending)
    }

    fn tick_timeouts(&mut self) -> Result<TxPoll, Error<T::Error>> {
        let timeout = self.retry_policy.timeout_ticks;
        let writing = self.writing;
        for (i, slot) in self.slots.iter_mut().enumerate() {
            let Some(slot) = slot else {
                continue;
            };
            if writing == Some(i) || !slot.written || slot.needs_retransmit {
                continue;
            }
            slot.empty_ticks = slot.empty_ticks.saturating_add(1);
            if slot.empty_ticks < timeout {
                continue;
            }
            if slot.retries_left == 0 {
                self.slots = [None; N];
                self.state = TxState::Failed;
                return Err(Error::RetriesExhausted);
            }
            slot.retries_left -= 1;
            slot.needs_retransmit = true;
            slot.empty_ticks = 0;
            self.state = TxState::Retrying;
        }
        Ok(TxPoll::Pending)
    }

    /// START + wait for ACK.
    pub fn send_start(&mut self) -> Result<(), Error<T::Error>> {
        self.offer_start()?;
        loop {
            match self.poll()? {
                TxPoll::Pending | TxPoll::Acked => {}
                TxPoll::SessionReady | TxPoll::TransferDone => return Ok(()),
            }
        }
    }

    /// Offer one DATA byte and wait until that slot is ACKed, or until
    /// the window must be drained first.
    pub fn send_byte(&mut self, data: u8) -> Result<(), Error<T::Error>> {
        loop {
            match self.offer(data) {
                Ok(()) => break,
                Err(Error::WindowFull) => {
                    let _ = self.poll()?;
                }
                Err(e) => return Err(e),
            }
        }
        let seq = self.next_seq.current().wrapping_sub(1);
        loop {
            match self.poll()? {
                TxPoll::Acked => {
                    if self.find_seq(seq).is_none() {
                        return Ok(());
                    }
                }
                TxPoll::Pending => {}
                TxPoll::SessionReady | TxPoll::TransferDone => return Ok(()),
            }
        }
    }

    /// FINISH + wait for ACK.
    pub fn send_finish(&mut self) -> Result<(), Error<T::Error>> {
        loop {
            match self.offer_finish() {
                Ok(()) => break,
                Err(Error::NotIdle) if self.outstanding() > 0 => {
                    let _ = self.poll()?;
                }
                Err(e) => return Err(e),
            }
        }
        loop {
            match self.poll()? {
                TxPoll::Pending | TxPoll::Acked => {}
                TxPoll::TransferDone | TxPoll::SessionReady => return Ok(()),
            }
        }
    }

    /// Returns the underlying transport.
    pub fn release(self) -> T {
        self.transport
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Frame;
    use crate::test_support::{concat2, concat3, MockTransport};

    fn offer_ok<T: ByteTransport, const N: usize>(tx: &mut WindowedSender<T, N>, b: u8)
    where
        T::Error: core::fmt::Debug + PartialEq,
    {
        assert_eq!(tx.offer(b), Ok(()));
    }

    fn drain_acks<T: ByteTransport, const N: usize>(tx: &mut WindowedSender<T, N>, want: u8) {
        let mut acked = 0u8;
        for _ in 0..1024 {
            match tx.poll() {
                Ok(TxPoll::Acked) => {
                    acked += 1;
                    if acked == want {
                        return;
                    }
                }
                Ok(TxPoll::Pending) => {}
                Ok(_) | Err(_) => break,
            }
        }
        assert_eq!(acked, want);
    }

    #[test]
    fn offer_fills_the_window_then_window_full() {
        let mut tx = WindowedSender::<_, 2>::new(MockTransport::with_incoming(&[]));
        offer_ok(&mut tx, 0x10);
        offer_ok(&mut tx, 0x11);
        assert_eq!(tx.outstanding(), 2);
        assert_eq!(tx.offer(0x12), Err(Error::WindowFull));
        assert_eq!(tx.next_seq(), 2);
    }

    #[test]
    fn two_data_frames_are_written_before_any_ack() {
        let incoming = concat2(Frame::ack(0).to_bytes(), Frame::ack(1).to_bytes());
        let mut tx = WindowedSender::<_, 4>::with_policy(
            MockTransport::with_incoming(&incoming),
            RetryPolicy::new(50, 3),
        );
        offer_ok(&mut tx, 0xAA);
        offer_ok(&mut tx, 0xBB);
        drain_acks(&mut tx, 2);
        assert_eq!(tx.outstanding(), 0);
        assert_eq!(
            tx.transport.written(),
            concat2(Frame::data(0, 0xAA).to_bytes(), Frame::data(1, 0xBB).to_bytes())
        );
    }

    #[test]
    fn ack_out_of_order_frees_the_matching_slot() {
        let incoming = concat3(
            Frame::ack(1).to_bytes(),
            Frame::ack(0).to_bytes(),
            Frame::ack(2).to_bytes(),
        );
        let mut tx = WindowedSender::<_, 4>::with_policy(
            MockTransport::with_incoming(&incoming),
            RetryPolicy::new(50, 3),
        );
        offer_ok(&mut tx, 0x00);
        offer_ok(&mut tx, 0x01);
        offer_ok(&mut tx, 0x02);
        drain_acks(&mut tx, 3);
        assert_eq!(tx.outstanding(), 0);
        assert_eq!(tx.next_seq(), 3);
    }

    #[test]
    fn nack_of_one_slot_retransmits_only_that_frame() {
        let incoming = concat3(
            Frame::nack(0).to_bytes(),
            Frame::ack(0).to_bytes(),
            Frame::ack(1).to_bytes(),
        );
        let mut tx = WindowedSender::<_, 4>::with_policy(
            MockTransport::with_incoming(&incoming),
            RetryPolicy::new(50, 3),
        );
        offer_ok(&mut tx, 0x10);
        offer_ok(&mut tx, 0x11);
        drain_acks(&mut tx, 2);
        assert_eq!(
            tx.transport.written(),
            concat3(
                Frame::data(0, 0x10).to_bytes(),
                Frame::data(1, 0x11).to_bytes(),
                Frame::data(0, 0x10).to_bytes(),
            )
        );
    }

    #[test]
    fn finish_refused_while_data_is_outstanding() {
        let mut tx = WindowedSender::<_, 4>::new(MockTransport::with_incoming(&[]));
        offer_ok(&mut tx, 0x01);
        assert_eq!(tx.offer_finish(), Err(Error::NotIdle));
    }

    #[test]
    fn left_edge_blocks_seq_outside_the_window() {
        let incoming = concat3(
            Frame::ack(1).to_bytes(),
            Frame::ack(2).to_bytes(),
            Frame::ack(3).to_bytes(),
        );
        let mut tx = WindowedSender::<_, 4>::with_policy(
            MockTransport::with_incoming(&incoming),
            RetryPolicy::new(50, 3),
        );
        offer_ok(&mut tx, 0);
        offer_ok(&mut tx, 1);
        offer_ok(&mut tx, 2);
        offer_ok(&mut tx, 3);
        drain_acks(&mut tx, 3);
        assert_eq!(tx.outstanding(), 1);
        assert_eq!(tx.offer(4), Err(Error::WindowFull));
    }
}

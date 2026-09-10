//! [`RxActor`]: one receiver as a cooperative actor.
//!
//! [`crate::rx::Receiver::poll`] already consumes at most one byte and
//! returns immediately when the link is quiet. That is exactly
//! [`Actor::tick`], so this type is a thin, honest wrapper — it does not
//! buffer frames, it does not spin, and it does not change the protocol.
//!
//! # Why there is no `TxActor` yet
//!
//! [`crate::tx::Sender::poll`] writes at most one byte or reads at most
//! one. [`crate::tx::Sender::send_byte`] is only that loop stacked, for callers who
//! can block. A `TxActor` can wrap `offer` + `poll` the same way
//! [`RxActor`] wraps [`Receiver::poll`]. It is not written yet; this
//! module still only schedules the receive side.

use super::actor::{Actor, Tick};
use crate::rx::{PollOutcome, Receiver};
use crate::transport::ByteTransport;
use crate::Error;

/// A [`Receiver`] driven one byte at a time by [`Actor::tick`].
pub struct RxActor<T: ByteTransport> {
    receiver: Receiver<T>,
}

impl<T: ByteTransport> RxActor<T> {
    /// Wraps a fresh receiver over `transport`.
    pub fn new(transport: T) -> Self {
        RxActor {
            receiver: Receiver::new(transport),
        }
    }

    /// Wraps an already-constructed receiver.
    pub fn from_receiver(receiver: Receiver<T>) -> Self {
        RxActor { receiver }
    }

    /// The inner receiver.
    pub fn receiver(&self) -> &Receiver<T> {
        &self.receiver
    }

    /// The inner receiver, mutably.
    pub fn receiver_mut(&mut self) -> &mut Receiver<T> {
        &mut self.receiver
    }

    /// Unwraps the receiver.
    pub fn into_receiver(self) -> Receiver<T> {
        self.receiver
    }
}

impl<T: ByteTransport> Actor for RxActor<T> {
    type Error = Error<T::Error>;
    type Output = u8;

    fn tick(&mut self) -> Result<Tick<u8>, Self::Error> {
        Ok(match self.receiver.poll()? {
            PollOutcome::Pending
            | PollOutcome::DuplicateIgnored
            | PollOutcome::Rejected
            | PollOutcome::Started => Tick::Pending,
            PollOutcome::Delivered(byte) => Tick::Ready(byte),
            PollOutcome::TransferFinished => Tick::Done,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actors::System;
    use crate::protocol::Frame;
    use crate::test_support::MockTransport;

    /// Drive the actor the way a real `System` would: one byte per tick.
    fn tick_until_settled<T: ByteTransport>(
        actor: &mut RxActor<T>,
    ) -> Result<Tick<u8>, Error<T::Error>> {
        loop {
            match actor.tick()? {
                Tick::Pending => continue,
                other => return Ok(other),
            }
        }
    }

    #[test]
    fn pending_until_frame_is_assembled_and_ack_is_written() {
        let bytes = Frame::data_frame(0, 0x42).to_bytes();
        let mut actor = RxActor::new(MockTransport::with_incoming(&bytes));

        assert_eq!(actor.tick().unwrap(), Tick::Pending);
        assert_eq!(actor.tick().unwrap(), Tick::Pending);
        assert_eq!(actor.tick().unwrap(), Tick::Pending);
        // Fourth incoming byte + first ACK byte: still pending.
        assert_eq!(actor.tick().unwrap(), Tick::Pending);
        assert_eq!(tick_until_settled(&mut actor).unwrap(), Tick::Ready(0x42));
    }

    #[test]
    fn delivers_in_order_byte_and_finish_is_done() {
        let incoming = crate::test_support::concat2(
            Frame::data_frame(0, 0xAA).to_bytes(),
            Frame::finish(1).to_bytes(),
        );
        let mut actor = RxActor::new(MockTransport::with_incoming(&incoming));

        assert_eq!(tick_until_settled(&mut actor).unwrap(), Tick::Ready(0xAA));
        assert_eq!(tick_until_settled(&mut actor).unwrap(), Tick::Done);
    }

    #[test]
    fn duplicate_retransmission_is_pending_not_ready() {
        let incoming = crate::test_support::concat2(
            Frame::data_frame(0, 0x11).to_bytes(),
            Frame::data_frame(0, 0x11).to_bytes(),
        );
        let mut actor = RxActor::new(MockTransport::with_incoming(&incoming));

        assert_eq!(tick_until_settled(&mut actor).unwrap(), Tick::Ready(0x11));
        assert_eq!(actor.tick().unwrap(), Tick::Pending);
        assert_eq!(actor.tick().unwrap(), Tick::Pending);
        assert_eq!(actor.tick().unwrap(), Tick::Pending);
        // Fourth byte completes the duplicate: re-ACK, do not re-deliver.
        assert_eq!(actor.tick().unwrap(), Tick::Pending);
        assert_eq!(actor.receiver().expected_seq(), 1);
    }

    #[test]
    fn two_links_are_served_by_one_system_without_blocking() {
        let mut sys: System<RxActor<MockTransport>, 2> = System::new();
        let a = sys
            .spawn(RxActor::new(MockTransport::with_incoming(
                &Frame::data_frame(0, 0x10).to_bytes(),
            )))
            .unwrap();
        let b = sys
            .spawn(RxActor::new(MockTransport::with_incoming(
                &Frame::data_frame(0, 0x20).to_bytes(),
            )))
            .unwrap();

        let mut got_a = None;
        let mut got_b = None;
        // 2 actors × (4-byte DATA + 4-byte ACK), plus slack.
        for _ in 0..32 {
            let Some(Ok(step)) = sys.tick() else {
                break;
            };
            if let Tick::Ready(byte) = step.tick {
                if step.id == a {
                    got_a = Some(byte);
                } else if step.id == b {
                    got_b = Some(byte);
                }
            }
        }

        assert_eq!(got_a, Some(0x10));
        assert_eq!(got_b, Some(0x20));
    }

    #[test]
    fn corrupted_frame_is_rejected_not_delivered() {
        let mut bytes = Frame::data_frame(0, 1).to_bytes();
        bytes[2] ^= 0xFF;
        let mut actor = RxActor::new(MockTransport::with_incoming(&bytes));

        for _ in 0..bytes.len() {
            assert_eq!(actor.tick().unwrap(), Tick::Pending);
        }
        assert_eq!(actor.receiver().expected_seq(), 0);
    }
}

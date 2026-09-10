//! The cooperative-step contract.
//!
//! An [`Actor`] is allowed to do a bounded, non-blocking unit of work per
//! [`Actor::tick`]. Blocking inside `tick` starves every other actor on
//! the same [`super::System`].

/// Outcome of a single cooperative step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tick<T> {
    /// No application-visible progress this step. Call `tick` again later.
    Pending,
    /// One unit of output is ready (for [`super::RxActor`], one payload byte).
    Ready(T),
    /// The actor has finished its work and should not be ticked again.
    Done,
}

/// A heapless, non-blocking unit of work that a [`super::System`] can
/// schedule in round-robin.
pub trait Actor {
    /// Error produced by one tick. For [`super::RxActor`] this is
    /// [`crate::Error`].
    type Error;
    /// Application-visible value produced by [`Tick::Ready`].
    type Output;

    /// Advance one cooperative step. Must return promptly.
    ///
    /// Implementations that internally loop until a response arrives
    /// (see the note on TX in [`super::rx_actor`]) are not actors under
    /// this contract.
    fn tick(&mut self) -> Result<Tick<Self::Output>, Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Once {
        fired: bool,
    }

    impl Actor for Once {
        type Error = core::convert::Infallible;
        type Output = u8;

        fn tick(&mut self) -> Result<Tick<u8>, Self::Error> {
            if self.fired {
                return Ok(Tick::Done);
            }
            self.fired = true;
            Ok(Tick::Ready(7))
        }
    }

    #[test]
    fn pending_ready_done_are_distinct() {
        assert_ne!(Tick::<u8>::Pending, Tick::Ready(0));
        assert_ne!(Tick::<u8>::Ready(1), Tick::Done);
        assert_ne!(Tick::<u8>::Pending, Tick::Done);
    }

    #[test]
    fn scripted_actor_emits_ready_then_done() {
        let mut actor = Once { fired: false };
        assert_eq!(actor.tick().unwrap(), Tick::Ready(7));
        assert_eq!(actor.tick().unwrap(), Tick::Done);
        assert_eq!(actor.tick().unwrap(), Tick::Done);
    }
}

//! A fixed-capacity, round-robin scheduler.
//!
//! [`System`] stores up to `N` actors of the *same* concrete type in a
//! stack array. Heterogeneous actors would require `dyn Actor` or an enum
//! — both are possible later, but they are not needed for N identical links.

use super::actor::{Actor, Tick};

/// Handle of a spawned actor. Stable for the lifetime of the slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ActorId(usize);

impl ActorId {
    /// Index into the system's slot table (`0..N`).
    pub const fn index(self) -> usize {
        self.0
    }
}

/// The actor table is full; [`System::spawn`] failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpawnError;

/// Result of ticking one live actor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step<T> {
    /// Which actor produced this step.
    pub id: ActorId,
    /// What that actor's [`Actor::tick`] returned.
    pub tick: Tick<T>,
}

struct Slot<A> {
    actor: A,
    done: bool,
}

/// Round-robin cooperative scheduler for up to `N` actors of type `A`.
///
/// In the architecture sketch this is `System<E, N>`: `E` is
/// [`Actor::Error`], carried by `A`, and `N` is the compile-time
/// capacity. Making the *actor type* the type parameter (instead of
/// `dyn Actor<Error = E>`) is what keeps the table heapless.
pub struct System<A, const N: usize>
where
    A: Actor,
{
    slots: [Option<Slot<A>>; N],
    /// Next slot to consider. Walks `0..N` and wraps.
    cursor: usize,
    /// Occupied slots that have not yet reported [`Tick::Done`].
    live: usize,
}

impl<A: Actor, const N: usize> System<A, N> {
    /// An empty system. All slots are free.
    pub fn new() -> Self {
        System {
            slots: core::array::from_fn(|_| None),
            cursor: 0,
            live: 0,
        }
    }

    /// Maximum number of actors this system can hold.
    pub const fn capacity(&self) -> usize {
        N
    }

    /// Actors that are still scheduled (spawned and not [`Tick::Done`]).
    pub const fn live(&self) -> usize {
        self.live
    }

    /// `true` when no actor is scheduled.
    pub const fn is_idle(&self) -> bool {
        self.live == 0
    }

    /// Inserts `actor` into the first free slot.
    pub fn spawn(&mut self, actor: A) -> Result<ActorId, SpawnError> {
        for (i, slot) in self.slots.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(Slot { actor, done: false });
                self.live += 1;
                return Ok(ActorId(i));
            }
        }
        Err(SpawnError)
    }

    /// Immutable view of a spawned actor, including finished ones still
    /// occupying a slot.
    pub fn get(&self, id: ActorId) -> Option<&A> {
        self.slots
            .get(id.0)
            .and_then(|s| s.as_ref().map(|s| &s.actor))
    }

    /// Mutable view of a spawned actor.
    pub fn get_mut(&mut self, id: ActorId) -> Option<&mut A> {
        self.slots
            .get_mut(id.0)
            .and_then(|s| s.as_mut().map(|s| &mut s.actor))
    }

    /// Ticks the next live actor in round-robin order.
    ///
    /// Returns [`None`] when the system is idle (empty, or every actor
    /// has reported [`Tick::Done`]). A system of only-pending actors
    /// still returns [`Some`] — pending is not idle.
    ///
    /// An actor that returns [`Tick::Done`] is retired and skipped on
    /// later ticks. An actor that returns `Err` stays scheduled: the
    /// caller decides whether to keep polling it.
    pub fn tick(&mut self) -> Option<Result<Step<A::Output>, A::Error>> {
        if self.live == 0 || N == 0 {
            return None;
        }

        for _ in 0..N {
            let i = self.cursor;
            self.cursor = if self.cursor + 1 == N {
                0
            } else {
                self.cursor + 1
            };

            let Some(slot) = self.slots[i].as_mut() else {
                continue;
            };
            if slot.done {
                continue;
            }

            return Some(match slot.actor.tick() {
                Ok(Tick::Done) => {
                    slot.done = true;
                    self.live -= 1;
                    Ok(Step {
                        id: ActorId(i),
                        tick: Tick::Done,
                    })
                }
                Ok(tick) => Ok(Step {
                    id: ActorId(i),
                    tick,
                }),
                Err(err) => Err(err),
            });
        }

        None
    }
}

impl<A: Actor, const N: usize> Default for System<A, N> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Counter {
        next: u8,
        left: u8,
    }

    impl Actor for Counter {
        type Error = core::convert::Infallible;
        type Output = u8;

        fn tick(&mut self) -> Result<Tick<u8>, Self::Error> {
            if self.left == 0 {
                return Ok(Tick::Done);
            }
            self.left -= 1;
            let v = self.next;
            self.next += 1;
            Ok(Tick::Ready(v))
        }
    }

    struct Pending;

    impl Actor for Pending {
        type Error = core::convert::Infallible;
        type Output = u8;

        fn tick(&mut self) -> Result<Tick<u8>, Self::Error> {
            Ok(Tick::Pending)
        }
    }

    struct Fail {
        remaining_ok: u8,
    }

    impl Actor for Fail {
        type Error = u8;
        type Output = u8;

        fn tick(&mut self) -> Result<Tick<u8>, Self::Error> {
            if self.remaining_ok > 0 {
                self.remaining_ok -= 1;
                return Ok(Tick::Pending);
            }
            Err(9)
        }
    }

    fn ready(step: Option<Result<Step<u8>, core::convert::Infallible>>) -> (usize, u8) {
        match step {
            Some(Ok(step)) => match step.tick {
                Tick::Ready(v) => (step.id.index(), v),
                tick => {
                    assert_eq!(tick, Tick::Ready(0));
                    (0, 0)
                }
            },
            other => {
                assert!(other.is_some());
                (0, 0)
            }
        }
    }

    #[test]
    fn spawn_assigns_stable_ids_in_slot_order() {
        let mut sys: System<Counter, 3> = System::new();
        assert_eq!(
            sys.spawn(Counter { next: 0, left: 1 }).map(|id| id.index()),
            Ok(0)
        );
        assert_eq!(
            sys.spawn(Counter { next: 0, left: 1 }).map(|id| id.index()),
            Ok(1)
        );
        assert_eq!(sys.live(), 2);
        assert_eq!(sys.capacity(), 3);
    }

    #[test]
    fn spawn_fails_when_full() {
        let mut sys: System<Pending, 1> = System::new();
        assert!(sys.spawn(Pending).is_ok());
        assert_eq!(sys.spawn(Pending), Err(SpawnError));
    }

    #[test]
    fn tick_none_when_empty() {
        let mut sys: System<Pending, 4> = System::new();
        assert!(sys.is_idle());
        assert!(sys.tick().is_none());
    }

    #[test]
    fn round_robin_visits_in_spawn_order() {
        let mut sys: System<Counter, 2> = System::new();
        assert!(sys.spawn(Counter { next: 10, left: 2 }).is_ok());
        assert!(sys.spawn(Counter { next: 20, left: 2 }).is_ok());

        assert_eq!(ready(sys.tick()), (0, 10));
        assert_eq!(ready(sys.tick()), (1, 20));
        assert_eq!(ready(sys.tick()), (0, 11));
        assert_eq!(ready(sys.tick()), (1, 21));
    }

    #[test]
    fn done_actor_is_retired_and_skipped() {
        let mut sys: System<Counter, 2> = System::new();
        assert!(sys.spawn(Counter { next: 1, left: 1 }).is_ok());
        assert!(sys.spawn(Counter { next: 2, left: 3 }).is_ok());

        assert_eq!(ready(sys.tick()), (0, 1));
        assert_eq!(
            sys.tick().map(|r| r.map(|s| s.tick)),
            Some(Ok(Tick::Ready(2)))
        );

        assert_eq!(sys.tick().map(|r| r.map(|s| s.tick)), Some(Ok(Tick::Done)));
        assert_eq!(sys.live(), 1);

        assert_eq!(ready(sys.tick()), (1, 3));
        assert_eq!(ready(sys.tick()), (1, 4));
        assert_eq!(sys.tick().map(|r| r.map(|s| s.tick)), Some(Ok(Tick::Done)));
        assert!(sys.tick().is_none());
    }

    #[test]
    fn pending_is_not_idle() {
        let mut sys: System<Pending, 1> = System::new();
        assert!(sys.spawn(Pending).is_ok());
        assert!(!sys.is_idle());
        assert_eq!(
            sys.tick().map(|r| r.map(|s| s.tick)),
            Some(Ok(Tick::Pending))
        );
        assert_eq!(sys.live(), 1);
    }

    #[test]
    fn error_is_propagated_and_actor_stays_scheduled() {
        let mut sys: System<Fail, 1> = System::new();
        let spawned = sys.spawn(Fail { remaining_ok: 1 });
        assert!(spawned.is_ok());
        if let Ok(id) = spawned {
            assert_eq!(
                sys.tick().map(|r| r.map(|s| s.tick)),
                Some(Ok(Tick::Pending))
            );
            assert_eq!(sys.tick(), Some(Err(9)));
            assert_eq!(sys.live(), 1);
            assert!(sys.get(id).is_some());
            assert_eq!(sys.tick(), Some(Err(9)));
        }
    }
}

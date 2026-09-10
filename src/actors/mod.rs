//! Cooperative orchestration **above** the wire protocol, still inside
//! this crate.
//!
//! The protocol (`frame`, `Sender`, `Receiver`) does not depend on this
//! module. This layer sits on top: when you have *N* links and want to
//! serve them from one thread, without an RTOS, you register one
//! [`RxActor`] per link on a [`System`] and call [`System::tick`].
//!
//! ```text
//!                  APPLICATION
//!                       │
//!                       ▼
//!             ┌──────────────────┐
//!             │  System<A, N>    │   ← psicose::actors
//!             │  (round-robin)   │
//!             └─────────┬────────┘
//!                        │ tick()
//!           ┌────────────┼────────────┐
//!           ▼            ▼            ▼
//!      RxActor<T>   RxActor<T>    (TxActor — see rx_actor)
//!           │            │
//!           ▼            ▼
//!    crate::rx::Receiver<T>
//!           │            │
//!           ▼            ▼
//!       UART #1      RADIO #1
//! ```
//!
//! [`crate::rx::Receiver::poll`] is already non-blocking and consumes
//! exactly one byte per call — that is the [`Actor::tick`] contract, so
//! [`RxActor`] is an honest wrapper. The actor table is
//! `[Option<Slot<A>>; N]` on the stack: no heap.

pub mod actor;
pub mod rx_actor;
pub mod system;

pub use actor::{Actor, Tick};
pub use rx_actor::RxActor;
pub use system::{ActorId, SpawnError, Step, System};

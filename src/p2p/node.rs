//! Local PSICOSE **node** — the framework entry point.
//!
//! ```text
//!        exemplos / application
//!                 │
//!                 ▼
//!        ┌─────────────────┐
//!        │  psicose::Node  │  ← motor (identity + neighbors + connect)
//!        └────────┬────────┘
//!                 │ PeerLink
//!                 ▼
//!              Wire / UART
//! ```
//!
//! You do **not** bring an external P2P stack and pass sockets into PSICOSE.
//! The node, table, hello, and link **are** PSICOSE.

use crate::pump::WindowedPump;
use crate::transport::ByteTransport;
use crate::window::MAX_WINDOW;

use super::link::PeerLink;
use super::peer::PeerId;
use super::session::SessionConfig;
use super::table::{PeerTable, TableError};

/// One local endpoint of the PSICOSE framework.
///
/// Owns the [`PeerTable`] (who we are + neighbors). Opens
/// [`PeerLink`]s on pumps that already speak the PSICOSE
/// wire ([`super::Wire::link_pumps`], or [`crate::LinkFace`] on UART).
///
/// # Example
///
/// ```
/// use psicose::prelude::*;
///
/// let wire = Wire::new();
/// let (pa, pb) = wire.link_pumps();
/// let mut alice = Node::<4>::with(PeerId::from_label(b"alice"), SessionConfig::FORUM);
/// let mut bob = Node::<4>::with(PeerId::from_label(b"bob"), SessionConfig::FORUM);
/// let mut a = alice.connect(pa).unwrap();
/// let mut b = bob.accept(pb);
/// assert!(establish(&mut a, &mut alice, &mut b, &mut bob));
/// ```
pub struct Node<const N: usize> {
    table: PeerTable<N>,
}

impl<const N: usize> Node<N> {
    /// Empty node offering [`SessionConfig::DEFAULT`].
    pub const fn new(local: PeerId) -> Self {
        Self {
            table: PeerTable::new(local),
        }
    }

    /// Empty node offering `config` on every connect/accept hello.
    pub const fn with(local: PeerId, config: SessionConfig) -> Self {
        Self {
            table: PeerTable::with(local, config),
        }
    }

    /// Our identity.
    pub const fn id(&self) -> PeerId {
        self.table.local()
    }

    /// Session offer announced in hellos.
    pub const fn config(&self) -> SessionConfig {
        self.table.config()
    }

    /// Neighbor directory (read-only).
    pub const fn table(&self) -> &PeerTable<N> {
        &self.table
    }

    /// Neighbor directory (for [`PeerLink::poll`] and lookups).
    pub fn table_mut(&mut self) -> &mut PeerTable<N> {
        &mut self.table
    }

    /// How many neighbors are `Established`.
    pub fn established(&self) -> usize {
        self.table.established()
    }

    /// Outgoing connect: allocate a slot and drive START + hello on `pump`.
    pub fn connect<Tx, Rx>(
        &mut self,
        pump: WindowedPump<Tx, Rx, MAX_WINDOW>,
    ) -> Result<PeerLink<Tx, Rx>, TableError>
    where
        Tx: ByteTransport,
        Rx: ByteTransport<Error = Tx::Error>,
    {
        PeerLink::connect(&mut self.table, pump)
    }

    /// Incoming accept: wait for hello on `pump`, then reply.
    pub fn accept<Tx, Rx>(&self, pump: WindowedPump<Tx, Rx, MAX_WINDOW>) -> PeerLink<Tx, Rx>
    where
        Tx: ByteTransport,
        Rx: ByteTransport<Error = Tx::Error>,
    {
        PeerLink::accept(&self.table, pump)
    }
}

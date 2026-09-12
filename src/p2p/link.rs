//! One live neighbor: a [`WindowedPump`] plus the hello state machine.
//!
//! ```text
//! connect:  START → hello(12) → wait hello → Established
//! accept:   wait hello → START → hello(12) → Established
//! ```
//!
//! Handshake DATA uses window limit 1 (stop-and-wait). After negotiate,
//! if [`Capabilities::WINDOW`] is set, the runtime limit becomes the
//! agreed `max_window` so DATA can pipeline. [`PeerLink::poll`] never loops.

use crate::error::Error;
use crate::pump::{PumpEvent, WindowedPump};
use crate::transport::ByteTransport;
use crate::tx::TxState;
use crate::window::MAX_WINDOW;

use super::session::{Capabilities, HandshakeError, PeerSession, SessionState, HELLO_LEN};
use super::table::{PeerTable, TableError};

/// Outcome of one [`PeerLink::poll`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkEvent {
    /// Quiet.
    Idle,
    /// START, hello byte, ACK, or other work — session not established yet.
    Progress,
    /// Hellos crossed. DATA may follow.
    Established,
    /// A payload byte after the handshake.
    Received(u8),
    /// Transport FINISH completed.
    Closed,
    /// Transport ABORT cancelled the link.
    Aborted,
}

/// Errors from a live link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkError<E> {
    /// The underlying pump / transport failed.
    Transport(Error<E>),
    /// The hello was invalid.
    Handshake(HandshakeError),
    /// The peer table rejected the slot.
    Table(TableError),
}

impl<E: core::fmt::Debug> core::fmt::Display for LinkError<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            LinkError::Transport(e) => write!(f, "transport: {e}"),
            LinkError::Handshake(e) => write!(f, "handshake: {e}"),
            LinkError::Table(e) => write!(f, "table: {e}"),
        }
    }
}

/// Connector or listener over one [`WindowedPump`] (`N = MAX_WINDOW`).
///
/// Prefer opening links through [`crate::Node::connect`] /
/// [`crate::Node::accept`]. These constructors are the low-level path
/// when you already hold a [`PeerTable`] yourself.
pub struct PeerLink<Tx, Rx>
where
    Tx: ByteTransport,
    Rx: ByteTransport<Error = Tx::Error>,
{
    pump: WindowedPump<Tx, Rx, MAX_WINDOW>,
    session: PeerSession,
    slot: Option<usize>,
    outgoing: [u8; HELLO_LEN],
    out_at: usize,
    out_len: usize,
    incoming: [u8; HELLO_LEN],
    in_len: usize,
    need_start: bool,
    initiator: bool,
    established: bool,
    announced: bool,
}

impl<Tx, Rx> PeerLink<Tx, Rx>
where
    Tx: ByteTransport,
    Rx: ByteTransport<Error = Tx::Error>,
{
    /// Allocates a table slot and starts an outgoing connect (START + hello).
    ///
    /// Prefer [`crate::Node::connect`] unless you manage the table yourself.
    pub fn connect<const N: usize>(
        table: &mut PeerTable<N>,
        mut pump: WindowedPump<Tx, Rx, MAX_WINDOW>,
    ) -> Result<Self, TableError> {
        pump.set_window_limit(1);
        let (slot, hello) = table.connect()?;
        let session = match table.get(slot) {
            Some(entry) => *entry.session(),
            None => return Err(TableError::Empty),
        };
        Ok(PeerLink {
            pump,
            session,
            slot: Some(slot),
            outgoing: hello,
            out_at: 0,
            out_len: HELLO_LEN,
            incoming: [0; HELLO_LEN],
            in_len: 0,
            need_start: true,
            initiator: true,
            established: false,
            announced: false,
        })
    }

    /// Listens on `pump`. The table slot is taken when the hello arrives.
    ///
    /// Prefer [`crate::Node::accept`] unless you manage the table yourself.
    pub fn accept<const N: usize>(
        table: &PeerTable<N>,
        mut pump: WindowedPump<Tx, Rx, MAX_WINDOW>,
    ) -> Self {
        pump.set_window_limit(1);
        PeerLink {
            pump,
            session: PeerSession::new(table.local(), table.config()),
            slot: None,
            outgoing: [0; HELLO_LEN],
            out_at: 0,
            out_len: 0,
            incoming: [0; HELLO_LEN],
            in_len: 0,
            need_start: false,
            initiator: false,
            established: false,
            announced: false,
        }
    }

    /// The session bookkeeping (copied from the table on each settle).
    pub const fn session(&self) -> &PeerSession {
        &self.session
    }

    /// Table slot, once allocated.
    pub const fn slot(&self) -> Option<usize> {
        self.slot
    }

    /// The windowed pump.
    pub fn pump(&self) -> &WindowedPump<Tx, Rx, MAX_WINDOW> {
        &self.pump
    }

    /// The windowed pump, mutably (offer DATA after [`LinkEvent::Established`]).
    pub fn pump_mut(&mut self) -> &mut WindowedPump<Tx, Rx, MAX_WINDOW> {
        &mut self.pump
    }

    /// Offer one payload byte. Same as `self.pump_mut().sender_mut().offer(byte)`.
    ///
    /// After a negotiated `WINDOW`, several offers may succeed before the
    /// first ACK (up to the runtime window limit).
    pub fn offer(&mut self, byte: u8) -> Result<(), Error<Tx::Error>> {
        self.pump.sender_mut().offer(byte)
    }

    /// Offer FINISH once the payload is done.
    pub fn finish(&mut self) -> Result<(), Error<Tx::Error>> {
        self.pump.sender_mut().offer_finish()
    }

    /// ABORT on the wire and mark the session aborted.
    pub fn abort<const N: usize>(
        &mut self,
        table: &mut PeerTable<N>,
    ) -> Result<(), LinkError<Tx::Error>> {
        self.pump.abort().map_err(LinkError::Transport)?;
        self.session.abort();
        self.sync(table);
        Ok(())
    }

    fn sync<const N: usize>(&mut self, table: &mut PeerTable<N>) {
        self.session.record_stats(self.pump.stats());
        if let Some(slot) = self.slot {
            let _ = table.store(slot, self.session);
        }
    }

    /// Raise DATA pipelining to the negotiated window when `WINDOW` is set.
    fn apply_negotiated_window(&mut self) {
        let Some(cfg) = self.session.negotiated() else {
            return;
        };
        let limit = if cfg.features.contains(Capabilities::WINDOW) {
            cfg.max_window as usize
        } else {
            1
        };
        self.pump.set_window_limit(limit);
    }

    fn offer_next(&mut self) -> Result<(), Error<Tx::Error>> {
        // Hello bytes stay stop-and-wait even on a windowed pump.
        if self.pump.sender().outstanding() != 0 || self.pump.sender().state() != TxState::Idle {
            return Ok(());
        }
        if self.need_start {
            self.pump.sender_mut().offer_start()?;
            self.need_start = false;
            return Ok(());
        }
        if self.out_at < self.out_len {
            let byte = self.outgoing[self.out_at];
            self.pump.sender_mut().offer(byte)?;
            self.out_at += 1;
        }
        Ok(())
    }

    fn on_hello_byte<const N: usize>(
        &mut self,
        table: &mut PeerTable<N>,
        byte: u8,
    ) -> Result<LinkEvent, LinkError<Tx::Error>> {
        if self.in_len >= HELLO_LEN {
            return Ok(LinkEvent::Progress);
        }
        self.incoming[self.in_len] = byte;
        self.in_len += 1;
        if self.in_len < HELLO_LEN {
            return Ok(LinkEvent::Progress);
        }

        if self.initiator {
            let slot = self.slot.ok_or(LinkError::Table(TableError::Empty))?;
            table
                .finish_connect(slot, &self.incoming)
                .map_err(LinkError::Table)?;
            if let Some(entry) = table.get(slot) {
                self.session = *entry.session();
            }
        } else {
            let (slot, reply) = table.accept(&self.incoming).map_err(LinkError::Table)?;
            self.slot = Some(slot);
            if let Some(entry) = table.get(slot) {
                self.session = *entry.session();
            }
            self.outgoing = reply;
            self.out_len = HELLO_LEN;
            self.out_at = 0;
            self.need_start = true;
        }

        self.established = self.session.state() == SessionState::Established;
        if self.established {
            self.apply_negotiated_window();
        }
        self.sync(table);
        if self.established && self.initiator && !self.announced {
            self.announced = true;
            Ok(LinkEvent::Established)
        } else {
            Ok(LinkEvent::Progress)
        }
    }

    /// One cooperative step. Polls the peer table when the hello completes
    /// so A and B show up as neighbors.
    pub fn poll<const N: usize>(
        &mut self,
        table: &mut PeerTable<N>,
    ) -> Result<LinkEvent, LinkError<Tx::Error>> {
        self.offer_next().map_err(LinkError::Transport)?;

        let ev = self.pump.poll().map_err(LinkError::Transport)?;
        self.session.record_stats(self.pump.stats());

        match ev {
            PumpEvent::Aborted => {
                self.session.abort();
                self.sync(table);
                Ok(LinkEvent::Aborted)
            }
            PumpEvent::Completed => {
                self.session.closed();
                self.sync(table);
                Ok(LinkEvent::Closed)
            }
            PumpEvent::Received(byte) => {
                if self.established {
                    Ok(LinkEvent::Received(byte))
                } else {
                    self.on_hello_byte(table, byte)
                }
            }
            PumpEvent::Idle => Ok(LinkEvent::Idle),
            PumpEvent::Progress | PumpEvent::Sent => {
                if !self.initiator
                    && self.established
                    && !self.announced
                    && self.out_at >= self.out_len
                    && !self.need_start
                    && self.pump.sender().state() == TxState::Idle
                    && self.pump.sender().outstanding() == 0
                {
                    self.announced = true;
                    Ok(LinkEvent::Established)
                } else {
                    Ok(LinkEvent::Progress)
                }
            }
        }
    }
}

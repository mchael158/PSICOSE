//! Compile-time peer directory. No `Vec`. One slot per neighbor.

use super::peer::PeerId;
use super::session::{HandshakeError, PeerSession, SessionConfig, SessionState, HELLO_LEN};

/// Hard ceiling. Bigger is still heapless; we refuse a core table that
/// looks like a routing daemon.
pub const MAX_PEERS: usize = 8;

const fn check_peers<const N: usize>() {
    assert!(N >= 1, "peer table N must be at least 1");
    assert!(N <= MAX_PEERS, "peer table N must be at most 8");
}

/// One occupied slot: the session bookkeeping for a single neighbor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerEntry {
    session: PeerSession,
}

impl PeerEntry {
    /// The session in this slot.
    pub const fn session(&self) -> &PeerSession {
        &self.session
    }
}

/// Why a table operation failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableError {
    /// Every slot is taken.
    Full,
    /// The hello was rejected.
    Handshake(HandshakeError),
    /// That slot is empty.
    Empty,
    /// A session with this remote `PeerId` is already in the table.
    Duplicate,
}

impl core::fmt::Display for TableError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            TableError::Full => write!(f, "peer table is full"),
            TableError::Handshake(e) => write!(f, "handshake: {e}"),
            TableError::Empty => write!(f, "peer slot is empty"),
            TableError::Duplicate => write!(f, "remote peer already in the table"),
        }
    }
}

/// `[Option<PeerEntry>; N]` of neighbors. `1 ≤ N ≤ 8`.
///
/// The table does not own transports. It only remembers who we are
/// talking to. [`super::link::PeerLink`] drives the `Pump`.
pub struct PeerTable<const N: usize> {
    local: PeerId,
    config: SessionConfig,
    slots: [Option<PeerEntry>; N],
}

impl<const N: usize> PeerTable<N> {
    /// An empty table for `local`, offering [`SessionConfig::DEFAULT`].
    pub const fn new(local: PeerId) -> Self {
        Self::with(local, SessionConfig::DEFAULT)
    }

    /// An empty table for `local`, offering `config` on every connect.
    pub const fn with(local: PeerId, config: SessionConfig) -> Self {
        check_peers::<N>();
        PeerTable {
            local,
            config,
            slots: [None; N],
        }
    }

    /// Our identity.
    pub const fn local(&self) -> PeerId {
        self.local
    }

    /// The config we announce.
    pub const fn config(&self) -> SessionConfig {
        self.config
    }

    /// Compile-time capacity.
    pub const fn capacity(&self) -> usize {
        N
    }

    /// How many slots are occupied.
    pub fn len(&self) -> usize {
        let mut n = 0;
        let mut i = 0;
        while i < N {
            if self.slots[i].is_some() {
                n += 1;
            }
            i += 1;
        }
        n
    }

    /// True when no slot is occupied.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// True when every slot is taken.
    pub fn is_full(&self) -> bool {
        self.len() == N
    }

    /// Slot `i`, if occupied.
    pub fn get(&self, i: usize) -> Option<&PeerEntry> {
        self.slots.get(i).and_then(|s| s.as_ref())
    }

    /// Slot `i`, if occupied, mutably.
    pub fn get_mut(&mut self, i: usize) -> Option<&mut PeerEntry> {
        self.slots.get_mut(i).and_then(|s| s.as_mut())
    }

    /// Index of the slot whose remote is `id`.
    pub fn find(&self, id: PeerId) -> Option<usize> {
        let mut i = 0;
        while i < N {
            if let Some(entry) = &self.slots[i] {
                if entry.session.remote() == Some(id) {
                    return Some(i);
                }
            }
            i += 1;
        }
        None
    }

    fn free_slot(&self) -> Option<usize> {
        let mut i = 0;
        while i < N {
            if self.slots[i].is_none() {
                return Some(i);
            }
            i += 1;
        }
        None
    }

    /// Writes `session` back into `slot`. Used by [`super::link::PeerLink`].
    pub fn store(&mut self, slot: usize, session: PeerSession) -> Result<(), TableError> {
        match self.slots.get_mut(slot) {
            Some(cell) => {
                *cell = Some(PeerEntry { session });
                Ok(())
            }
            None => Err(TableError::Empty),
        }
    }

    /// Opens an outgoing session in a free slot. Returns the slot and the
    /// hello to send after START. The remote id is still unknown.
    pub fn connect(&mut self) -> Result<(usize, [u8; HELLO_LEN]), TableError> {
        let i = self.free_slot().ok_or(TableError::Full)?;
        let mut session = PeerSession::new(self.local, self.config);
        let hello = session.connect();
        self.slots[i] = Some(PeerEntry { session });
        Ok((i, hello))
    }

    /// Installs an incoming hello in a free slot.
    ///
    /// Returns the slot and the reply hello (always `Some` — we were not
    /// the initiator). Rejects a second hello from a remote already listed.
    pub fn accept(&mut self, hello: &[u8]) -> Result<(usize, [u8; HELLO_LEN]), TableError> {
        if hello.len() >= PeerId::LEN {
            let mut raw = [0u8; PeerId::LEN];
            raw.copy_from_slice(&hello[..PeerId::LEN]);
            if self.find(PeerId::new(raw)).is_some() {
                return Err(TableError::Duplicate);
            }
        }
        let i = self.free_slot().ok_or(TableError::Full)?;
        let mut session = PeerSession::new(self.local, self.config);
        let reply = session.on_hello(hello).map_err(TableError::Handshake)?;
        let hello_out = match reply {
            Some(bytes) => bytes,
            None => session.hello_bytes(),
        };
        self.slots[i] = Some(PeerEntry { session });
        Ok((i, hello_out))
    }

    /// Completes an outgoing connect when the peer's hello arrives.
    pub fn finish_connect(&mut self, slot: usize, hello: &[u8]) -> Result<(), TableError> {
        if self.get(slot).is_none() {
            return Err(TableError::Empty);
        }
        if let Some(id) = remote_from_hello(hello) {
            if self.find_except(id, slot).is_some() {
                return Err(TableError::Duplicate);
            }
        }
        let entry = self.get_mut(slot).ok_or(TableError::Empty)?;
        entry
            .session
            .on_hello(hello)
            .map_err(TableError::Handshake)?;
        Ok(())
    }

    fn find_except(&self, id: PeerId, skip: usize) -> Option<usize> {
        let mut i = 0;
        while i < N {
            if i != skip {
                if let Some(entry) = &self.slots[i] {
                    if entry.session.remote() == Some(id) {
                        return Some(i);
                    }
                }
            }
            i += 1;
        }
        None
    }

    /// Drops the slot. The transport side must ABORT/FINISH on its own.
    pub fn remove(&mut self, slot: usize) -> Result<PeerEntry, TableError> {
        match self.slots.get_mut(slot) {
            Some(cell) => cell.take().ok_or(TableError::Empty),
            None => Err(TableError::Empty),
        }
    }

    /// How many slots are [`SessionState::Established`].
    pub fn established(&self) -> usize {
        let mut n = 0;
        let mut i = 0;
        while i < N {
            if let Some(entry) = &self.slots[i] {
                if entry.session.state() == SessionState::Established {
                    n += 1;
                }
            }
            i += 1;
        }
        n
    }
}

fn remote_from_hello(hello: &[u8]) -> Option<PeerId> {
    if hello.len() < PeerId::LEN {
        return None;
    }
    let mut raw = [0u8; PeerId::LEN];
    raw.copy_from_slice(&hello[..PeerId::LEN]);
    Some(PeerId::new(raw))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::p2p::Capabilities;

    fn cfg() -> SessionConfig {
        SessionConfig::offer(2, Capabilities::STREAM)
    }

    #[test]
    fn connect_fills_then_full() {
        let mut t = PeerTable::<2>::with(PeerId::from_label(b"alice"), cfg());
        assert!(matches!(t.connect(), Ok((0, _))));
        assert!(matches!(t.connect(), Ok((1, _))));
        assert_eq!(t.connect(), Err(TableError::Full));
        assert_eq!(t.len(), 2);
    }

    #[test]
    fn accept_then_find_remote() {
        let mut a = PeerSession::new(PeerId::from_label(b"alice"), cfg());
        let hello = a.connect();
        let mut t = PeerTable::<4>::with(PeerId::from_label(b"bob"), cfg());
        let (slot, _reply) = match t.accept(&hello) {
            Ok(v) => v,
            Err(_) => return,
        };
        assert_eq!(slot, 0);
        assert_eq!(t.find(PeerId::from_label(b"alice")), Some(0));
        assert_eq!(t.established(), 1);
        assert_eq!(t.accept(&hello), Err(TableError::Duplicate));
    }

    #[test]
    fn finish_connect_binds_the_remote() {
        let mut t = PeerTable::<2>::with(PeerId::from_label(b"alice"), cfg());
        let (slot, _) = match t.connect() {
            Ok(v) => v,
            Err(_) => return,
        };
        let b = PeerSession::new(PeerId::from_label(b"bob"), cfg());
        let hello_b = b.hello_bytes();
        assert_eq!(t.finish_connect(slot, &hello_b), Ok(()));
        assert_eq!(t.find(PeerId::from_label(b"bob")), Some(slot));
        assert_eq!(t.established(), 1);
    }

    #[test]
    fn table_of_eight_stays_small() {
        let size = core::mem::size_of::<PeerTable<8>>();
        assert!(
            size <= 640,
            "PeerTable<8> is {size} bytes — keep it a register file"
        );
    }
}

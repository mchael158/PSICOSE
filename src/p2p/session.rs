//! Explicit peer session: identity, capabilities, and the hello handshake.
//!
//! ```text
//! CONNECT → ESTABLISHED → DATA … → FINISH → CLOSED
//!                └────────── ABORT → ABORTED
//! ```
//!
//! The hello is **12 payload bytes**, not a new frame type:
//!
//! ```text
//! ┌────────────┬─────────┬────────────┬──────────────┐
//! │ PeerId (8) │ ver (1) │ window (1) │ features (2) │
//! └────────────┴─────────┴────────────┴──────────────┘
//! ```
//!
//! [`PeerSession`] is bookkeeping only. The bytes it produces ride the
//! transport ([`Sender`](crate::tx::Sender) / [`Pump`](crate::pump::Pump))
//! like any other application data; the state mirrors the transport
//! session (Connecting ≈ START, Closing ≈ FINISH, Aborted ≈ ABORT).

use crate::pump::SessionStats;
use crate::window::MAX_WINDOW;

use super::peer::PeerId;

/// Version this crate speaks. Carried in the hello, never in the frame.
pub const PROTOCOL_VERSION: u8 = 1;

/// Hello size on the wire: [`PeerId::LEN`] + [`SessionConfig::WIRE_LEN`].
pub const HELLO_LEN: usize = PeerId::LEN + SessionConfig::WIRE_LEN;

/// Feature bits announced in the hello.
///
/// CRC is deliberately **not** a capability: the transport frame always
/// carries it. Only optional behavior is negotiated.
///
/// Bits are **announcements**, except where a layer honours them:
/// - [`Self::WINDOW`]: [`PeerLink`](crate::PeerLink) raises its runtime
///   DATA window to the negotiated `max_window` after hello (still
///   `N ≤ 8`). Outside P2P, the app may also use [`WindowedSender`](crate::WindowedSender).
/// - [`Self::FRAGMENTATION`]: the app chooses [`Fragmenter`](crate::Fragmenter).
/// - [`Self::COMPRESSION`]: reserved (no implementation).
/// - [`Self::ENCRYPTION`]: with feature `aead`, means peers may use
///   [`crate::aead`] on application payloads before the transport.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Capabilities {
    bits: u16,
}

impl Capabilities {
    /// No optional features.
    pub const NONE: Capabilities = Capabilities { bits: 0 };
    /// Peer may use selective-repeat (`N ≤ 8`). Honoured by [`crate::PeerLink`]
    /// after hello (runtime window limit = negotiated `max_window`).
    pub const WINDOW: Capabilities = Capabilities { bits: 1 << 0 };
    /// Byte streams over the session.
    pub const STREAM: Capabilities = Capabilities { bits: 1 << 1 };
    /// Application messages (bit in the hello only).
    pub const FORUM: Capabilities = Capabilities { bits: 1 << 2 };
    /// Reserved: advertisement only. No compression is implemented.
    pub const COMPRESSION: Capabilities = Capabilities { bits: 1 << 3 };
    /// With feature `aead`: sealed application payloads via [`crate::aead`].
    /// Without `aead`: advertisement only.
    pub const ENCRYPTION: Capabilities = Capabilities { bits: 1 << 4 };
    /// Peer may use [`crate::Fragmenter`] on application messages.
    pub const FRAGMENTATION: Capabilities = Capabilities { bits: 1 << 5 };

    /// Raw bit set (wire form: big-endian `u16` in the hello).
    pub const fn bits(self) -> u16 {
        self.bits
    }

    /// Rebuilds from raw bits. Unknown bits are kept as-is; they simply
    /// never survive an intersection with a peer that ignores them.
    pub const fn from_bits(bits: u16) -> Self {
        Capabilities { bits }
    }

    /// Union: announce both.
    pub const fn with(self, other: Capabilities) -> Self {
        Capabilities {
            bits: self.bits | other.bits,
        }
    }

    /// Intersection: what both peers support. This *is* the negotiation.
    pub const fn common(self, other: Capabilities) -> Self {
        Capabilities {
            bits: self.bits & other.bits,
        }
    }

    /// Does this set include every bit of `other`?
    pub const fn contains(self, other: Capabilities) -> bool {
        self.bits & other.bits == other.bits
    }
}

impl core::ops::BitOr for Capabilities {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        self.with(rhs)
    }
}

impl core::ops::BitAnd for Capabilities {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self {
        self.common(rhs)
    }
}

/// What a peer offers before DATA flows. Serialized as 4 payload bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionConfig {
    /// Protocol version ([`PROTOCOL_VERSION`] unless testing compat).
    pub protocol_version: u8,
    /// Largest selective-repeat window this peer accepts (`1..=8`).
    pub max_window: u8,
    /// Announced features.
    pub features: Capabilities,
}

impl SessionConfig {
    /// Wire size of a serialized config.
    pub const WIRE_LEN: usize = 4;

    /// This crate's usual offer: version [`PROTOCOL_VERSION`], window 8,
    /// [`Capabilities::STREAM`].
    pub const DEFAULT: SessionConfig =
        SessionConfig::new(PROTOCOL_VERSION, MAX_WINDOW as u8, Capabilities::STREAM);

    /// Window 8 plus stream, window, app-message, and fragmentation bits.
    pub const FORUM: SessionConfig = SessionConfig::offer(
        MAX_WINDOW as u8,
        Capabilities::STREAM
            .with(Capabilities::WINDOW)
            .with(Capabilities::FORUM)
            .with(Capabilities::FRAGMENTATION),
    );

    /// [`Self::FORUM`] plus [`Capabilities::ENCRYPTION`] (announce AEAD).
    pub const SECURE: SessionConfig = SessionConfig::offer(
        MAX_WINDOW as u8,
        Capabilities::STREAM
            .with(Capabilities::WINDOW)
            .with(Capabilities::FORUM)
            .with(Capabilities::FRAGMENTATION)
            .with(Capabilities::ENCRYPTION),
    );

    /// Builds a config, clamping `max_window` into `1..=MAX_WINDOW`.
    pub const fn new(protocol_version: u8, max_window: u8, features: Capabilities) -> Self {
        let clamped = if max_window == 0 {
            1
        } else if max_window > MAX_WINDOW as u8 {
            MAX_WINDOW as u8
        } else {
            max_window
        };
        SessionConfig {
            protocol_version,
            max_window: clamped,
            features,
        }
    }

    /// [`PROTOCOL_VERSION`] plus the given window and features.
    pub const fn offer(max_window: u8, features: Capabilities) -> Self {
        Self::new(PROTOCOL_VERSION, max_window, features)
    }

    /// Same version and features, different window.
    pub const fn window(self, max_window: u8) -> Self {
        Self::new(self.protocol_version, max_window, self.features)
    }

    /// Same version and window, different features.
    pub const fn with_features(self, features: Capabilities) -> Self {
        Self::new(self.protocol_version, self.max_window, features)
    }

    /// `ver | window | features_hi | features_lo`.
    pub const fn to_bytes(&self) -> [u8; SessionConfig::WIRE_LEN] {
        let f = self.features.bits().to_be_bytes();
        [self.protocol_version, self.max_window, f[0], f[1]]
    }

    /// Parses and validates: version must be nonzero, window in `1..=8`.
    pub fn from_bytes(bytes: [u8; SessionConfig::WIRE_LEN]) -> Result<Self, HandshakeError> {
        if bytes[0] == 0 {
            return Err(HandshakeError::Version);
        }
        if bytes[1] == 0 || bytes[1] as usize > MAX_WINDOW {
            return Err(HandshakeError::Window);
        }
        Ok(SessionConfig {
            protocol_version: bytes[0],
            max_window: bytes[1],
            features: Capabilities::from_bits(u16::from_be_bytes([bytes[2], bytes[3]])),
        })
    }

    /// Deterministic on both ends: min version, min window, intersection
    /// of features. No follow-up round needed.
    pub fn negotiate(&self, remote: &SessionConfig) -> SessionConfig {
        SessionConfig {
            protocol_version: core::cmp::min(self.protocol_version, remote.protocol_version),
            max_window: core::cmp::min(self.max_window, remote.max_window),
            features: self.features.common(remote.features),
        }
    }
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Where the session is in its life cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// No session. The initial state, and the state after a clean close.
    Disconnected,
    /// Our hello left; the peer's hello has not arrived yet.
    Connecting,
    /// Hellos crossed; DATA may flow under the negotiated config.
    Established,
    /// FINISH is in flight at the transport.
    Closing,
    /// The session was cancelled (ABORT at the transport).
    Aborted,
}

/// Why a hello was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandshakeError {
    /// The hello did not have exactly [`HELLO_LEN`] bytes.
    Length,
    /// The config carried version `0`.
    Version,
    /// The config carried a window outside `1..=8`.
    Window,
}

impl core::fmt::Display for HandshakeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            HandshakeError::Length => write!(f, "hello must be exactly {HELLO_LEN} bytes"),
            HandshakeError::Version => write!(f, "protocol version 0 is invalid"),
            HandshakeError::Window => write!(f, "window must be 1..=8"),
        }
    }
}

/// One logical link between two peers, above one PSICOSE session.
///
/// This type never touches a transport. It produces and consumes hello
/// bytes; the caller moves them with [`Pump`](crate::pump::Pump),
/// [`send_bytes`](crate::stream::send_bytes), or anything else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerSession {
    local: PeerId,
    config: SessionConfig,
    remote: Option<PeerId>,
    negotiated: Option<SessionConfig>,
    state: SessionState,
    stats: SessionStats,
}

impl PeerSession {
    /// A disconnected session for `local`, offering `config`.
    pub const fn new(local: PeerId, config: SessionConfig) -> Self {
        PeerSession {
            local,
            config,
            remote: None,
            negotiated: None,
            state: SessionState::Disconnected,
            stats: SessionStats::new(),
        }
    }

    /// Our identity.
    pub const fn local(&self) -> PeerId {
        self.local
    }

    /// The peer's identity, once a hello arrived.
    pub const fn remote(&self) -> Option<PeerId> {
        self.remote
    }

    /// Current life-cycle state.
    pub const fn state(&self) -> SessionState {
        self.state
    }

    /// The config both sides agreed on, once established.
    pub const fn negotiated(&self) -> Option<SessionConfig> {
        self.negotiated
    }

    /// Last transport counters recorded via [`PeerSession::record_stats`].
    pub const fn stats(&self) -> SessionStats {
        self.stats
    }

    /// Stores a copy of the transport counters (e.g. `pump.stats()`).
    pub fn record_stats(&mut self, stats: SessionStats) {
        self.stats = stats;
    }

    /// Our hello: `PeerId (8) | SessionConfig (4)`.
    pub fn hello_bytes(&self) -> [u8; HELLO_LEN] {
        let mut out = [0u8; HELLO_LEN];
        out[..PeerId::LEN].copy_from_slice(&self.local.as_bytes());
        out[PeerId::LEN..].copy_from_slice(&self.config.to_bytes());
        out
    }

    /// Starts (or restarts) a connection: forgets the previous peer and
    /// returns the hello to transmit. Send these bytes right after START.
    pub fn connect(&mut self) -> [u8; HELLO_LEN] {
        self.remote = None;
        self.negotiated = None;
        self.state = SessionState::Connecting;
        self.hello_bytes()
    }

    /// Feeds the peer's hello.
    ///
    /// Returns `Ok(Some(reply))` when we had not sent our hello yet (the
    /// accepting side must transmit the reply), `Ok(None)` when this
    /// completes a connect we initiated. Either way the session becomes
    /// [`SessionState::Established`] with the deterministic negotiation.
    pub fn on_hello(&mut self, bytes: &[u8]) -> Result<Option<[u8; HELLO_LEN]>, HandshakeError> {
        if bytes.len() != HELLO_LEN {
            return Err(HandshakeError::Length);
        }
        let mut id = [0u8; PeerId::LEN];
        id.copy_from_slice(&bytes[..PeerId::LEN]);
        let mut cfg = [0u8; SessionConfig::WIRE_LEN];
        cfg.copy_from_slice(&bytes[PeerId::LEN..]);
        let remote_cfg = SessionConfig::from_bytes(cfg)?;

        let reply = if self.state == SessionState::Connecting {
            None
        } else {
            Some(self.hello_bytes())
        };
        self.remote = Some(PeerId::new(id));
        self.negotiated = Some(self.config.negotiate(&remote_cfg));
        self.state = SessionState::Established;
        Ok(reply)
    }

    /// FINISH left at the transport; delivery of new DATA should stop.
    pub fn close(&mut self) {
        self.state = SessionState::Closing;
    }

    /// FINISH was ACKed; the session is cleanly over.
    pub fn closed(&mut self) {
        self.state = SessionState::Disconnected;
    }

    /// ABORT happened (sent or received). The endpoint stays reusable:
    /// call [`PeerSession::connect`] to open a new session.
    pub fn abort(&mut self) {
        self.state = SessionState::Aborted;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(window: u8, features: Capabilities) -> SessionConfig {
        SessionConfig::new(PROTOCOL_VERSION, window, features)
    }

    #[test]
    fn config_roundtrips_through_bytes() {
        let config = cfg(4, Capabilities::WINDOW.with(Capabilities::STREAM));
        assert_eq!(SessionConfig::from_bytes(config.to_bytes()), Ok(config));
    }

    #[test]
    fn config_rejects_bad_version_and_window() {
        assert_eq!(
            SessionConfig::from_bytes([0, 4, 0, 0]),
            Err(HandshakeError::Version)
        );
        assert_eq!(
            SessionConfig::from_bytes([1, 0, 0, 0]),
            Err(HandshakeError::Window)
        );
        assert_eq!(
            SessionConfig::from_bytes([1, 9, 0, 0]),
            Err(HandshakeError::Window)
        );
    }

    #[test]
    fn new_clamps_window_into_transport_bounds() {
        assert_eq!(cfg(0, Capabilities::NONE).max_window, 1);
        assert_eq!(cfg(200, Capabilities::NONE).max_window, 8);
    }

    #[test]
    fn negotiation_is_min_and_intersection() {
        let a = cfg(8, Capabilities::WINDOW.with(Capabilities::STREAM));
        let b = cfg(4, Capabilities::STREAM);
        let n = a.negotiate(&b);
        assert_eq!(n.max_window, 4);
        assert_eq!(n.features, Capabilities::STREAM);
        assert_eq!(a.negotiate(&b), b.negotiate(&a));
    }

    #[test]
    fn capabilities_or_is_union() {
        assert_eq!(
            Capabilities::STREAM | Capabilities::WINDOW,
            Capabilities::STREAM.with(Capabilities::WINDOW)
        );
        assert_eq!(
            (Capabilities::STREAM | Capabilities::WINDOW | Capabilities::FORUM)
                & Capabilities::STREAM,
            Capabilities::STREAM
        );
        assert_eq!(SessionConfig::DEFAULT.max_window, 8);
        assert_eq!(SessionConfig::DEFAULT.features, Capabilities::STREAM);
    }

    #[test]
    fn handshake_establishes_both_sides() {
        let mut a = PeerSession::new(
            PeerId::from_label(b"alice"),
            cfg(8, Capabilities::WINDOW.with(Capabilities::STREAM)),
        );
        let mut b = PeerSession::new(PeerId::from_label(b"bob"), cfg(4, Capabilities::STREAM));

        let hello_a = a.connect();
        assert_eq!(a.state(), SessionState::Connecting);

        let reply = b.on_hello(&hello_a);
        assert_eq!(reply, Ok(Some(b.hello_bytes())));
        assert_eq!(b.state(), SessionState::Established);
        assert_eq!(b.remote(), Some(PeerId::from_label(b"alice")));

        let Ok(Some(hello_b)) = reply else {
            return;
        };
        assert_eq!(a.on_hello(&hello_b), Ok(None));
        assert_eq!(a.state(), SessionState::Established);
        assert_eq!(a.remote(), Some(PeerId::from_label(b"bob")));

        let expected = cfg(4, Capabilities::STREAM);
        assert_eq!(a.negotiated(), Some(expected));
        assert_eq!(b.negotiated(), Some(expected));
    }

    #[test]
    fn hello_with_wrong_length_is_rejected() {
        let mut s = PeerSession::new(PeerId::from_label(b"solo"), cfg(1, Capabilities::NONE));
        assert_eq!(s.on_hello(&[0u8; 5]), Err(HandshakeError::Length));
        assert_eq!(s.state(), SessionState::Disconnected);
    }

    #[test]
    fn abort_leaves_the_session_reusable() {
        let mut a = PeerSession::new(PeerId::from_label(b"alice"), cfg(2, Capabilities::STREAM));
        let mut b = PeerSession::new(PeerId::from_label(b"bob"), cfg(2, Capabilities::STREAM));

        let hello = a.connect();
        assert!(matches!(b.on_hello(&hello), Ok(Some(_))));
        a.abort();
        assert_eq!(a.state(), SessionState::Aborted);

        let hello = a.connect();
        assert_eq!(a.state(), SessionState::Connecting);
        assert!(matches!(b.on_hello(&hello), Ok(Some(_))));
    }

    #[test]
    fn close_then_closed_returns_to_disconnected() {
        let mut s = PeerSession::new(PeerId::from_label(b"solo"), cfg(1, Capabilities::NONE));
        let _ = s.connect();
        s.close();
        assert_eq!(s.state(), SessionState::Closing);
        s.closed();
        assert_eq!(s.state(), SessionState::Disconnected);
    }
}

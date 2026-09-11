//! Node identity. Belongs to the session/P2P layer, never to the frame.

/// 64-bit peer identity.
///
/// ```text
/// Peer A
///   │
///   │ PeerId = 0x82A1…
///   ▼
/// PSICOSE session
/// ```
///
/// The 4-byte wire frame does not carry it: the hello exchange transmits
/// it as ordinary payload bytes, one DATA frame per byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PeerId {
    bytes: [u8; PeerId::LEN],
}

impl PeerId {
    /// Identity size on the wire, in payload bytes.
    pub const LEN: usize = 8;

    /// Builds an identity from raw bytes. How they are generated (random,
    /// hash of a key, serial number) is the application's business.
    pub const fn new(bytes: [u8; PeerId::LEN]) -> Self {
        PeerId { bytes }
    }

    /// The raw 8 bytes.
    pub const fn as_bytes(&self) -> [u8; PeerId::LEN] {
        self.bytes
    }
}

impl From<[u8; PeerId::LEN]> for PeerId {
    fn from(bytes: [u8; PeerId::LEN]) -> Self {
        PeerId::new(bytes)
    }
}

impl From<PeerId> for [u8; PeerId::LEN] {
    fn from(id: PeerId) -> Self {
        id.bytes
    }
}

impl core::fmt::Display for PeerId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let b = &self.bytes;
        write!(
            f,
            "{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_raw_bytes() {
        let id = PeerId::new([0x82, 0xA1, 0, 1, 2, 3, 4, 5]);
        assert_eq!(id.as_bytes(), [0x82, 0xA1, 0, 1, 2, 3, 4, 5]);
        assert_eq!(PeerId::new(id.as_bytes()), id);
        assert_eq!(PeerId::from(id.as_bytes()), id);
        assert_eq!(<[u8; 8]>::from(id), id.as_bytes());
    }

    #[test]
    fn distinct_bytes_are_distinct_peers() {
        assert_ne!(PeerId::new([0; 8]), PeerId::new([1, 0, 0, 0, 0, 0, 0, 0]));
    }
}

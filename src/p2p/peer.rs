//! Node identity. Belongs to the session/P2P layer, never to the frame.

/// 64-bit peer identity.
///
/// ```text
/// Peer A
///   │
///   │ PeerId = "alice"
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

    /// Pads `label` with zeros to 8 bytes. Truncates if longer.
    ///
    /// ```
    /// use psicose::PeerId;
    /// assert_eq!(PeerId::from_label(b"alice").as_bytes()[0], b'a');
    /// ```
    pub const fn from_label(label: &[u8]) -> Self {
        let mut bytes = [0u8; PeerId::LEN];
        let mut i = 0;
        while i < label.len() && i < PeerId::LEN {
            bytes[i] = label[i];
            i += 1;
        }
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
        if let Some(label) = label_bytes(&self.bytes) {
            return f.write_str(label);
        }
        let b = &self.bytes;
        write!(
            f,
            "{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]
        )
    }
}

fn label_bytes(bytes: &[u8; 8]) -> Option<&str> {
    let mut n = 0usize;
    while n < 8 && bytes[n] != 0 {
        let c = bytes[n];
        if !c.is_ascii_graphic() && c != b' ' {
            return None;
        }
        n += 1;
    }
    if n == 0 {
        return None;
    }
    let mut i = n;
    while i < 8 {
        if bytes[i] != 0 {
            return None;
        }
        i += 1;
    }
    core::str::from_utf8(&bytes[..n]).ok()
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

    #[test]
    fn from_label_pads_and_truncates() {
        let alice = PeerId::from_label(b"alice");
        assert_eq!(&alice.as_bytes()[..5], b"alice");
        assert_eq!(alice.as_bytes()[5], 0);
        assert_eq!(PeerId::from_label(b"toolongname").as_bytes(), *b"toolongn");
    }

    #[test]
    fn display_prints_label_or_hex() {
        let mut buf = [0u8; 16];
        let alice = PeerId::from_label(b"alice");
        let n = write_id(&alice, &mut buf);
        assert_eq!(&buf[..n], b"alice");

        let raw = PeerId::new([0x82, 0xA1, 0, 1, 2, 3, 4, 5]);
        let n = write_id(&raw, &mut buf);
        assert_eq!(&buf[..n], b"82A1000102030405");
    }

    fn write_id(id: &PeerId, buf: &mut [u8]) -> usize {
        use core::fmt::Write;
        struct Sink<'a> {
            buf: &'a mut [u8],
            n: usize,
        }
        impl Write for Sink<'_> {
            fn write_str(&mut self, s: &str) -> core::fmt::Result {
                let bytes = s.as_bytes();
                if self.n + bytes.len() > self.buf.len() {
                    return Err(core::fmt::Error);
                }
                self.buf[self.n..self.n + bytes.len()].copy_from_slice(bytes);
                self.n += bytes.len();
                Ok(())
            }
        }
        let mut sink = Sink { buf, n: 0 };
        let _ = write!(sink, "{id}");
        sink.n
    }
}

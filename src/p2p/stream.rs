//! Logical streams, message identity, and fragmentation.
//!
//! Do not confuse the counters:
//!
//! ```text
//! SEQ        u8   transport — which DATA frame is this
//! StreamId   u8   application — which conversation (control/forum/file…)
//! MessageId  u16  application — which message inside the stream
//! fragment   u16  application — which piece of that message
//! ```
//!
//! One link multiplexes many conversations without touching the frame:
//!
//! ```text
//!                  PEER
//!                   │
//!           ┌───────┼────────┐
//!           │       │        │
//!        stream0 stream1  stream2
//!           │       │        │
//!        control   forum    file
//! ```
//!
//! A forum post does not fit in one byte — and does not need to. The
//! [`Fragmenter`] cuts it into `header + chunk` packets; PSICOSE still
//! only ever sees the next byte.

/// Identifies a logical conversation over one link.
///
/// The mapping is application policy; only stream 0 is reserved here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct StreamId(u8);

impl StreamId {
    /// Stream 0: session control messages (hello, ping, credit…).
    pub const CONTROL: StreamId = StreamId(0);

    /// An application-defined stream.
    pub const fn new(id: u8) -> Self {
        StreamId(id)
    }

    /// The raw id.
    pub const fn raw(self) -> u8 {
        self.0
    }
}

impl From<u8> for StreamId {
    fn from(id: u8) -> Self {
        StreamId(id)
    }
}

/// Identifies one message inside a stream. Wraps mod 65536; the
/// transport `SEQ` (`u8`) is a different counter for a different problem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct MessageId(u16);

impl MessageId {
    /// A message id chosen by the application.
    pub const fn new(id: u16) -> Self {
        MessageId(id)
    }

    /// The raw id.
    pub const fn raw(self) -> u16 {
        self.0
    }

    /// The next id, wrapping mod 65536.
    pub const fn next(self) -> Self {
        MessageId(self.0.wrapping_add(1))
    }
}

impl From<u16> for MessageId {
    fn from(id: u16) -> Self {
        MessageId(id)
    }
}

/// Serialized size of a [`MessageHeader`], in payload bytes.
pub const HEADER_LEN: usize = 7;

const FLAG_LAST: u8 = 0x01;

/// Per-fragment header. Payload bytes, never part of the 4-byte frame.
///
/// ```text
/// ┌────────────┬────────────────┬──────────────┬───────────┬─────────┐
/// │ stream (1) │ message id (2) │ fragment (2) │ flags (1) │ len (1) │
/// └────────────┴────────────────┴──────────────┴───────────┴─────────┘
/// ```
///
/// `flags` bit 0 marks the last fragment. All other bits must be zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageHeader {
    /// Which conversation this fragment belongs to.
    pub stream: StreamId,
    /// Which message inside the stream.
    pub message: MessageId,
    /// Fragment index, starting at 0.
    pub fragment: u16,
    /// True on the final fragment of the message.
    pub last: bool,
    /// How many payload bytes follow this header.
    pub len: u8,
}

/// Why a serialized header was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderError {
    /// A reserved flag bit was set.
    Flags,
}

impl core::fmt::Display for HeaderError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            HeaderError::Flags => write!(f, "reserved header flag bits set"),
        }
    }
}

impl MessageHeader {
    /// Serializes to the 7-byte wire form (big-endian ids).
    pub const fn to_bytes(&self) -> [u8; HEADER_LEN] {
        let m = self.message.raw().to_be_bytes();
        let g = self.fragment.to_be_bytes();
        let flags = if self.last { FLAG_LAST } else { 0 };
        [self.stream.raw(), m[0], m[1], g[0], g[1], flags, self.len]
    }

    /// Parses the 7-byte wire form. Reserved flag bits are rejected.
    pub fn from_bytes(bytes: [u8; HEADER_LEN]) -> Result<Self, HeaderError> {
        if bytes[5] & !FLAG_LAST != 0 {
            return Err(HeaderError::Flags);
        }
        Ok(MessageHeader {
            stream: StreamId::new(bytes[0]),
            message: MessageId::new(u16::from_be_bytes([bytes[1], bytes[2]])),
            fragment: u16::from_be_bytes([bytes[3], bytes[4]]),
            last: bytes[5] & FLAG_LAST != 0,
            len: bytes[6],
        })
    }
}

/// Cuts a caller-owned message into `header + chunk` fragments.
///
/// Borrows the payload; owns nothing. A 4 GB message and an 11-byte
/// "hello world" go through the same iterator, one bounded chunk at a time.
pub struct Fragmenter<'a> {
    stream: StreamId,
    message: MessageId,
    rest: &'a [u8],
    fragment: u16,
    chunk: usize,
    done: bool,
}

impl<'a> Fragmenter<'a> {
    /// Fragments `payload` into chunks of at most `max_fragment` bytes
    /// (`0` is treated as `1`). An empty payload still yields exactly one
    /// empty, last fragment, so the receiver sees the message exist.
    pub const fn new(
        stream: StreamId,
        message: MessageId,
        payload: &'a [u8],
        max_fragment: u8,
    ) -> Self {
        Fragmenter {
            stream,
            message,
            rest: payload,
            fragment: 0,
            chunk: if max_fragment == 0 {
                1
            } else {
                max_fragment as usize
            },
            done: false,
        }
    }

    /// Bytes not yet emitted.
    pub const fn remaining(&self) -> usize {
        self.rest.len()
    }

    /// The next `(header, chunk)`, or `None` after the last fragment.
    pub fn next_fragment(&mut self) -> Option<(MessageHeader, &'a [u8])> {
        if self.done {
            return None;
        }
        let take = core::cmp::min(self.chunk, self.rest.len());
        let (chunk, rest) = self.rest.split_at(take);
        self.rest = rest;
        let last = rest.is_empty();
        self.done = last;
        let header = MessageHeader {
            stream: self.stream,
            message: self.message,
            fragment: self.fragment,
            last,
            len: take as u8,
        };
        self.fragment = self.fragment.wrapping_add(1);
        Some((header, chunk))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_roundtrips_through_bytes() {
        let header = MessageHeader {
            stream: StreamId::new(1),
            message: MessageId::new(42),
            fragment: 3,
            last: true,
            len: 11,
        };
        assert_eq!(MessageHeader::from_bytes(header.to_bytes()), Ok(header));
    }

    #[test]
    fn reserved_flag_bits_are_rejected() {
        let mut bytes = MessageHeader {
            stream: StreamId::CONTROL,
            message: MessageId::new(0),
            fragment: 0,
            last: false,
            len: 0,
        }
        .to_bytes();
        bytes[5] = 0x02;
        assert_eq!(MessageHeader::from_bytes(bytes), Err(HeaderError::Flags));
    }

    #[test]
    fn ola_mundo_fragments_into_three_pieces() {
        // "Holá mundo" — the 11 bytes from the design discussion.
        let post = [
            0x48, 0x6F, 0x6C, 0xC3, 0xA1, 0x20, 0x6D, 0x75, 0x6E, 0x64, 0x6F,
        ];
        let mut frag = Fragmenter::new(StreamId::new(1), MessageId::new(42), &post, 4);

        let mut rebuilt = [0u8; 16];
        let mut filled = 0usize;
        let mut count = 0u16;
        let mut saw_last = false;
        while let Some((header, chunk)) = frag.next_fragment() {
            assert_eq!(header.stream, StreamId::new(1));
            assert_eq!(header.message, MessageId::new(42));
            assert_eq!(header.fragment, count);
            assert_eq!(header.len as usize, chunk.len());
            rebuilt[filled..filled + chunk.len()].copy_from_slice(chunk);
            filled += chunk.len();
            count += 1;
            saw_last = header.last;
        }
        assert_eq!(count, 3);
        assert!(saw_last);
        assert_eq!(&rebuilt[..filled], &post);
    }

    #[test]
    fn empty_message_yields_one_empty_last_fragment() {
        let mut frag = Fragmenter::new(StreamId::CONTROL, MessageId::new(7), &[], 4);
        let first = frag.next_fragment();
        assert_eq!(
            first,
            Some((
                MessageHeader {
                    stream: StreamId::CONTROL,
                    message: MessageId::new(7),
                    fragment: 0,
                    last: true,
                    len: 0,
                },
                &[][..]
            ))
        );
        assert_eq!(frag.next_fragment(), None);
    }

    #[test]
    fn zero_chunk_is_treated_as_one_byte() {
        let mut frag = Fragmenter::new(StreamId::new(2), MessageId::new(1), &[0xAB, 0xCD], 0);
        let mut count = 0;
        while let Some((header, chunk)) = frag.next_fragment() {
            assert_eq!(chunk.len(), 1);
            assert_eq!(header.len, 1);
            count += 1;
        }
        assert_eq!(count, 2);
    }

    #[test]
    fn message_id_wraps_independently_of_seq() {
        assert_eq!(MessageId::new(65535).next(), MessageId::new(0));
    }
}

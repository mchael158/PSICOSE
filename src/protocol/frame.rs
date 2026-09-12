//! The PSICOSE-1B wire frame.
//!
//! ```text
//! ┌──────┬─────┬─────┬───────┐
//! │ TYPE │ SEQ │ DATA│  CRC  │
//! └──────┴─────┴─────┴───────┘
//!    1      1     1      1     = 4 bytes on the wire
//!                   ▲
//!                   └── application payload: exactly 8 bits
//! ```
//!
//! A [`Frame`] is **semantically** valid by construction: the only public
//! builders are [`Frame::data`], [`Frame::ack`], [`Frame::nack`],
//! [`Frame::start`], [`Frame::finish`], and [`Frame::abort`]. Control
//! frames always carry `DATA = 0`. [`Frame::from_bytes`] also rejects a CRC-valid frame that
//! breaks those rules (e.g. an ACK with `DATA = 0xFF` on the wire).
//! CRC-valid ≠ semantically valid.

use super::crc::crc8;

/// Application payload per DATA frame, in bytes. Always exactly 8 bits.
pub const PAYLOAD_LEN: usize = 1;

/// Total size in bytes of a serialized frame. Fixed and known at compile time.
pub const FRAME_LEN: usize = 4;

const _: [(); 1] = [(); PAYLOAD_LEN];
const _: [(); 4] = [(); FRAME_LEN];

/// The role a frame plays on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FrameType {
    /// Carries one payload byte.
    Data = 0x01,
    /// Positive acknowledgement of a given sequence number.
    Ack = 0x02,
    /// Negative acknowledgement: this sequence number was rejected.
    Nack = 0x03,
    /// Opens a session; receiver resets `expected_seq` to 0.
    Start = 0x04,
    /// Closes a session; must be ACKed before either side is done.
    Finish = 0x05,
    /// Cancels a session immediately. `SEQ = 0`, `DATA = 0`. Must be ACKed.
    Abort = 0x06,
}

impl FrameType {
    const fn from_u8(byte: u8) -> Option<Self> {
        match byte {
            0x01 => Some(FrameType::Data),
            0x02 => Some(FrameType::Ack),
            0x03 => Some(FrameType::Nack),
            0x04 => Some(FrameType::Start),
            0x05 => Some(FrameType::Finish),
            0x06 => Some(FrameType::Abort),
            _ => None,
        }
    }

    const fn is_control(self) -> bool {
        !matches!(self, FrameType::Data)
    }
}

/// Errors that can occur while decoding raw bytes into a [`Frame`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    /// The `TYPE` byte did not match any known [`FrameType`] variant.
    InvalidType(u8),
    /// The trailing `CRC` byte did not match the CRC computed over
    /// `TYPE || SEQ || DATA`.
    CrcMismatch {
        /// The CRC computed locally from the received bytes.
        expected: u8,
        /// The CRC byte actually present in the frame.
        got: u8,
    },
    /// CRC and TYPE were fine, but the frame broke the control-frame
    /// contract (`DATA` must be 0; `START`/`ABORT` seq must be 0).
    InvalidSemantics,
}

/// A single, fixed-size PSICOSE-1B frame.
///
/// Always valid by construction: no public constructor accepts an ACK
/// with a non-zero payload. Wire bytes go through [`Frame::from_bytes`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    frame_type: FrameType,
    seq: u8,
    data: u8,
}

impl Frame {
    const fn raw(frame_type: FrameType, seq: u8, data: u8) -> Self {
        Frame {
            frame_type,
            seq,
            data,
        }
    }

    /// One application payload byte at `seq`.
    pub const fn data(seq: u8, byte: u8) -> Self {
        Frame::raw(FrameType::Data, seq, byte)
    }

    /// ACK for `seq`. `DATA` is 0.
    pub const fn ack(seq: u8) -> Self {
        Frame::raw(FrameType::Ack, seq, 0)
    }

    /// NACK for `seq`. `DATA` is 0.
    pub const fn nack(seq: u8) -> Self {
        Frame::raw(FrameType::Nack, seq, 0)
    }

    /// Session start. `SEQ = 0`, `DATA = 0`.
    pub const fn start() -> Self {
        Frame::raw(FrameType::Start, 0, 0)
    }

    /// Session finish at `seq`. `DATA` is 0.
    pub const fn finish(seq: u8) -> Self {
        Frame::raw(FrameType::Finish, seq, 0)
    }

    /// Session abort. `SEQ = 0`, `DATA = 0`.
    pub const fn abort() -> Self {
        Frame::raw(FrameType::Abort, 0, 0)
    }

    /// The frame's type.
    pub const fn frame_type(&self) -> FrameType {
        self.frame_type
    }

    /// The frame's sequence number.
    pub const fn seq(&self) -> u8 {
        self.seq
    }

    /// The payload byte. `0x00` on every control frame.
    pub const fn payload(&self) -> u8 {
        self.data
    }

    /// Serializes the frame to its 4-byte wire representation.
    pub const fn to_bytes(&self) -> [u8; FRAME_LEN] {
        let header = [self.frame_type as u8, self.seq, self.data];
        let crc = crc8(&header);
        [header[0], header[1], header[2], crc]
    }

    const fn semantics_ok(frame_type: FrameType, seq: u8, data: u8) -> bool {
        if frame_type.is_control() && data != 0 {
            return false;
        }
        if matches!(frame_type, FrameType::Start | FrameType::Abort) && seq != 0 {
            return false;
        }
        true
    }

    /// Parses a 4-byte wire representation: TYPE, then CRC, then semantics.
    pub fn from_bytes(bytes: [u8; FRAME_LEN]) -> Result<Self, FrameError> {
        let frame_type = FrameType::from_u8(bytes[0]).ok_or(FrameError::InvalidType(bytes[0]))?;

        let expected = crc8(&bytes[..3]);
        let got = bytes[3];
        if expected != got {
            return Err(FrameError::CrcMismatch { expected, got });
        }

        let seq = bytes[1];
        let data = bytes[2];
        if !Self::semantics_ok(frame_type, seq, data) {
            return Err(FrameError::InvalidSemantics);
        }

        Ok(Frame::raw(frame_type, seq, data))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_data_frame() {
        let frame = Frame::data(42, b'A');
        assert_eq!(Frame::from_bytes(frame.to_bytes()), Ok(frame));
        assert_eq!(frame.payload(), b'A');
    }

    #[test]
    fn control_constructors_force_data_zero() {
        assert_eq!(Frame::ack(7).payload(), 0);
        assert_eq!(Frame::nack(7).payload(), 0);
        assert_eq!(Frame::start().payload(), 0);
        assert_eq!(Frame::start().seq(), 0);
        assert_eq!(Frame::finish(255).payload(), 0);
        assert_eq!(Frame::abort().payload(), 0);
        assert_eq!(Frame::abort().seq(), 0);
    }

    #[test]
    fn roundtrip_all_control_frames() {
        for frame in [
            Frame::ack(7),
            Frame::nack(7),
            Frame::start(),
            Frame::finish(255),
            Frame::abort(),
        ] {
            assert_eq!(Frame::from_bytes(frame.to_bytes()), Ok(frame));
        }
    }

    #[test]
    fn rejects_unknown_type_byte() {
        let mut bytes = Frame::data(1, 2).to_bytes();
        bytes[0] = 0x00;
        assert_eq!(Frame::from_bytes(bytes), Err(FrameError::InvalidType(0x00)));
    }

    #[test]
    fn rejects_corrupted_payload() {
        let mut bytes = Frame::data(1, 2).to_bytes();
        bytes[2] ^= 0xFF;
        assert!(matches!(
            Frame::from_bytes(bytes),
            Err(FrameError::CrcMismatch { .. })
        ));
    }

    #[test]
    fn rejects_ack_with_nonzero_data_even_if_crc_matches() {
        let header = [FrameType::Ack as u8, 42, 0xFF];
        let crc = crc8(&header);
        let bytes = [header[0], header[1], header[2], crc];
        assert_eq!(Frame::from_bytes(bytes), Err(FrameError::InvalidSemantics));
    }

    #[test]
    fn rejects_start_with_nonzero_seq() {
        let header = [FrameType::Start as u8, 1, 0];
        let crc = crc8(&header);
        let bytes = [header[0], header[1], header[2], crc];
        assert_eq!(Frame::from_bytes(bytes), Err(FrameError::InvalidSemantics));
    }

    #[test]
    fn rejects_abort_with_nonzero_seq() {
        let header = [FrameType::Abort as u8, 1, 0];
        let crc = crc8(&header);
        let bytes = [header[0], header[1], header[2], crc];
        assert_eq!(Frame::from_bytes(bytes), Err(FrameError::InvalidSemantics));
    }

    #[test]
    fn seq_and_data_span_full_u8_range() {
        for &seq in &[0u8, 1, 127, 128, 254, 255] {
            for &data in &[0u8, 1, 127, 128, 254, 255] {
                assert_eq!(
                    Frame::from_bytes(Frame::data(seq, data).to_bytes()),
                    Ok(Frame::data(seq, data))
                );
            }
        }
    }
}

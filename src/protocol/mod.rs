//! The wire-level building blocks of PSICOSE-1B: frames, CRC, and sequence
//! numbers. This module has no knowledge of transports or state machines —
//! it only knows how to turn `(type, seq, data)` into 4 trustworthy bytes
//! and back.

pub mod assembler;
pub mod crc;
pub mod frame;
pub mod outbuf;
pub mod sequence;

pub use assembler::FrameAssembler;
pub use crc::{crc8, CRC8_POLY};
pub use frame::{Frame, FrameError, FrameType, FRAME_LEN, PAYLOAD_LEN};
pub use outbuf::OutBuf;
pub use sequence::Sequence;

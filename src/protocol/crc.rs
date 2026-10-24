//! CRC-8 (poly 0x07, init 0x00, no reflection, no xor-out) — the same
//! polynomial used by CRC-8/SMBUS.
//!
//! A frame covers exactly three bytes. Folding each byte as two nibbles
//! through a 16-byte table (built from the bit step, so it cannot drift)
//! keeps that path to a straight shift/xor sequence. A 256-byte table
//! stays out: it does not pay for a 3-byte header.

/// The generator polynomial used by every CRC computation in this crate.
pub const CRC8_POLY: u8 = 0x07;

/// One bit of the polynomial. `hibit` is 0 or 1; its negation is the mask.
#[inline(always)]
const fn bit_step(crc: u8) -> u8 {
    let hibit = crc >> 7;
    crc.wrapping_shl(1) ^ (hibit.wrapping_neg() & CRC8_POLY)
}

/// Four bit-steps of a high nibble. Index `n` is that nibble in bits 7..4.
const NIBBLE: [u8; 16] = {
    let mut table = [0u8; 16];
    let mut n = 0;
    while n < 16 {
        let mut crc = (n as u8) << 4;
        let mut bit = 0;
        while bit < 4 {
            crc = bit_step(crc);
            bit += 1;
        }
        table[n] = crc;
        n += 1;
    }
    table
};

/// Fold the high nibble, then shift the low nibble into its place.
///
/// Held in `u32` so the index is a single `>> 4` of a zero-extended byte.
#[inline(always)]
const fn fold_nibble(crc: u32) -> u32 {
    let index = (crc >> 4) & 0x0F;
    ((crc << 4) & 0xFF) ^ (NIBBLE[index as usize] as u32)
}

/// One data byte. Two nibble folds, no per-bit branch.
#[inline(always)]
const fn fold_byte(crc: u32, byte: u8) -> u32 {
    fold_nibble(fold_nibble(crc ^ (byte as u32)))
}

/// Computes the CRC-8 checksum of `bytes`.
///
/// This is a `const fn` so frame constants (e.g. in tests or static frame
/// tables) can be built at compile time with zero runtime cost.
///
/// # Determinism
/// Pure function of its input. No global state, no undefined behavior on
/// any input length (including the empty slice, which returns `0`).
#[inline(never)]
pub const fn crc8(bytes: &[u8]) -> u8 {
    let mut crc: u32 = 0;
    let mut i = 0;
    while i < bytes.len() {
        crc = fold_byte(crc, bytes[i]);
        i += 1;
    }
    crc as u8
}

/// CRC of a frame header (`TYPE`, `SEQ`, `DATA`). No slice at the call site.
///
/// The body is the shared [`crc8`] loop. Inlining this only stores the
/// three bytes and calls that loop, so encode and decode do not each grow
/// a private copy.
#[inline(always)]
pub(crate) const fn crc8_header(type_byte: u8, seq: u8, data: u8) -> u8 {
    crc8(&[type_byte, seq, data])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_is_zero() {
        assert_eq!(crc8(&[]), 0x00);
    }

    #[test]
    fn is_deterministic() {
        let a = crc8(&[0x01, 0x2A, 0xAA]);
        let b = crc8(&[0x01, 0x2A, 0xAA]);
        assert_eq!(a, b);
    }

    #[test]
    fn detects_single_bit_flip() {
        let base = [0x01, 0x2A, 0xAA];
        let base_crc = crc8(&base);
        for byte_idx in 0..base.len() {
            for bit in 0..8u8 {
                let mut corrupted = base;
                corrupted[byte_idx] ^= 1 << bit;
                assert_ne!(
                    crc8(&corrupted),
                    base_crc,
                    "undetected single-bit error at byte {byte_idx}, bit {bit}"
                );
            }
        }
    }

    #[test]
    fn is_const_evaluable() {
        const CRC: u8 = crc8(&[0xDE, 0xAD, 0xBE]);
        assert_eq!(CRC, crc8(&[0xDE, 0xAD, 0xBE]));
    }

    #[test]
    fn differs_from_identity() {
        // Sanity: CRC of nonzero data should not be trivially the last byte
        // or a simple XOR, which would indicate a broken shift/poly.
        assert_ne!(crc8(&[0xFF, 0x00, 0x00]), 0xFF);
    }

    fn crc8_bits(bytes: &[u8]) -> u8 {
        let mut crc = 0u8;
        for &byte in bytes {
            crc ^= byte;
            let mut bit = 0;
            while bit < 8 {
                crc = bit_step(crc);
                bit += 1;
            }
        }
        crc
    }

    #[test]
    fn nibble_fold_matches_the_bit_step() {
        let mut byte = 0u16;
        while byte < 256 {
            let only = [byte as u8];
            assert_eq!(crc8(&only), crc8_bits(&only));
            byte += 1;
        }
        let header = [0x01u8, 0x2A, 0xAA];
        assert_eq!(crc8(&header), crc8_bits(&header));
        assert_eq!(crc8_header(0x01, 0x2A, 0xAA), crc8(&header));
    }
}

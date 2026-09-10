//! CRC-8 (poly 0x07, init 0x00, no reflection, no xor-out) — the same
//! polynomial used by CRC-8/SMBUS. Chosen over a lookup table on purpose:
//! PSICOSE-1B targets memory-constrained MCUs where 256 bytes of ROM table
//! is not free, and every frame is only 3 bytes wide anyway, so the
//! bit-by-bit cost is negligible per frame.

/// The generator polynomial used by every CRC computation in this crate.
pub const CRC8_POLY: u8 = 0x07;

/// Computes the CRC-8 checksum of `bytes`.
///
/// This is a `const fn` so frame constants (e.g. in tests or static frame
/// tables) can be built at compile time with zero runtime cost.
///
/// # Determinism
/// Pure function of its input. No global state, no lookup tables, no
/// undefined behavior on any input length (including the empty slice,
/// which returns `0`).
pub const fn crc8(bytes: &[u8]) -> u8 {
    let mut crc: u8 = 0x00;
    let mut i = 0;
    while i < bytes.len() {
        crc ^= bytes[i];
        let mut bit = 0;
        while bit < 8 {
            if crc & 0x80 != 0 {
                crc = (crc << 1) ^ CRC8_POLY;
            } else {
                crc <<= 1;
            }
            bit += 1;
        }
        i += 1;
    }
    crc
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
        assert_ne!(crc8(&[0xFF, 0x00, 0x00]), 0xFF ^ 0x00 ^ 0x00);
    }
}

//! Authenticated encryption **above** the transport (feature `aead`).
//!
//! ```text
//! application message
//!         │
//!    ChaCha20-Poly1305   ← this module (RFC 8439)
//!         │  ciphertext ‖ tag
//!         ▼
//!      PSICOSE           ← still 1 DATA byte per frame + CRC-8
//! ```
//!
//! CRC-8 on the 4-byte frame only detects accidental bit errors. It does
//! **not** authenticate. Use this module when the link may be adversarial.
//!
//! Stack-only: caller owns plaintext / ciphertext buffers. No heap.
//! Nonce must be unique per key for the lifetime of the key.

use chacha20poly1305::aead::{AeadInPlace, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Nonce, Tag};

/// ChaCha20 key length (bytes).
pub const KEY_LEN: usize = 32;
/// Poly1305 tag length (bytes).
pub const TAG_LEN: usize = 16;
/// IETF ChaCha20-Poly1305 nonce length (bytes).
pub const NONCE_LEN: usize = 12;

/// `plaintext_len + TAG_LEN`, or `None` on overflow.
pub const fn sealed_len(plaintext_len: usize) -> Option<usize> {
    plaintext_len.checked_add(TAG_LEN)
}

/// AEAD failure (wrong key/nonce/tag, or buffer too small).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AeadError {
    /// `out` / working buffer cannot hold ciphertext + tag (or plaintext).
    Buffer,
    /// Authentication failed or cipher rejected the inputs.
    Crypto,
}

impl core::fmt::Display for AeadError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            AeadError::Buffer => write!(f, "aead buffer too small"),
            AeadError::Crypto => write!(f, "aead authentication or cipher failure"),
        }
    }
}

/// Encrypt `buf` in place and write the 16-byte tag into `tag`.
///
/// `aad` is authenticated but not encrypted (may be empty).
pub fn seal(
    key: &[u8; KEY_LEN],
    nonce: &[u8; NONCE_LEN],
    aad: &[u8],
    buf: &mut [u8],
    tag: &mut [u8; TAG_LEN],
) -> Result<(), AeadError> {
    let cipher = ChaCha20Poly1305::new_from_slice(key).map_err(|_| AeadError::Crypto)?;
    let n = Nonce::from_slice(nonce);
    let t = cipher
        .encrypt_in_place_detached(n, aad, buf)
        .map_err(|_| AeadError::Crypto)?;
    tag.copy_from_slice(t.as_slice());
    Ok(())
}

/// Decrypt `buf` in place after verifying `tag`.
///
/// On failure, `buf` must be treated as untrusted junk.
pub fn open(
    key: &[u8; KEY_LEN],
    nonce: &[u8; NONCE_LEN],
    aad: &[u8],
    buf: &mut [u8],
    tag: &[u8; TAG_LEN],
) -> Result<(), AeadError> {
    let cipher = ChaCha20Poly1305::new_from_slice(key).map_err(|_| AeadError::Crypto)?;
    let n = Nonce::from_slice(nonce);
    let t = Tag::from_slice(tag);
    cipher
        .decrypt_in_place_detached(n, aad, buf, t)
        .map_err(|_| AeadError::Crypto)
}

/// Seal into `out`: `ciphertext ‖ tag`. Returns total bytes written.
///
/// `out.len()` must be at least `plaintext.len() + TAG_LEN`.
pub fn seal_to(
    key: &[u8; KEY_LEN],
    nonce: &[u8; NONCE_LEN],
    aad: &[u8],
    plaintext: &[u8],
    out: &mut [u8],
) -> Result<usize, AeadError> {
    let need = plaintext
        .len()
        .checked_add(TAG_LEN)
        .ok_or(AeadError::Buffer)?;
    if out.len() < need {
        return Err(AeadError::Buffer);
    }
    out[..plaintext.len()].copy_from_slice(plaintext);
    let (body, tag_slot) = out.split_at_mut(plaintext.len());
    let mut tag = [0u8; TAG_LEN];
    seal(key, nonce, aad, body, &mut tag)?;
    tag_slot[..TAG_LEN].copy_from_slice(&tag);
    Ok(need)
}

/// Open `ciphertext ‖ tag` from `input` into `out`. Returns plaintext length.
pub fn open_from(
    key: &[u8; KEY_LEN],
    nonce: &[u8; NONCE_LEN],
    aad: &[u8],
    input: &[u8],
    out: &mut [u8],
) -> Result<usize, AeadError> {
    if input.len() < TAG_LEN {
        return Err(AeadError::Crypto);
    }
    let pt_len = input.len() - TAG_LEN;
    if out.len() < pt_len {
        return Err(AeadError::Buffer);
    }
    out[..pt_len].copy_from_slice(&input[..pt_len]);
    let mut tag = [0u8; TAG_LEN];
    tag.copy_from_slice(&input[pt_len..]);
    open(key, nonce, aad, &mut out[..pt_len], &tag)?;
    Ok(pt_len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_detached() {
        let key = [0x11u8; KEY_LEN];
        let nonce = [0x22u8; NONCE_LEN];
        let mut buf = *b"psicose-aead";
        let mut tag = [0u8; TAG_LEN];
        assert_eq!(seal(&key, &nonce, b"aad", &mut buf, &mut tag), Ok(()));
        assert_ne!(&buf, b"psicose-aead");
        assert_eq!(open(&key, &nonce, b"aad", &mut buf, &tag), Ok(()));
        assert_eq!(&buf, b"psicose-aead");
    }

    #[test]
    fn tampered_tag_fails() {
        let key = [0x33u8; KEY_LEN];
        let nonce = [0x44u8; NONCE_LEN];
        let mut buf = *b"hello";
        let mut tag = [0u8; TAG_LEN];
        assert_eq!(seal(&key, &nonce, b"", &mut buf, &mut tag), Ok(()));
        tag[0] ^= 0x01;
        assert_eq!(
            open(&key, &nonce, b"", &mut buf, &tag),
            Err(AeadError::Crypto)
        );
    }

    #[test]
    fn seal_to_open_from() {
        let key = [0x55u8; KEY_LEN];
        let nonce = [0x66u8; NONCE_LEN];
        let pt = [0x01u8, 0x02, 0x03, 0x04];
        let mut packed = [0u8; 64];
        assert_eq!(sealed_len(pt.len()), Some(pt.len() + TAG_LEN));
        assert_eq!(
            seal_to(&key, &nonce, b"", &pt, &mut packed),
            Ok(pt.len() + TAG_LEN)
        );
        let n = pt.len() + TAG_LEN;
        let mut out = [0u8; 32];
        assert_eq!(
            open_from(&key, &nonce, b"", &packed[..n], &mut out),
            Ok(pt.len())
        );
        assert_eq!(&out[..pt.len()], &pt);
    }
}

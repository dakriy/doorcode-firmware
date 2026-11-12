//! The signature block the server appends to a firmware image it serves, as
//! `espsecure sign_data --version 1` writes it and ESP-IDF's signed-app
//! check reads it: the image, then a 4-byte little-endian version word
//! that is 0, then the 64-byte ECDSA P-256 signature `r || s`, each half
//! big-endian, of the SHA-256 of everything before the block. The key is the
//! raw public point `x || y`, each half big-endian, which the configuration
//! carries as hex. `docs/firmware-signing.md` records the layout.

use p256::ecdsa::signature::hazmat::PrehashVerifier;
use p256::ecdsa::{Signature, VerifyingKey};
use sha2::{Digest, Sha256};

/// The version word and the signature.
const BLOCK_BYTES: usize = 4 + 64;
/// Where the signature starts in the block.
const SIGNATURE_AT: usize = 4;
/// The SEC1 tag of an uncompressed point, which the raw key leaves out.
const UNCOMPRESSED_POINT: u8 = 4;
/// The hex digits of a raw key: two for each byte of `x || y`.
const KEY_DIGITS: usize = 128;

/// An image followed as it arrives: everything but the last bytes, which
/// may turn out to be the block, is hashed as the signed part.
#[derive(Default)]
pub(crate) struct SignedImage {
    signed: Sha256,
    /// The last bytes seen, at most a block's worth.
    tail: Vec<u8>,
}

impl SignedImage {
    /// Takes the next bytes of the image.
    pub(crate) fn feed(&mut self, bytes: &[u8]) {
        self.tail.extend_from_slice(bytes);
        let signed = self.tail.len().saturating_sub(BLOCK_BYTES);
        self.signed.update(&self.tail[..signed]);
        self.tail.drain(..signed);
    }

    /// Whether the image ends in a block of version 0 whose signature of
    /// the rest verifies with `key`.
    pub(crate) fn verifies(self, key: &VerifyingKey) -> bool {
        let Ok(block) = <[u8; BLOCK_BYTES]>::try_from(self.tail.as_slice()) else {
            return false;
        };
        let (version, signature) = block.split_at(SIGNATURE_AT);
        let digest = self.signed.finalize();
        Signature::from_slice(signature).is_ok_and(|signature| {
            version == [0; SIGNATURE_AT] && key.verify_prehash(&digest, &signature).is_ok()
        })
    }
}

/// The key that `hex`, the raw public point `x || y` in 128 hex digits,
/// spells, or `None` when it spells no point on the curve.
pub(crate) fn signing_key(hex: &str) -> Option<VerifyingKey> {
    if hex.len() != KEY_DIGITS {
        return None;
    }
    let digits = hex
        .chars()
        .map(|digit| digit.to_digit(16))
        .collect::<Option<Vec<u32>>>()?;
    let (pairs, _) = digits.as_chunks::<2>();
    let point: Vec<u8> = pairs
        .iter()
        .map(|[high, low]| (high * 16 + low) as u8)
        .collect();
    let sec1 = [&[UNCOMPRESSED_POINT][..], &point].concat();
    VerifyingKey::from_sec1_bytes(&sec1).ok()
}

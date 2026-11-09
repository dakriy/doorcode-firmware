//! The signature block a release appends to a firmware image, as
//! `espsecure sign_data --version 1` writes it and ESP-IDF's signed-app
//! check reads it: the image, then a 4-byte little-endian version word
//! that is 0, then the 64-byte ECDSA P-256 signature `r || s`, each half
//! big-endian, of the SHA-256 of everything before the block. The key is the
//! raw public point `x || y`, each half big-endian. `docs/firmware-signing.md`
//! records the layout.

use p256::ecdsa::signature::hazmat::PrehashVerifier;
use p256::ecdsa::{Signature, VerifyingKey};
use sha2::{Digest, Sha256};

/// The version word and the signature.
const BLOCK_BYTES: usize = 4 + 64;
/// Where the signature starts in the block.
const SIGNATURE_AT: usize = 4;
/// The SEC1 tag of an uncompressed point, which the raw key leaves out.
const UNCOMPRESSED_POINT: u8 = 4;

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
    /// the rest verifies with the raw public `key`.
    pub(crate) fn verifies(self, key: &[u8]) -> bool {
        let Ok(block) = <[u8; BLOCK_BYTES]>::try_from(self.tail.as_slice()) else {
            return false;
        };
        let (version, signature) = block.split_at(SIGNATURE_AT);
        let point = [&[UNCOMPRESSED_POINT][..], key].concat();
        let key = VerifyingKey::from_sec1_bytes(&point).ok();
        let signature = Signature::from_slice(signature).ok();
        let digest = self.signed.finalize();
        key.zip(signature).is_some_and(|(key, signature)| {
            version == [0; SIGNATURE_AT] && key.verify_prehash(&digest, &signature).is_ok()
        })
    }
}

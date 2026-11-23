//! Certificates as PEM text, as the configuration carries them, and as the
//! DER they encode, which takes a third less room in storage.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;

const BEGIN: &str = "-----BEGIN CERTIFICATE-----";
const END: &str = "-----END CERTIFICATE-----";
/// The base64 characters on each line of a block written out.
const LINE: usize = 64;

/// The DER of each certificate in `pem`, in order, when `pem` is one or
/// more certificate blocks with nothing but whitespace around them.
pub(crate) fn certificates(pem: &str) -> Option<Vec<Vec<u8>>> {
    let mut rest = pem.trim();
    let mut found = Vec::new();
    while !rest.is_empty() {
        let block = rest.strip_prefix(BEGIN)?;
        let (body, after) = block.split_once(END)?;
        let digits: String = body.split_whitespace().collect();
        found.push(STANDARD.decode(digits).ok()?);
        rest = after.trim_start();
    }
    match found.is_empty() {
        true => None,
        false => Some(found),
    }
}

/// `certificates` as PEM blocks, each line of base64 64 characters long
/// and every line ending in a newline.
pub(crate) fn text(certificates: &[Vec<u8>]) -> String {
    let mut pem = String::new();
    for der in certificates {
        pem.push_str(BEGIN);
        pem.push('\n');
        let digits = STANDARD.encode(der);
        for line in digits.as_bytes().chunks(LINE) {
            pem.push_str(&String::from_utf8_lossy(line));
            pem.push('\n');
        }
        pem.push_str(END);
        pem.push('\n');
    }
    pem
}

/// Whether `a` and `b` name the same certificates: the same DER in the same
/// order, however the text is laid out, or the same text when either is not
/// certificate blocks.
pub(crate) fn same(a: &str, b: &str) -> bool {
    match (certificates(a), certificates(b)) {
        (Some(a), Some(b)) => a == b,
        _ => a == b,
    }
}

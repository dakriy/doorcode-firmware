//! Certificate lists of the sizes real ones have, for tests that count
//! storage: PEM blocks around filler of each DER length. Nothing parses
//! the DER, so filler stands in for it.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;

/// PEM certificate blocks, one of DER length `lengths[i]` each, the bytes
/// of the `i`-th all `i + 1`, with 64 base64 characters to a line.
pub fn certificates(lengths: &[usize]) -> String {
    let mut pem = String::new();
    for (at, length) in lengths.iter().enumerate() {
        let der = vec![(at + 1) as u8; *length];
        pem.push_str("-----BEGIN CERTIFICATE-----\n");
        let digits = STANDARD.encode(der);
        for line in digits.as_bytes().chunks(64) {
            pem.push_str(&String::from_utf8_lossy(line));
            pem.push('\n');
        }
        pem.push_str("-----END CERTIFICATE-----\n");
    }
    pem
}

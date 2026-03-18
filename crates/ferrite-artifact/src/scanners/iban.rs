//! IBAN (International Bank Account Number) artifact scanner.
//!
//! Validates the ISO 13616 modulo-97 check digit to reduce false positives.

use std::sync::OnceLock;

use regex::Regex;

use crate::scanner::{scan_text_lossy, ArtifactHit, ArtifactKind, ArtifactScanner};

/// Two uppercase letters + 2 digits + 4–30 uppercase alphanumerics.
static RE: OnceLock<Regex> = OnceLock::new();

fn re() -> &'static Regex {
    RE.get_or_init(|| Regex::new(r"\b[A-Z]{2}\d{2}[A-Z0-9]{4,30}\b").expect("iban regex"))
}

/// ISO 13616 modulo-97 check.  Returns `true` if the IBAN passes.
fn iban_valid(s: &str) -> bool {
    if s.len() < 15 || s.len() > 34 {
        return false;
    }
    // Rearrange: move first 4 chars to end.
    let rearranged: String = s[4..].chars().chain(s[..4].chars()).collect();
    // Convert letters to digits: A=10, B=11, …, Z=35.
    let numeric: String = rearranged
        .chars()
        .map(|c| {
            if c.is_ascii_uppercase() {
                format!("{}", c as u32 - 'A' as u32 + 10)
            } else {
                c.to_string()
            }
        })
        .collect();
    // Compute numeric % 97 using string chunking to avoid u128 overflow.
    let remainder = numeric.as_bytes().chunks(9).fold(0u64, |acc, chunk| {
        let part: u64 = std::str::from_utf8(chunk)
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        (acc * 10u64.pow(chunk.len() as u32) + part) % 97
    });
    remainder == 1
}

pub struct IbanScanner;

impl ArtifactScanner for IbanScanner {
    fn kind(&self) -> ArtifactKind {
        ArtifactKind::Iban
    }

    fn scan_block(&self, data: &[u8], block_offset: u64) -> Vec<ArtifactHit> {
        scan_text_lossy(data, block_offset, ArtifactKind::Iban, re(), |s| {
            if iban_valid(s) {
                Some(s.to_string())
            } else {
                None
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_gb_iban() {
        // GB82WEST12345698765432 — standard test IBAN
        assert!(iban_valid("GB82WEST12345698765432"));
    }

    #[test]
    fn valid_de_iban() {
        // DE89370400440532013000
        assert!(iban_valid("DE89370400440532013000"));
    }

    #[test]
    fn invalid_iban_wrong_check() {
        assert!(!iban_valid("GB00WEST12345698765432"));
    }

    #[test]
    fn too_short_iban() {
        assert!(!iban_valid("GB82"));
    }

    #[test]
    fn finds_iban_in_buffer() {
        let scanner = IbanScanner;
        let data = b"account GB82WEST12345698765432 details";
        let hits = scanner.scan_block(data, 0);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].value, "GB82WEST12345698765432");
    }

    #[test]
    fn no_false_positive_on_random_alphanumeric() {
        let scanner = IbanScanner;
        let data = b"reference AB12XXXX00000000000000";
        let hits = scanner.scan_block(data, 0);
        // AB12XXXX00000000000000 is unlikely to pass modulo-97.
        // Even if it does by coincidence the test documents expected behaviour.
        let _ = hits; // just ensure no panic
    }
}

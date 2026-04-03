//! Credit card number artifact scanner with Luhn checksum validation.
//!
//! Detected card numbers are **masked** — only the last 4 digits are stored
//! in the `ArtifactHit` value.  No raw card numbers are ever retained.

use std::sync::OnceLock;

use regex::Regex;

use crate::scanner::{ArtifactHit, ArtifactKind, ArtifactScanner, Confidence};

/// Matches 13–19 consecutive ASCII digits (no separators — raw disk format).
static RE: OnceLock<Regex> = OnceLock::new();

fn re() -> &'static Regex {
    RE.get_or_init(|| Regex::new(r"\d{13,19}").expect("cc regex"))
}

/// Luhn algorithm — returns `true` if `digits` (ASCII `b'0'`..=`b'9'`) passes.
fn luhn_valid(digits: &[u8]) -> bool {
    let mut sum = 0u32;
    let mut double = false;
    for &b in digits.iter().rev() {
        let mut n = (b - b'0') as u32;
        if double {
            n *= 2;
            if n > 9 {
                n -= 9;
            }
        }
        sum += n;
        double = !double;
    }
    sum.is_multiple_of(10)
}

/// Mask a card number: keep only the last 4 digits.
fn mask(s: &str) -> String {
    let last4 = &s[s.len().saturating_sub(4)..];
    format!("****-****-****-{last4}")
}

/// Check whether the surrounding context of a match looks like printable text.
///
/// Examines up to 32 bytes before and after the match within `data`.  If
/// fewer than 50 % of those bytes are printable ASCII (0x20–0x7E), the match
/// is embedded in binary data and gets `Confidence::Low`.
fn context_is_printable(data: &[u8], match_start: usize, match_end: usize) -> bool {
    let ctx_start = match_start.saturating_sub(32);
    let ctx_end = (match_end + 32).min(data.len());
    let ctx = &data[ctx_start..ctx_end];
    if ctx.is_empty() {
        return true;
    }
    let printable = ctx.iter().filter(|&&b| (0x20..=0x7E).contains(&b)).count();
    printable * 2 >= ctx.len()
}

pub struct CreditCardScanner;

impl ArtifactScanner for CreditCardScanner {
    fn kind(&self) -> ArtifactKind {
        ArtifactKind::CreditCard
    }

    fn scan_block(&self, data: &[u8], block_offset: u64) -> Vec<ArtifactHit> {
        let text = String::from_utf8_lossy(data);
        re().find_iter(text.as_ref())
            .filter(|m| !m.as_str().contains('\u{FFFD}'))
            .filter_map(|m| {
                let s = m.as_str();
                let bytes = s.as_bytes();
                if luhn_valid(bytes) {
                    // Downgrade to Low when the digits appear in binary context —
                    // many binary formats contain Luhn-valid integer runs by chance.
                    let confidence = if context_is_printable(data, m.start(), m.end()) {
                        Confidence::High
                    } else {
                        Confidence::Low
                    };
                    Some(ArtifactHit {
                        kind: ArtifactKind::CreditCard,
                        byte_offset: block_offset + m.start() as u64,
                        value: mask(s),
                        confidence,
                    })
                } else {
                    None
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luhn_valid_visa_test_number() {
        // 4111111111111111 — well-known Luhn-valid test number
        assert!(luhn_valid(b"4111111111111111"));
    }

    #[test]
    fn luhn_invalid_modified_number() {
        // Change the last digit — should fail
        assert!(!luhn_valid(b"4111111111111112"));
    }

    #[test]
    fn luhn_valid_mastercard_test() {
        // 5500005555555559
        assert!(luhn_valid(b"5500005555555559"));
    }

    #[test]
    fn finds_luhn_valid_card_in_buffer() {
        let scanner = CreditCardScanner;
        let data = b"card: 4111111111111111 expiry 12/26";
        let hits = scanner.scan_block(data, 0);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].value, "****-****-****-1111");
        assert_eq!(hits[0].byte_offset, 6);
    }

    #[test]
    fn skips_luhn_invalid_digit_run() {
        let scanner = CreditCardScanner;
        let data = b"1234567890123456"; // not Luhn-valid
        let hits = scanner.scan_block(data, 0);
        assert!(hits.is_empty());
    }

    #[test]
    fn masking_preserves_last_four() {
        assert_eq!(mask("4111111111111111"), "****-****-****-1111");
        assert_eq!(mask("1234"), "****-****-****-1234");
    }
}

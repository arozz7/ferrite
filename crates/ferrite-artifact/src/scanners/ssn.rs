//! US Social Security Number (SSN) artifact scanner.

use std::sync::OnceLock;

use regex::Regex;

use crate::scanner::{scan_text_lossy, ArtifactHit, ArtifactKind, ArtifactScanner, Confidence};

static RE: OnceLock<Regex> = OnceLock::new();

fn re() -> &'static Regex {
    RE.get_or_init(|| {
        // Strict format: NNN-NN-NNNN with word boundaries.
        Regex::new(r"\b\d{3}-\d{2}-\d{4}\b").expect("ssn regex")
    })
}

pub struct SsnScanner;

impl ArtifactScanner for SsnScanner {
    fn kind(&self) -> ArtifactKind {
        ArtifactKind::Ssn
    }

    fn scan_block(&self, data: &[u8], block_offset: u64) -> Vec<ArtifactHit> {
        scan_text_lossy(data, block_offset, ArtifactKind::Ssn, re(), |s| {
            // Filter out obviously invalid SSNs: area 000 or 666, group 00, serial 0000.
            let parts: Vec<&str> = s.split('-').collect();
            if parts.len() == 3 {
                let area = parts[0];
                let group = parts[1];
                let serial = parts[2];
                if area == "000" || area == "666" || group == "00" || serial == "0000" {
                    return None;
                }
            }
            Some((s.to_string(), Confidence::High))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_ssn() {
        let scanner = SsnScanner;
        let data = b"SSN: 123-45-6789 on file";
        let hits = scanner.scan_block(data, 0);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].value, "123-45-6789");
    }

    #[test]
    fn filters_invalid_area_000() {
        let scanner = SsnScanner;
        let data = b"000-45-6789";
        let hits = scanner.scan_block(data, 0);
        assert!(hits.is_empty());
    }

    #[test]
    fn filters_invalid_area_666() {
        let scanner = SsnScanner;
        let data = b"666-45-6789";
        let hits = scanner.scan_block(data, 0);
        assert!(hits.is_empty());
    }

    #[test]
    fn filters_zero_group() {
        let scanner = SsnScanner;
        let data = b"123-00-6789";
        let hits = scanner.scan_block(data, 0);
        assert!(hits.is_empty());
    }

    #[test]
    fn no_false_positive_on_phone_number() {
        let scanner = SsnScanner;
        // Phone numbers like 555-867-5309 won't match because they're NNN-NNN-NNNN
        let data = b"call 555-867-5309 today";
        let hits = scanner.scan_block(data, 0);
        assert!(hits.is_empty());
    }
}

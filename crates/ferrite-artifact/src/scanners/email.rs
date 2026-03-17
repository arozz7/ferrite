//! Email-address artifact scanner.

use std::sync::OnceLock;

use regex::Regex;

use crate::scanner::{scan_text_lossy, ArtifactHit, ArtifactKind, ArtifactScanner};

static RE: OnceLock<Regex> = OnceLock::new();

fn re() -> &'static Regex {
    RE.get_or_init(|| {
        Regex::new(r"[a-zA-Z0-9._%+\-]{1,64}@[a-zA-Z0-9.\-]{1,253}\.[a-zA-Z]{2,10}")
            .expect("email regex")
    })
}

pub struct EmailScanner;

impl ArtifactScanner for EmailScanner {
    fn kind(&self) -> ArtifactKind {
        ArtifactKind::Email
    }

    fn scan_block(&self, data: &[u8], block_offset: u64) -> Vec<ArtifactHit> {
        scan_text_lossy(data, block_offset, ArtifactKind::Email, re(), |s| {
            // Basic sanity: must contain exactly one '@'.
            if s.chars().filter(|&c| c == '@').count() == 1 {
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
    fn finds_plain_email() {
        let scanner = EmailScanner;
        let data = b"contact us at alice@example.com for help";
        let hits = scanner.scan_block(data, 0);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].value, "alice@example.com");
        assert_eq!(hits[0].byte_offset, 14);
    }

    #[test]
    fn finds_multiple_emails() {
        let scanner = EmailScanner;
        let data = b"from: bob@test.org to: carol@corp.example.co.uk";
        let hits = scanner.scan_block(data, 0);
        let values: Vec<&str> = hits.iter().map(|h| h.value.as_str()).collect();
        assert!(values.contains(&"bob@test.org"));
        assert!(values.contains(&"carol@corp.example.co.uk"));
    }

    #[test]
    fn no_false_positive_on_plain_text() {
        let scanner = EmailScanner;
        let data = b"hello world no emails here just text";
        let hits = scanner.scan_block(data, 0);
        assert!(hits.is_empty());
    }

    #[test]
    fn block_offset_applied() {
        let scanner = EmailScanner;
        let data = b"x@y.com";
        let hits = scanner.scan_block(data, 1000);
        assert_eq!(hits[0].byte_offset, 1000);
    }
}

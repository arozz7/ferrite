//! URL artifact scanner (`http://` and `https://`).

use std::sync::OnceLock;

use regex::Regex;

use crate::scanner::{scan_text_lossy, ArtifactHit, ArtifactKind, ArtifactScanner};

static RE: OnceLock<Regex> = OnceLock::new();

fn re() -> &'static Regex {
    RE.get_or_init(|| {
        // Match http/https URLs up to 200 chars; stop at whitespace or common
        // string delimiters that indicate the URL has ended.
        Regex::new(r#"https?://[^\x00-\x20"'<>\[\]\\{}\|^`]{4,200}"#).expect("url regex")
    })
}

pub struct UrlScanner;

impl ArtifactScanner for UrlScanner {
    fn kind(&self) -> ArtifactKind {
        ArtifactKind::Url
    }

    fn scan_block(&self, data: &[u8], block_offset: u64) -> Vec<ArtifactHit> {
        scan_text_lossy(data, block_offset, ArtifactKind::Url, re(), |s| {
            // Strip common trailing punctuation that regex may have captured.
            let trimmed = s.trim_end_matches(['.', ',', ')', ';']);
            if trimmed.len() >= 11 {
                // Minimum valid URL: "http://a.bc"
                Some(trimmed.to_string())
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
    fn finds_https_url() {
        let scanner = UrlScanner;
        let data = b"visit https://example.com/path?q=1 for more";
        let hits = scanner.scan_block(data, 0);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].value.starts_with("https://example.com"));
    }

    #[test]
    fn finds_http_url() {
        let scanner = UrlScanner;
        let data = b"see http://old-site.org/page";
        let hits = scanner.scan_block(data, 0);
        assert!(!hits.is_empty());
        assert!(hits[0].value.starts_with("http://"));
    }

    #[test]
    fn no_false_positive() {
        let scanner = UrlScanner;
        let data = b"just some random words without any urls";
        let hits = scanner.scan_block(data, 0);
        assert!(hits.is_empty());
    }

    #[test]
    fn strips_trailing_period() {
        let scanner = UrlScanner;
        let data = b"See https://example.com.";
        let hits = scanner.scan_block(data, 0);
        assert!(!hits.is_empty());
        assert!(!hits[0].value.ends_with('.'));
    }
}

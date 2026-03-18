//! Windows absolute file path artifact scanner.

use std::sync::OnceLock;

use regex::Regex;

use crate::scanner::{scan_text_lossy, ArtifactHit, ArtifactKind, ArtifactScanner};

static RE: OnceLock<Regex> = OnceLock::new();

fn re() -> &'static Regex {
    RE.get_or_init(|| {
        // Drive letter + colon + backslash + at least 3 non-control, non-reserved chars.
        // Max 260 chars (Windows MAX_PATH).
        Regex::new(r#"[A-Za-z]:\\[^\x00-\x1f"*?<>|]{3,260}"#).expect("win_path regex")
    })
}

pub struct WinPathScanner;

impl ArtifactScanner for WinPathScanner {
    fn kind(&self) -> ArtifactKind {
        ArtifactKind::WindowsPath
    }

    fn scan_block(&self, data: &[u8], block_offset: u64) -> Vec<ArtifactHit> {
        scan_text_lossy(data, block_offset, ArtifactKind::WindowsPath, re(), |s| {
            // Trim trailing whitespace / null-like chars that leaked in.
            let trimmed = s.trim_end_matches(|c: char| c.is_whitespace() || c == '\0');
            if trimmed.len() >= 6 {
                // Minimum: "C:\abc"
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
    fn finds_windows_path() {
        let scanner = WinPathScanner;
        let data = b"opened C:\\Users\\alice\\Documents\\report.docx for edit";
        let hits = scanner.scan_block(data, 0);
        assert!(!hits.is_empty());
        assert!(hits[0].value.contains("Users"));
    }

    #[test]
    fn finds_system_path() {
        let scanner = WinPathScanner;
        let data = b"C:\\Windows\\System32\\cmd.exe executed";
        let hits = scanner.scan_block(data, 0);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].value.starts_with("C:\\Windows"));
    }

    #[test]
    fn no_false_positive_on_unix_path() {
        let scanner = WinPathScanner;
        let data = b"/usr/local/bin/ferrite was not found";
        let hits = scanner.scan_block(data, 0);
        assert!(hits.is_empty());
    }
}

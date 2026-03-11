//! Signature definitions and TOML loader.

use serde::Deserialize;

use crate::error::{CarveError, Result};

// ── Public types ──────────────────────────────────────────────────────────────

/// A single file-type signature: header magic bytes, optional footer, and a
/// maximum extraction window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    /// Human-readable label (e.g. `"JPEG Image"`).
    pub name: String,
    /// File extension without leading dot (e.g. `"jpg"`).
    pub extension: String,
    /// Header magic bytes that mark the start of the file.
    pub header: Vec<u8>,
    /// Optional footer magic bytes that mark the end.  Empty = no footer.
    pub footer: Vec<u8>,
    /// Maximum number of bytes to extract (caps the search window).
    pub max_size: u64,
}

/// Configuration passed to [`crate::Carver`].
#[derive(Debug, Clone)]
pub struct CarvingConfig {
    pub signatures: Vec<Signature>,
    /// How many bytes to read per scan chunk (default: 1 MiB).
    pub scan_chunk_size: usize,
}

impl Default for CarvingConfig {
    fn default() -> Self {
        Self {
            signatures: Vec::new(),
            scan_chunk_size: 1024 * 1024,
        }
    }
}

impl CarvingConfig {
    /// Parse a `signatures.toml`-format string and return a config.
    ///
    /// The TOML must contain an array of `[[signature]]` tables with the
    /// fields `name`, `extension`, `header`, `footer`, and `max_size`.
    pub fn from_toml_str(s: &str) -> Result<Self> {
        #[derive(Deserialize)]
        struct RawSig {
            name: String,
            extension: String,
            header: String,
            footer: String,
            max_size: u64,
        }

        #[derive(Deserialize)]
        struct Raw {
            signature: Vec<RawSig>,
        }

        let raw: Raw = toml::from_str(s)?;

        let signatures: Result<Vec<Signature>> = raw
            .signature
            .into_iter()
            .map(|r| {
                let footer = if r.footer.is_empty() {
                    Vec::new()
                } else {
                    parse_hex(&r.footer)?
                };
                Ok(Signature {
                    name: r.name,
                    extension: r.extension,
                    header: parse_hex(&r.header)?,
                    footer,
                    max_size: r.max_size,
                })
            })
            .collect();

        Ok(CarvingConfig {
            signatures: signatures?,
            ..Default::default()
        })
    }
}

// ── Hex parsing ───────────────────────────────────────────────────────────────

/// Parse a space-separated hex string (e.g. `"FF D8 FF"`) into bytes.
pub fn parse_hex(s: &str) -> Result<Vec<u8>> {
    s.split_whitespace()
        .map(|tok| {
            u8::from_str_radix(tok, 16)
                .map_err(|_| CarveError::InvalidSignature(format!("invalid hex byte: {tok}")))
        })
        .collect()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_basic() {
        assert_eq!(parse_hex("FF D8 FF").unwrap(), &[0xFF, 0xD8, 0xFF]);
        assert_eq!(parse_hex("00").unwrap(), &[0x00]);
        assert_eq!(parse_hex("").unwrap(), &[] as &[u8]);
    }

    #[test]
    fn parse_hex_rejects_invalid() {
        assert!(parse_hex("ZZ").is_err());
        assert!(parse_hex("FF GG").is_err());
    }

    #[test]
    fn load_toml_jpeg() {
        let toml = r#"
[[signature]]
name      = "JPEG Image"
extension = "jpg"
header    = "FF D8 FF"
footer    = "FF D9"
max_size  = 10485760
"#;
        let cfg = CarvingConfig::from_toml_str(toml).unwrap();
        assert_eq!(cfg.signatures.len(), 1);
        let sig = &cfg.signatures[0];
        assert_eq!(sig.name, "JPEG Image");
        assert_eq!(sig.extension, "jpg");
        assert_eq!(sig.header, &[0xFF, 0xD8, 0xFF]);
        assert_eq!(sig.footer, &[0xFF, 0xD9]);
        assert_eq!(sig.max_size, 10_485_760);
    }

    #[test]
    fn load_toml_no_footer() {
        let toml = r#"
[[signature]]
name      = "MP3 Audio"
extension = "mp3"
header    = "49 44 33"
footer    = ""
max_size  = 52428800
"#;
        let cfg = CarvingConfig::from_toml_str(toml).unwrap();
        assert!(cfg.signatures[0].footer.is_empty());
    }

    #[test]
    fn load_toml_multiple() {
        let toml = r#"
[[signature]]
name      = "A"
extension = "a"
header    = "AA"
footer    = ""
max_size  = 100

[[signature]]
name      = "B"
extension = "b"
header    = "BB CC"
footer    = "DD"
max_size  = 200
"#;
        let cfg = CarvingConfig::from_toml_str(toml).unwrap();
        assert_eq!(cfg.signatures.len(), 2);
        assert_eq!(cfg.signatures[1].header, &[0xBB, 0xCC]);
    }

    #[test]
    fn load_toml_invalid_hex_errors() {
        let toml = r#"
[[signature]]
name = "Bad" extension = "bad" header = "ZZ" footer = "" max_size = 100
"#;
        assert!(CarvingConfig::from_toml_str(toml).is_err());
    }
}

//! Signature definitions and TOML loader.

use serde::Deserialize;

use crate::error::{CarveError, Result};

// ── Public types ──────────────────────────────────────────────────────────────

/// Hints the extractor on how to derive the actual file size from the file
/// header, rather than always writing `max_size` bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SizeHint {
    /// Read a fixed-width integer at a known offset and add a constant.
    ///
    /// `total_size = parse(data[offset..offset+len]) + add`
    ///
    /// Used by RIFF-based formats (AVI, WAV) where bytes 4–7 store the
    /// payload length and the total file size = value + 8.
    /// Also used by BMP where bytes 2–5 store the total file size directly.
    Linear {
        /// Byte offset within the file where the size field starts.
        offset: usize,
        /// Width of the size field in bytes (2, 4, or 8).
        len: u8,
        /// `true` = little-endian, `false` = big-endian.
        little_endian: bool,
        /// Constant added to the parsed value to obtain the total file size.
        add: u64,
    },

    /// OLE2 Compound File Binary Format (legacy DOC / XLS / PPT).
    ///
    /// File size is derived from two header fields:
    /// - `uSectorShift` (u16 LE at offset 30): `sector_size = 1 << uSectorShift`
    /// - `csectFat`     (u32 LE at offset 44): number of FAT sectors
    ///
    /// `max_size = (csectFat × (sector_size / 4) + 1) × sector_size`
    ///
    /// This gives a tight upper bound: each FAT sector can reference
    /// `sector_size / 4` data sectors, so the result is the maximum possible
    /// file size for the number of FAT sectors actually present.
    Ole2,
}

/// A single file-type signature: header magic bytes (with optional wildcard
/// bytes), optional footer, a maximum extraction window, and an optional hint
/// for reading the true file size from an embedded field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    /// Human-readable label (e.g. `"JPEG Image"`).
    pub name: String,
    /// File extension without leading dot (e.g. `"jpg"`).
    pub extension: String,
    /// Header pattern.  `Some(b)` requires an exact byte match; `None` is a
    /// wildcard that matches any byte at that position (written as `??` in the
    /// TOML).
    pub header: Vec<Option<u8>>,
    /// Optional footer magic bytes that mark the end.  Empty = no footer.
    pub footer: Vec<u8>,
    /// Maximum number of bytes to extract (caps the search window).
    pub max_size: u64,
    /// If present, the extractor reads the actual file size from this field
    /// and uses `min(parsed + add, max_size)` as the extraction length.
    pub size_hint: Option<SizeHint>,
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
    /// The TOML must contain an array of `[[signature]]` tables.
    pub fn from_toml_str(s: &str) -> Result<Self> {
        #[derive(Deserialize)]
        struct RawSig {
            name: String,
            extension: String,
            header: String,
            footer: String,
            max_size: u64,
            // Optional size-hint fields (Linear variant).
            size_hint_offset: Option<usize>,
            size_hint_len: Option<u8>,
            size_hint_endian: Option<String>,
            size_hint_add: Option<u64>,
            // Named variant selector (e.g. "ole2" → SizeHint::Ole2).
            size_hint_kind: Option<String>,
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

                let size_hint = match r.size_hint_kind.as_deref() {
                    Some(k) if k.eq_ignore_ascii_case("ole2") => Some(SizeHint::Ole2),
                    _ => match (r.size_hint_offset, r.size_hint_len) {
                        (Some(offset), Some(len)) => Some(SizeHint::Linear {
                            offset,
                            len,
                            little_endian: r
                                .size_hint_endian
                                .as_deref()
                                .map(|e| e.eq_ignore_ascii_case("le"))
                                .unwrap_or(true),
                            add: r.size_hint_add.unwrap_or(0),
                        }),
                        _ => None,
                    },
                };

                Ok(Signature {
                    name: r.name,
                    extension: r.extension,
                    header: parse_hex_pattern(&r.header)?,
                    footer,
                    max_size: r.max_size,
                    size_hint,
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

/// Parse a space-separated hex string into exact bytes.
///
/// Used for footers and test helpers.  Does not accept `??` wildcards.
pub fn parse_hex(s: &str) -> Result<Vec<u8>> {
    s.split_whitespace()
        .map(|tok| {
            u8::from_str_radix(tok, 16)
                .map_err(|_| CarveError::InvalidSignature(format!("invalid hex byte: {tok}")))
        })
        .collect()
}

/// Parse a header pattern string where `??` denotes a wildcard byte.
///
/// Returns `None` for `??` tokens and `Some(byte)` for all others.
pub fn parse_hex_pattern(s: &str) -> Result<Vec<Option<u8>>> {
    s.split_whitespace()
        .map(|tok| {
            if tok == "??" {
                Ok(None)
            } else {
                u8::from_str_radix(tok, 16)
                    .map(Some)
                    .map_err(|_| CarveError::InvalidSignature(format!("invalid hex byte: {tok}")))
            }
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
    fn parse_hex_pattern_wildcards() {
        let p = parse_hex_pattern("52 49 46 46 ?? ?? ?? ?? 41 56 49 20").unwrap();
        assert_eq!(p[0], Some(0x52));
        assert_eq!(p[4], None);
        assert_eq!(p[5], None);
        assert_eq!(p[8], Some(0x41));
    }

    #[test]
    fn parse_hex_pattern_no_wildcards() {
        let p = parse_hex_pattern("FF D8 FF").unwrap();
        assert_eq!(p, vec![Some(0xFF), Some(0xD8), Some(0xFF)]);
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
        assert_eq!(sig.header, vec![Some(0xFF), Some(0xD8), Some(0xFF)]);
        assert_eq!(sig.footer, &[0xFF, 0xD9]);
        assert_eq!(sig.max_size, 10_485_760);
        assert!(sig.size_hint.is_none());
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
    fn load_toml_size_hint_linear() {
        let toml = r#"
[[signature]]
name             = "AVI Video (RIFF)"
extension        = "avi"
header           = "52 49 46 46 ?? ?? ?? ?? 41 56 49 20"
footer           = ""
max_size         = 2147483648
size_hint_offset = 4
size_hint_len    = 4
size_hint_endian = "le"
size_hint_add    = 8
"#;
        let cfg = CarvingConfig::from_toml_str(toml).unwrap();
        let sig = &cfg.signatures[0];
        assert_eq!(sig.header[0], Some(0x52));
        assert_eq!(sig.header[4], None); // wildcard
        assert_eq!(sig.header[8], Some(0x41));
        match sig.size_hint.as_ref().unwrap() {
            SizeHint::Linear { offset, len, little_endian, add } => {
                assert_eq!(*offset, 4);
                assert_eq!(*len, 4);
                assert!(little_endian);
                assert_eq!(*add, 8);
            }
            other => panic!("expected Linear, got {other:?}"),
        }
    }

    #[test]
    fn load_toml_size_hint_ole2() {
        let toml = r#"
[[signature]]
name           = "OLE2 Compound"
extension      = "ole"
header         = "D0 CF 11 E0 A1 B1 1A E1"
footer         = ""
max_size       = 524288000
size_hint_kind = "ole2"
"#;
        let cfg = CarvingConfig::from_toml_str(toml).unwrap();
        let sig = &cfg.signatures[0];
        assert_eq!(sig.size_hint, Some(SizeHint::Ole2));
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
        assert_eq!(cfg.signatures[1].header, vec![Some(0xBB), Some(0xCC)]);
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

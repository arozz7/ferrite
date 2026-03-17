//! User-defined carving signatures, persisted to `ferrite-user-signatures.toml`.
//!
//! Uses the same TOML schema as `config/signatures.toml`.  Loaded at startup
//! and merged as a "Custom" group in the carving signature panel.

use ferrite_carver::{parse_hex, parse_hex_pattern, CarvingConfig, Signature};

// ── Types ─────────────────────────────────────────────────────────────────────

/// A user-defined signature stored as raw strings for easy editing and
/// round-tripping through TOML.
#[derive(Debug, Clone, PartialEq)]
pub struct UserSigDef {
    /// Human-readable label.
    pub name: String,
    /// File extension without leading dot (e.g. `"dat"`).
    pub extension: String,
    /// Header magic bytes as space-separated uppercase hex (e.g. `"FF D8 FF"`).
    /// `??` tokens are wildcards.
    pub header: String,
    /// Footer bytes as space-separated uppercase hex.  Empty string = no footer.
    pub footer: String,
    /// Maximum extraction window in bytes.
    pub max_size: u64,
}

impl UserSigDef {
    /// Convert to a [`Signature`] that the carving engine can use.
    ///
    /// Returns `Err(message)` if the header is empty or contains invalid hex.
    pub fn to_signature(&self) -> Result<Signature, String> {
        if self.name.trim().is_empty() {
            return Err("name must not be empty".to_string());
        }
        let ext = self.extension.trim().trim_start_matches('.');
        if ext.is_empty() {
            return Err("extension must not be empty".to_string());
        }
        let header = parse_hex_pattern(&self.header)
            .map_err(|e| format!("invalid header: {e}"))?;
        if header.is_empty() {
            return Err("header must not be empty".to_string());
        }
        let footer = if self.footer.trim().is_empty() {
            Vec::new()
        } else {
            parse_hex(&self.footer).map_err(|e| format!("invalid footer: {e}"))?
        };
        if self.max_size == 0 {
            return Err("max_size must be greater than zero".to_string());
        }
        Ok(Signature {
            name: self.name.clone(),
            extension: ext.to_string(),
            header,
            footer,
            footer_last: false,
            max_size: self.max_size,
            size_hint: None,
            min_size: 0,
            pre_validate: None,
            header_offset: 0,
        })
    }
}

// ── Validation ────────────────────────────────────────────────────────────────

/// Returns `true` if `hex` is a non-empty space-separated hex pattern where
/// each token is either `??` (wildcard) or a 2-character hex byte.
pub fn validate_header(hex: &str) -> bool {
    let tokens: Vec<&str> = hex.split_whitespace().collect();
    !tokens.is_empty()
        && tokens
            .iter()
            .all(|t| *t == "??" || (t.len() == 2 && u8::from_str_radix(t, 16).is_ok()))
}

/// Returns `true` if `hex` is empty (no footer) or a valid space-separated
/// hex byte string with no wildcards.
pub fn validate_footer(hex: &str) -> bool {
    let s = hex.trim();
    s.is_empty()
        || s.split_whitespace()
            .all(|t| t.len() == 2 && u8::from_str_radix(t, 16).is_ok())
}

// ── Persistence ───────────────────────────────────────────────────────────────

/// Load user signatures from `path`.  Returns an empty list if the file does
/// not exist, is empty, or cannot be parsed.
pub fn load_user_sigs(path: &str) -> Vec<UserSigDef> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) if !c.trim().is_empty() => c,
        _ => return Vec::new(),
    };

    match CarvingConfig::from_toml_str(&content) {
        Ok(cfg) => cfg
            .signatures
            .into_iter()
            .map(|sig| {
                let header = sig
                    .header
                    .iter()
                    .map(|b| b.map_or("??".to_string(), |v| format!("{v:02X}")))
                    .collect::<Vec<_>>()
                    .join(" ");
                let footer = sig
                    .footer
                    .iter()
                    .map(|b| format!("{b:02X}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                UserSigDef {
                    name: sig.name,
                    extension: sig.extension,
                    header,
                    footer,
                    max_size: sig.max_size,
                }
            })
            .collect(),
        Err(e) => {
            tracing::warn!(?e, path, "failed to parse user signature file");
            Vec::new()
        }
    }
}

/// Serialize `sigs` to `signatures.toml`-compatible TOML and write to `path`.
pub fn save_user_sigs(path: &str, sigs: &[UserSigDef]) -> std::io::Result<()> {
    let mut out = String::from(
        "# Ferrite user-defined carving signatures.\n\
         # Same schema as config/signatures.toml.\n\
         # Edit manually or use the TUI (u key on the Carving screen).\n\n",
    );
    for sig in sigs {
        out.push_str("[[signature]]\n");
        out.push_str(&format!("name      = {:?}\n", sig.name));
        out.push_str(&format!("extension = {:?}\n", sig.extension));
        out.push_str(&format!("header    = {:?}\n", sig.header));
        out.push_str(&format!("footer    = {:?}\n", sig.footer));
        out.push_str(&format!("max_size  = {}\n", sig.max_size));
        out.push('\n');
    }
    std::fs::write(path, out)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_header_valid() {
        assert!(validate_header("FF D8 FF"));
        assert!(validate_header("FF D8 ?? FF"));
        assert!(validate_header("AA"));
        assert!(validate_header("00"));
    }

    #[test]
    fn validate_header_invalid() {
        assert!(!validate_header(""));
        assert!(!validate_header("   "));
        assert!(!validate_header("ZZ"));
        assert!(!validate_header("FF GG"));
        assert!(!validate_header("F")); // single nibble
        assert!(!validate_header("FFF")); // three nibbles
    }

    #[test]
    fn validate_footer_valid() {
        assert!(validate_footer(""));
        assert!(validate_footer("   "));
        assert!(validate_footer("FF D9"));
        assert!(validate_footer("25 25 45 4F 46"));
    }

    #[test]
    fn validate_footer_invalid() {
        assert!(!validate_footer("ZZ"));
        assert!(!validate_footer("FF GG"));
        assert!(!validate_footer("??")); // wildcards not allowed in footers
    }

    #[test]
    fn to_signature_valid() {
        let def = UserSigDef {
            name: "Test".to_string(),
            extension: "tst".to_string(),
            header: "AA BB CC".to_string(),
            footer: "DD EE".to_string(),
            max_size: 1_000_000,
        };
        let sig = def.to_signature().unwrap();
        assert_eq!(sig.name, "Test");
        assert_eq!(sig.extension, "tst");
        assert_eq!(sig.header, vec![Some(0xAA), Some(0xBB), Some(0xCC)]);
        assert_eq!(sig.footer, vec![0xDD, 0xEE]);
        assert_eq!(sig.max_size, 1_000_000);
        assert!(sig.size_hint.is_none());
        assert!(sig.pre_validate.is_none());
    }

    #[test]
    fn to_signature_strips_leading_dot_from_extension() {
        let def = UserSigDef {
            name: "Test".to_string(),
            extension: ".jpg".to_string(),
            header: "FF D8".to_string(),
            footer: "".to_string(),
            max_size: 1024,
        };
        assert_eq!(def.to_signature().unwrap().extension, "jpg");
    }

    #[test]
    fn to_signature_with_wildcard() {
        let def = UserSigDef {
            name: "Wildcard".to_string(),
            extension: "wc".to_string(),
            header: "AA ?? CC".to_string(),
            footer: "".to_string(),
            max_size: 512,
        };
        let sig = def.to_signature().unwrap();
        assert_eq!(sig.header[1], None);
        assert!(sig.footer.is_empty());
    }

    #[test]
    fn to_signature_empty_header_errors() {
        let def = UserSigDef {
            name: "Bad".to_string(),
            extension: "bad".to_string(),
            header: "".to_string(),
            footer: "".to_string(),
            max_size: 100,
        };
        assert!(def.to_signature().is_err());
    }

    #[test]
    fn to_signature_zero_max_size_errors() {
        let def = UserSigDef {
            name: "Bad".to_string(),
            extension: "bad".to_string(),
            header: "AA".to_string(),
            footer: "".to_string(),
            max_size: 0,
        };
        assert!(def.to_signature().is_err());
    }

    #[test]
    fn save_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("user-sigs.toml");
        let path_str = path.to_str().unwrap();

        let sigs = vec![
            UserSigDef {
                name: "Alpha".to_string(),
                extension: "alp".to_string(),
                header: "AA BB".to_string(),
                footer: "CC".to_string(),
                max_size: 1024,
            },
            UserSigDef {
                name: "Beta".to_string(),
                extension: "bet".to_string(),
                header: "11 22 ??".to_string(),
                footer: "".to_string(),
                max_size: 2048,
            },
        ];

        save_user_sigs(path_str, &sigs).unwrap();
        let loaded = load_user_sigs(path_str);

        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].name, "Alpha");
        assert_eq!(loaded[0].header, "AA BB");
        assert_eq!(loaded[0].footer, "CC");
        assert_eq!(loaded[1].name, "Beta");
        assert_eq!(loaded[1].header, "11 22 ??");
        assert_eq!(loaded[1].footer, "");
    }

    #[test]
    fn load_missing_file_returns_empty() {
        let sigs = load_user_sigs("/nonexistent/path/ferrite-user-signatures.toml");
        assert!(sigs.is_empty());
    }

    #[test]
    fn load_empty_file_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.toml");
        std::fs::write(&path, "").unwrap();
        let sigs = load_user_sigs(path.to_str().unwrap());
        assert!(sigs.is_empty());
    }
}

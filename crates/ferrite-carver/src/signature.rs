//! Signature definitions and TOML loader.

use serde::Deserialize;

use crate::error::{CarveError, Result};
use crate::pre_validate::PreValidate;

// ── Public types ──────────────────────────────────────────────────────────────

/// Hints the extractor on how to derive the actual file size from the file
/// header, rather than always writing `max_size` bytes.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
    Ole2,

    /// Read a fixed-width integer, multiply by a scale factor, then add a constant.
    ///
    /// `total_size = parse(data[offset..offset+len]) × scale + add`
    ///
    /// Used by Windows Event Log (EVTX) where offset 42 holds the chunk count
    /// (u16 LE), each chunk is exactly 65536 bytes, and the file header is
    /// 4096 bytes: `total = chunk_count × 65536 + 4096`.
    LinearScaled {
        offset: usize,
        len: u8,
        little_endian: bool,
        scale: u64,
        add: u64,
    },

    /// SQLite database file.
    ///
    /// File size is derived from two big-endian header fields:
    /// - `page_size`  (u16 BE at offset 16): bytes per page; value `1` encodes 65536.
    /// - `db_pages`   (u32 BE at offset 28): total pages in the database file.
    ///   A value of `0` means the field is not set (pre-3.7.0 databases);
    ///   the extractor falls back to `max_size` in that case.
    ///
    /// `total_size = page_size × db_pages`
    Sqlite,

    /// 7-Zip archive.
    ///
    /// The 32-byte start header contains two u64 LE fields that together
    /// describe where the encoded header ends:
    /// - `NextHeaderOffset` (u64 LE at offset 12): bytes from end of start header
    ///   to the encoded header.
    /// - `NextHeaderSize`   (u64 LE at offset 20): byte length of the encoded header.
    ///
    /// `total_size = 32 + NextHeaderOffset + NextHeaderSize`
    SevenZip,

    /// Ogg bitstream container (Ogg Vorbis, Ogg Opus, Ogg FLAC, …).
    ///
    /// The container uses a sequence of *pages*.  Each page begins with the
    /// four-byte capture pattern `OggS` and carries a `header_type_flag` byte
    /// (at page offset 5) where bit 2 (`0x04`) marks the last page of the
    /// logical bitstream.  Page size is derived by reading the segment table
    /// stored in the page header:
    ///
    /// ```text
    /// page_size = 27 + num_page_segments + sum(segment_table)
    /// ```
    ///
    /// The extractor walks pages forward from the file header until a page with
    /// `header_type_flag & 0x04` is found, then returns
    /// `page_start_offset - file_offset + page_size` as the total file size.
    /// Returns `None` if no EOS page is found within `max_size` bytes (falls
    /// back to writing `max_size` bytes, as before).
    OggStream,

    /// ISO Base Media File Format box walker (MP4, MOV, M4A, M4V, 3GP, …).
    ///
    /// ISOBMFF files are a sequence of *boxes* (atoms).  Each box begins with:
    ///
    /// ```text
    /// [0..4]  box_size  — u32 BE; 0 = extends to EOF, 1 = largesize follows
    /// [4..8]  box_type  — 4 printable-ASCII bytes
    /// [8..16] largesize — u64 BE (only present when box_size == 1)
    /// ```
    ///
    /// The walker sums sequential top-level box sizes from `file_offset` until
    /// it encounters a non-printable box type (sync lost), a size-0 box
    /// (EOF-extending, size unknowable), an invalid size, or a 2 000-box safety
    /// cap.  Returns `None` when no valid boxes are found (falls back to
    /// `max_size`).
    Isobmff,

    /// TIFF IFD chain walker (Sony ARW, Canon CR2, Panasonic RW2, standard TIFF).
    ///
    /// Determines true file size by walking the IFD chain and finding the
    /// maximum byte extent referenced by any external data pointer, strip/tile
    /// offset+bytecount pair, or SubIFD (tag `0x014A`) link.  Supports both
    /// little-endian (`II`) and big-endian (`MM`) byte orders, and the
    /// Panasonic RW2 variant magic (`0x55` instead of `0x2A`).
    Tiff,

    /// Fujifilm RAF size hint.
    ///
    /// The RAF header encodes two data extents at fixed big-endian u32 offsets:
    ///
    /// ```text
    /// offset  84: JPEG preview offset
    /// offset  88: JPEG preview length
    /// offset  92: CFA raw sensor data offset
    /// offset  96: CFA raw sensor data length
    /// ```
    ///
    /// Returns `max(jpeg_offset + jpeg_length, cfa_offset + cfa_length)`.
    Raf,

    /// MPEG Transport Stream / Blu-ray M2TS stream walker.
    ///
    /// Walks the stream packet-by-packet, checking for the TS sync byte
    /// (`0x47`) at `ts_offset` within each `stride`-byte packet.  Returns
    /// the byte length of the contiguous valid-packet run.  Walking stops
    /// as soon as 10 consecutive packets fail the sync-byte check.
    ///
    /// | Format | `ts_offset` | `stride` |
    /// |--------|-------------|----------|
    /// | TS     | 0           | 188      |
    /// | M2TS   | 4           | 192      |
    MpegTs {
        /// Byte position of the `0x47` sync byte within each packet.
        ts_offset: u8,
        /// Total packet size in bytes (188 for TS, 192 for M2TS).
        stride: u16,
    },

    /// Windows PE executable.
    ///
    /// Derives file size from the PE section table: walks all sections and
    /// returns `max(PointerToRawData + SizeOfRawData)` across all sections.
    Pe,

    /// ELF executable or shared library.
    ///
    /// Derives file size from section and program headers:
    /// `max(section_table_end, max(p_offset + p_filesz))`.
    /// Supports both 32-bit and 64-bit, LE and BE.
    Elf,

    /// RAR archive (version 4 or 5).
    ///
    /// Walks the block structure to find the end-of-archive marker.
    /// RAR4 uses fixed-width fields; RAR5 uses variable-length integers.
    Rar,

    /// EBML container (MKV / WebM).
    ///
    /// Reads the top-level Segment element size from the EBML header.
    /// Returns `None` for unknown-size segments (streaming encodes).
    Ebml,

    /// Text-boundary scanner for text-based formats (XML, etc.).
    ///
    /// Reads forward from `file_offset` and stops when a null byte or a
    /// sustained run of non-text bytes is encountered.  Returns the offset
    /// of the last text byte, rounded up to include the final line/tag.
    TextBound,

    /// TrueType / OpenType font table directory walker.
    ///
    /// Reads the font header's `numTables` (u16 BE @4), then walks the
    /// table directory (16 bytes per record) and returns the maximum
    /// `table_offset + table_length` across all entries.
    Ttf,

    /// PDF linearized-length reader.
    ///
    /// Reads the first ~256 bytes looking for `/Linearized` and `/L <n>`.
    /// Returns the declared file length for linearized PDFs; non-linearized
    /// PDFs return `None` and fall back to footer-based extraction.
    Pdf,
}

impl SizeHint {
    /// Returns a short label used in display/debug contexts.
    pub fn kind_name(&self) -> &'static str {
        match self {
            SizeHint::Linear { .. } => "linear",
            SizeHint::Ole2 => "ole2",
            SizeHint::LinearScaled { .. } => "linear_scaled",
            SizeHint::Sqlite => "sqlite",
            SizeHint::SevenZip => "seven_zip",
            SizeHint::OggStream => "ogg_stream",
            SizeHint::Isobmff => "mp4",
            SizeHint::Tiff => "tiff",
            SizeHint::Raf => "raf",
            SizeHint::MpegTs { .. } => "mpeg_ts",
            SizeHint::Pe => "pe",
            SizeHint::Elf => "elf",
            SizeHint::Rar => "rar",
            SizeHint::Ebml => "ebml",
            SizeHint::TextBound => "text_bound",
            SizeHint::Ttf => "ttf",
            SizeHint::Pdf => "pdf",
        }
    }
}

/// A single file-type signature: header magic bytes (with optional wildcard
/// bytes), optional footer, a maximum extraction window, and an optional hint
/// for reading the true file size from an embedded field.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
    /// When `true`, extraction uses the **last** occurrence of `footer` within
    /// the extraction window rather than the first.
    ///
    /// Use for formats like PDF that may contain the footer byte sequence
    /// inside binary streams, or that accumulate multiple EOF markers through
    /// incremental updates.
    pub footer_last: bool,
    /// Maximum number of bytes to extract (caps the search window).
    pub max_size: u64,
    /// If present, the extractor reads the actual file size from this field
    /// and uses `min(parsed + add, max_size)` as the extraction length.
    pub size_hint: Option<SizeHint>,
    /// Minimum extraction size in bytes (0 = disabled).  Hits where the
    /// extracted data is known to be smaller than this threshold are skipped.
    #[serde(default)]
    pub min_size: u64,
    /// Optional format-specific structural validator applied at scan time.
    ///
    /// When set, each candidate hit is passed to `pre_validate::is_valid`
    /// before being recorded.  Hits that fail are silently discarded, reducing
    /// false positives without affecting extraction logic.  Configured via
    /// `pre_validate = "<kind>"` in `signatures.toml`.
    #[serde(default)]
    pub pre_validate: Option<PreValidate>,
    /// Byte offset within the file where `header` magic appears.
    ///
    /// Most formats have their magic at byte 0 (default `0`).  Formats like
    /// ISO 9660 ("CD001" at 32769), DICOM ("DICM" at 128), and TAR ("ustar"
    /// at 257) carry their identifying magic at a non-zero position.  When
    /// `header_offset > 0` the scanner finds the magic at the usual position
    /// and then shifts the reported `CarveHit.byte_offset` back by this
    /// amount so extraction begins at the true file start.
    #[serde(default)]
    pub header_offset: u64,
    /// Minimum byte distance between consecutive hits of this signature (0 = disabled).
    ///
    /// Formats like MPEG Program Stream embed their magic (`00 00 01 BA`) at
    /// every pack boundary throughout the file — not just at the true start.
    /// Setting `min_hit_gap` to a value larger than the typical file size
    /// (e.g. 16 MiB for MPG) suppresses the flood of intra-file false hits
    /// while still detecting multiple distinct files that are far apart.
    #[serde(default)]
    pub min_hit_gap: u64,
    /// Cross-signature suppression group (optional).
    ///
    /// When two or more signatures share the same `suppress_group` string,
    /// a hit from any one of them advances the shared gap counter for that
    /// group.  This suppresses cross-format duplicates such as M2TS (4-byte
    /// timestamp prefix + 0x47) and TS (bare 0x47), where every M2TS packet
    /// also produces a spurious TS hit 4 bytes later.
    ///
    /// Signatures without a group use their own `name` as the key (existing
    /// behaviour, fully backward-compatible).
    #[serde(default)]
    pub suppress_group: Option<String>,
    /// Extra bytes to include after the footer match (default: 0).
    ///
    /// Some formats have a footer marker that identifies the start of a
    /// trailing record rather than the exact end of the file.  For example,
    /// ZIP's EOCD footer `PK\x05\x06` is followed by 18 bytes of fixed
    /// fields (disk number, central dir offset, entry count, comment length)
    /// plus a variable-length comment.  Setting `footer_extra = 18` ensures
    /// the essential EOCD metadata is included in the extracted file.
    #[serde(default)]
    pub footer_extra: usize,
}

/// Configuration passed to [`crate::Carver`].
#[derive(Debug, Clone)]
pub struct CarvingConfig {
    pub signatures: Vec<Signature>,
    /// How many bytes to read per scan chunk (default: 4 MiB).
    pub scan_chunk_size: usize,
    /// First byte to scan from (0 = beginning of device).
    pub start_byte: u64,
    /// Last byte to scan up to, exclusive (None = end of device).
    pub end_byte: Option<u64>,
}

impl Default for CarvingConfig {
    fn default() -> Self {
        Self {
            signatures: Vec::new(),
            scan_chunk_size: 4 * 1024 * 1024,
            start_byte: 0,
            end_byte: None,
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
            #[serde(default)]
            footer_last: bool,
            max_size: u64,
            #[serde(default)]
            min_size: u64,
            // Optional size-hint fields (Linear / LinearScaled variants).
            size_hint_offset: Option<usize>,
            size_hint_len: Option<u8>,
            size_hint_endian: Option<String>,
            size_hint_add: Option<u64>,
            size_hint_scale: Option<u64>,
            // Named variant selector (e.g. "ole2" → SizeHint::Ole2).
            size_hint_kind: Option<String>,
            // Named pre-validator (e.g. "zip").
            pre_validate: Option<String>,
            // Offset of the magic bytes within the file (0 for most formats).
            #[serde(default)]
            header_offset: u64,
            // Minimum gap between consecutive hits of this signature (0 = disabled).
            #[serde(default)]
            min_hit_gap: u64,
            // Cross-signature suppression group (None = use sig name as key).
            #[serde(default)]
            suppress_group: Option<String>,
            // MpegTs size-hint fields (ts_offset and stride).
            #[serde(default)]
            size_hint_ts_offset: Option<u8>,
            #[serde(default)]
            size_hint_stride: Option<u16>,
            // Extra bytes to include after footer match.
            #[serde(default)]
            footer_extra: usize,
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

                let le = r
                    .size_hint_endian
                    .as_deref()
                    .map(|e| e.eq_ignore_ascii_case("le"))
                    .unwrap_or(true);

                let size_hint = match r.size_hint_kind.as_deref() {
                    Some(k) if k.eq_ignore_ascii_case("ole2") => Some(SizeHint::Ole2),
                    Some(k) if k.eq_ignore_ascii_case("sqlite") => Some(SizeHint::Sqlite),
                    Some(k) if k.eq_ignore_ascii_case("seven_zip") => Some(SizeHint::SevenZip),
                    Some(k) if k.eq_ignore_ascii_case("ogg_stream") => Some(SizeHint::OggStream),
                    Some(k)
                        if k.eq_ignore_ascii_case("mp4") || k.eq_ignore_ascii_case("isobmff") =>
                    {
                        Some(SizeHint::Isobmff)
                    }
                    Some(k) if k.eq_ignore_ascii_case("tiff") => Some(SizeHint::Tiff),
                    Some(k) if k.eq_ignore_ascii_case("raf") => Some(SizeHint::Raf),
                    Some(k) if k.eq_ignore_ascii_case("pe") => Some(SizeHint::Pe),
                    Some(k) if k.eq_ignore_ascii_case("elf") => Some(SizeHint::Elf),
                    Some(k) if k.eq_ignore_ascii_case("rar") => Some(SizeHint::Rar),
                    Some(k) if k.eq_ignore_ascii_case("ebml") => Some(SizeHint::Ebml),
                    Some(k) if k.eq_ignore_ascii_case("text_bound") => Some(SizeHint::TextBound),
                    Some(k) if k.eq_ignore_ascii_case("ttf") => Some(SizeHint::Ttf),
                    Some(k) if k.eq_ignore_ascii_case("pdf") => Some(SizeHint::Pdf),
                    Some(k) if k.eq_ignore_ascii_case("mpeg_ts") => {
                        match (r.size_hint_ts_offset, r.size_hint_stride) {
                            (Some(ts_offset), Some(stride)) => {
                                Some(SizeHint::MpegTs { ts_offset, stride })
                            }
                            _ => None,
                        }
                    }
                    Some(k) if k.eq_ignore_ascii_case("linear_scaled") => {
                        match (r.size_hint_offset, r.size_hint_len, r.size_hint_scale) {
                            (Some(offset), Some(len), Some(scale)) => {
                                Some(SizeHint::LinearScaled {
                                    offset,
                                    len,
                                    little_endian: le,
                                    scale,
                                    add: r.size_hint_add.unwrap_or(0),
                                })
                            }
                            _ => None,
                        }
                    }
                    _ => match (r.size_hint_offset, r.size_hint_len) {
                        (Some(offset), Some(len)) => Some(SizeHint::Linear {
                            offset,
                            len,
                            little_endian: le,
                            add: r.size_hint_add.unwrap_or(0),
                        }),
                        _ => None,
                    },
                };

                let pre_validate = r.pre_validate.as_deref().and_then(PreValidate::from_kind);

                Ok(Signature {
                    name: r.name,
                    extension: r.extension,
                    header: parse_hex_pattern(&r.header)?,
                    footer,
                    footer_last: r.footer_last,
                    max_size: r.max_size,
                    size_hint,
                    min_size: r.min_size,
                    pre_validate,
                    header_offset: r.header_offset,
                    min_hit_gap: r.min_hit_gap,
                    suppress_group: r.suppress_group,
                    footer_extra: r.footer_extra,
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
            SizeHint::Linear {
                offset,
                len,
                little_endian,
                add,
            } => {
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
    fn load_toml_size_hint_sqlite() {
        let toml = r#"
[[signature]]
name           = "SQLite Database"
extension      = "db"
header         = "53 51 4C 69 74 65 20 66 6F 72 6D 61 74 20 33 00"
footer         = ""
max_size       = 10737418240
size_hint_kind = "sqlite"
"#;
        let cfg = CarvingConfig::from_toml_str(toml).unwrap();
        let sig = &cfg.signatures[0];
        assert_eq!(sig.size_hint, Some(SizeHint::Sqlite));
    }

    #[test]
    fn load_toml_size_hint_seven_zip() {
        let toml = r#"
[[signature]]
name           = "7-Zip Archive"
extension      = "7z"
header         = "37 7A BC AF 27 1C"
footer         = ""
max_size       = 524288000
size_hint_kind = "seven_zip"
"#;
        let cfg = CarvingConfig::from_toml_str(toml).unwrap();
        let sig = &cfg.signatures[0];
        assert_eq!(sig.size_hint, Some(SizeHint::SevenZip));
    }

    #[test]
    fn load_toml_size_hint_linear_scaled() {
        let toml = r#"
[[signature]]
name              = "Windows Event Log"
extension         = "evtx"
header            = "45 4C 46 49 4C 45 00"
footer            = ""
max_size          = 104857600
size_hint_kind    = "linear_scaled"
size_hint_offset  = 42
size_hint_len     = 2
size_hint_endian  = "le"
size_hint_scale   = 65536
size_hint_add     = 4096
"#;
        let cfg = CarvingConfig::from_toml_str(toml).unwrap();
        let sig = &cfg.signatures[0];
        match sig.size_hint.as_ref().unwrap() {
            SizeHint::LinearScaled {
                offset,
                len,
                little_endian,
                scale,
                add,
            } => {
                assert_eq!(*offset, 42);
                assert_eq!(*len, 2);
                assert!(little_endian);
                assert_eq!(*scale, 65536);
                assert_eq!(*add, 4096);
            }
            other => panic!("expected LinearScaled, got {other:?}"),
        }
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
    fn load_toml_footer_last_defaults_false() {
        let toml = r#"
[[signature]]
name      = "Test"
extension = "tst"
header    = "AA BB"
footer    = "CC DD"
max_size  = 1000
"#;
        let cfg = CarvingConfig::from_toml_str(toml).unwrap();
        assert!(
            !cfg.signatures[0].footer_last,
            "footer_last should default to false"
        );
    }

    #[test]
    fn load_toml_footer_last_explicit_true() {
        let toml = r#"
[[signature]]
name        = "PDF Document"
extension   = "pdf"
header      = "25 50 44 46"
footer      = "25 25 45 4F 46"
footer_last = true
max_size    = 104857600
"#;
        let cfg = CarvingConfig::from_toml_str(toml).unwrap();
        assert!(
            cfg.signatures[0].footer_last,
            "footer_last should be true when set"
        );
    }

    #[test]
    fn load_toml_size_hint_ogg_stream() {
        let toml = r#"
[[signature]]
name           = "OGG Media"
extension      = "ogg"
header         = "4F 67 67 53"
footer         = ""
max_size       = 2147483648
size_hint_kind = "ogg_stream"
"#;
        let cfg = CarvingConfig::from_toml_str(toml).unwrap();
        let sig = &cfg.signatures[0];
        assert_eq!(sig.size_hint, Some(SizeHint::OggStream));
    }

    #[test]
    fn load_toml_header_offset() {
        let toml = r#"
[[signature]]
name          = "ISO 9660"
extension     = "iso"
header        = "43 44 30 30 31"
footer        = ""
max_size      = 9395240960
header_offset = 32769
"#;
        let cfg = CarvingConfig::from_toml_str(toml).unwrap();
        let sig = &cfg.signatures[0];
        assert_eq!(sig.header_offset, 32769);
    }

    #[test]
    fn load_toml_header_offset_defaults_zero() {
        let toml = r#"
[[signature]]
name      = "JPEG Image"
extension = "jpg"
header    = "FF D8 FF"
footer    = "FF D9"
max_size  = 10485760
"#;
        let cfg = CarvingConfig::from_toml_str(toml).unwrap();
        assert_eq!(cfg.signatures[0].header_offset, 0);
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

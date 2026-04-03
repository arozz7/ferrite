use std::io::{BufRead, Write};
use std::path::Path;

use crate::error::{ImagingError, Result};
use crate::mapfile::{Block, BlockStatus, Mapfile};

/// Comment tag Ferrite writes to mark the CRC32 of the block-data lines.
/// GNU ddrescue ignores unknown comment lines, so the format stays compatible.
const CRC_TAG: &str = "# ferrite_crc32: ";

// ── Parse ─────────────────────────────────────────────────────────────────────

/// Parse a GNU ddrescue-compatible mapfile from a reader.
///
/// Format:
/// ```text
/// # Comment lines start with '#'
/// # First non-comment line: current_pos  current_status  current_pass
/// 0x00000000  ?  1
/// # pos         size       status
/// 0x00000000   0x40000000  ?
/// # ferrite_crc32: XXXXXXXX   ← optional integrity tag written by Ferrite
/// ```
///
/// If a `# ferrite_crc32:` tag is present, the CRC32 of the block-data lines
/// is validated.  An absent tag is silently accepted for compatibility with
/// GNU ddrescue and older Ferrite mapfiles.
pub fn parse(reader: impl std::io::Read, device_size: u64) -> Result<Mapfile> {
    let reader = std::io::BufReader::new(reader);
    let mut blocks: Vec<Block> = Vec::new();
    let mut header_seen = false;
    let mut declared_crc: Option<u32> = None;
    let mut hasher = crc32fast::Hasher::new();

    for (line_no, line) in reader.lines().enumerate() {
        let line = line.map_err(|e| ImagingError::MapfileIo {
            path: "<reader>".into(),
            source: e,
        })?;
        let line = line.trim();

        if line.is_empty() {
            continue;
        }

        // Check for our CRC tag before the generic comment skip.
        if let Some(hex) = line.strip_prefix(CRC_TAG) {
            declared_crc = u32::from_str_radix(hex.trim(), 16).ok();
            continue;
        }

        if line.starts_with('#') {
            continue;
        }

        let fields: Vec<&str> = line.split_whitespace().collect();

        if !header_seen {
            // First non-comment line: current_pos  current_status  [current_pass]
            if fields.len() < 2 {
                return Err(ImagingError::MapfileParse {
                    line: line_no + 1,
                    message: "header line needs at least 2 fields".into(),
                });
            }
            header_seen = true;
            // We don't store current_pos / current_pass — Ferrite always resumes
            // from the mapfile block states, not the ddrescue cursor.
            continue;
        }

        // Block line: pos  size  status_char
        if fields.len() < 3 {
            return Err(ImagingError::MapfileParse {
                line: line_no + 1,
                message: format!("block line needs 3 fields, got {}", fields.len()),
            });
        }

        // Hash the trimmed block line before parsing.
        hasher.update(line.as_bytes());

        let pos = parse_hex(fields[0], line_no + 1)?;
        let size = parse_hex(fields[1], line_no + 1)?;
        let ch = fields[2]
            .chars()
            .next()
            .ok_or_else(|| ImagingError::MapfileParse {
                line: line_no + 1,
                message: "empty status field".into(),
            })?;
        let status = BlockStatus::from_char(ch).ok_or_else(|| ImagingError::MapfileParse {
            line: line_no + 1,
            message: format!("unknown status char '{ch}'"),
        })?;

        blocks.push(Block { pos, size, status });
    }

    // Validate CRC when the tag is present.
    if let Some(declared) = declared_crc {
        let actual = hasher.finalize();
        if actual != declared {
            return Err(ImagingError::MapfileChecksum { declared, actual });
        }
    }

    Ok(Mapfile::from_blocks(blocks, device_size))
}

fn parse_hex(s: &str, line: usize) -> Result<u64> {
    let hex = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    u64::from_str_radix(hex, 16).map_err(|_| ImagingError::MapfileParse {
        line,
        message: format!("invalid hex value '{s}'"),
    })
}

// ── Serialize ─────────────────────────────────────────────────────────────────

/// Write a mapfile in GNU ddrescue-compatible format.
///
/// A `# ferrite_crc32: XXXXXXXX` tag is appended after the block-data lines
/// so that [`parse`] can detect corruption on the next load.
pub fn serialize(mapfile: &Mapfile, mut writer: impl Write) -> Result<()> {
    writeln!(writer, "# Mapfile. Created by Ferrite").map_err(io_err)?;
    writeln!(writer, "# current_pos  current_status  current_pass").map_err(io_err)?;
    // Use pos=0, status=?, pass=1 as the "current position" header.
    writeln!(writer, "0x{:016x}  ?  1", 0u64).map_err(io_err)?;
    writeln!(writer, "# pos              size             status").map_err(io_err)?;

    let mut hasher = crc32fast::Hasher::new();
    for block in mapfile.blocks() {
        let line = format!(
            "0x{:016x}  0x{:016x}  {}",
            block.pos,
            block.size,
            block.status.to_char(),
        );
        hasher.update(line.as_bytes());
        writeln!(writer, "{line}").map_err(io_err)?;
    }

    // Write the integrity tag as a trailing comment so GNU ddrescue ignores it.
    let checksum = hasher.finalize();
    writeln!(writer, "{CRC_TAG}{checksum:08x}").map_err(io_err)?;

    Ok(())
}

fn io_err(e: std::io::Error) -> ImagingError {
    ImagingError::MapfileIo {
        path: "<writer>".into(),
        source: e,
    }
}

// ── File helpers ──────────────────────────────────────────────────────────────

/// Load a mapfile from `path`, or create a fresh one if the file does not exist.
pub fn load_or_create(path: &Path, device_size: u64) -> Result<Mapfile> {
    match std::fs::File::open(path) {
        Ok(f) => parse(f, device_size),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Ok(Mapfile::from_device_size(device_size))
        }
        Err(e) => Err(ImagingError::MapfileIo {
            path: path.display().to_string(),
            source: e,
        }),
    }
}

/// Atomically save a mapfile: write to `<path>.tmp`, then rename.
pub fn save_atomic(mapfile: &Mapfile, path: &Path) -> Result<()> {
    let tmp = path.with_extension("tmp");
    {
        let f = std::fs::File::create(&tmp).map_err(|e| ImagingError::MapfileIo {
            path: tmp.display().to_string(),
            source: e,
        })?;
        let w = std::io::BufWriter::new(f);
        serialize(mapfile, w)?;
    }
    std::fs::rename(&tmp, path).map_err(|e| ImagingError::MapfileIo {
        path: path.display().to_string(),
        source: e,
    })
}

/// Count the number of 512-byte sectors recorded as unreadable in a mapfile.
///
/// "Unreadable" means `NonTrimmed`, `NonScraped`, or `BadSector` — statuses
/// that indicate the imaging engine could not recover good data for those
/// regions.
///
/// Returns `None` when the file does not exist or cannot be parsed (e.g. it
/// is still being written by the imaging engine).  Caller should treat `None`
/// as "data unavailable" rather than "no bad sectors".
pub fn count_unreadable_sectors(path: &Path) -> Option<u64> {
    let f = std::fs::File::open(path).ok()?;
    // device_size=0 is fine for counting purposes — from_blocks doesn't
    // need the real size to compute per-status byte counts.
    let mapfile = parse(f, 0).ok()?;
    let bad_bytes = mapfile.bytes_with_status(BlockStatus::NonTrimmed)
        + mapfile.bytes_with_status(BlockStatus::NonScraped)
        + mapfile.bytes_with_status(BlockStatus::BadSector);
    Some(bad_bytes / 512)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(mapfile: &Mapfile) -> Mapfile {
        let mut buf = Vec::new();
        serialize(mapfile, &mut buf).unwrap();
        let text = String::from_utf8(buf).unwrap();
        parse(text.as_bytes(), mapfile.device_size()).unwrap()
    }

    #[test]
    fn roundtrip_fresh_mapfile() {
        let original = Mapfile::from_device_size(4096);
        let parsed = roundtrip(&original);
        assert_eq!(parsed.blocks(), original.blocks());
    }

    #[test]
    fn roundtrip_multiblock_mapfile() {
        let mut m = Mapfile::from_device_size(5 * 512);
        m.update_range(0, 512, BlockStatus::Finished);
        m.update_range(512, 512, BlockStatus::BadSector);
        m.update_range(1024, 512, BlockStatus::NonScraped);
        m.update_range(1536, 512, BlockStatus::NonTrimmed);
        // [2048, 2560) stays NonTried
        let parsed = roundtrip(&m);
        assert_eq!(parsed.blocks(), m.blocks());
    }

    #[test]
    fn parse_ddrescue_reference_format() {
        // No ferrite_crc32 tag — must load without error (backwards compat).
        let input = "\
# Mapfile. Created by GNU ddrescue version 1.27\n\
# current_pos  current_status  current_pass\n\
0x00000000     ?               1\n\
# pos         size    status\n\
0x00000000  0x00000200  +\n\
0x00000200  0x00000200  -\n\
";
        let m = parse(input.as_bytes(), 1024).unwrap();
        assert_eq!(m.blocks().len(), 2);
        assert_eq!(m.blocks()[0].status, BlockStatus::Finished);
        assert_eq!(m.blocks()[1].status, BlockStatus::BadSector);
    }

    #[test]
    fn parse_ignores_comment_lines() {
        let input = "\
# this is a comment\n\
0x0  ?  1\n\
# another comment\n\
0x00000000  0x00000200  ?\n\
";
        let m = parse(input.as_bytes(), 512).unwrap();
        assert_eq!(m.blocks().len(), 1);
    }

    #[test]
    fn parse_rejects_unknown_status_char() {
        let input = "0x0  ?  1\n0x00000000  0x00000200  X\n";
        assert!(parse(input.as_bytes(), 512).is_err());
    }

    #[test]
    fn save_atomic_leaves_no_tmp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.mapfile");
        let m = Mapfile::from_device_size(512);
        save_atomic(&m, &path).unwrap();
        assert!(path.exists());
        assert!(!path.with_extension("tmp").exists());
    }

    #[test]
    fn crc_tag_is_written_and_validated() {
        let mut m = Mapfile::from_device_size(2 * 512);
        m.update_range(0, 512, BlockStatus::Finished);
        let mut buf = Vec::new();
        serialize(&m, &mut buf).unwrap();
        let text = String::from_utf8(buf).unwrap();
        // Tag must be present.
        assert!(
            text.contains("# ferrite_crc32: "),
            "CRC tag missing from serialized mapfile"
        );
        // Round-trip through parse must succeed.
        let parsed = parse(text.as_bytes(), m.device_size()).unwrap();
        assert_eq!(parsed.blocks(), m.blocks());
    }

    #[test]
    fn corrupted_block_line_is_detected() {
        let m = Mapfile::from_device_size(512);
        let mut buf = Vec::new();
        serialize(&m, &mut buf).unwrap();
        let mut text = String::from_utf8(buf).unwrap();

        // Flip the status char of the first block line to corrupt the content.
        text = text.replace("  ?", "  +");

        let result = parse(text.as_bytes(), m.device_size());
        assert!(
            matches!(result, Err(ImagingError::MapfileChecksum { .. })),
            "expected MapfileChecksum error on corrupted block line"
        );
    }

    #[test]
    fn absent_crc_tag_loads_without_error() {
        // Simulate a GNU ddrescue or pre-CRC Ferrite mapfile with no tag.
        let input = "0x0  ?  1\n0x00000000  0x00000200  ?\n";
        let result = parse(input.as_bytes(), 512);
        assert!(result.is_ok(), "mapfile without CRC tag must load cleanly");
    }
}

/// GPT (GUID Partition Table) parser.
///
/// GPT layout (primary copy):
///   - LBA 0  : Protective MBR
///   - LBA 1  : Primary GPT header (92 bytes, sector-padded to 512)
///   - LBA 2–N: Partition entries (128 bytes each, typically 128 entries)
///   - Last LBA: Backup GPT header
///
/// Header byte layout (offsets within the header sector):
///   0– 7: Signature "EFI PART"
///   8–11: Revision  0x00010000
///  12–15: Header size (≥ 92)
///  16–19: Header CRC32 (zeroed when computing)
///  20–23: Reserved (0)
///  24–31: MyLBA
///  32–39: AlternateLBA
///  40–47: FirstUsableLBA
///  48–55: LastUsableLBA
///  56–71: Disk GUID (mixed-endian)
///  72–79: Partition entry start LBA
///  80–83: Number of partition entries
///  84–87: Size of partition entry
///  88–91: Partition entry array CRC32
///
/// Partition entry byte layout (128 bytes):
///   0–15 : Type GUID (mixed-endian)
///  16–31 : Unique partition GUID (mixed-endian)
///  32–39 : First LBA
///  40–47 : Last LBA (inclusive)
///  48–55 : Attribute flags
///  56–127: Name (UTF-16LE, null-terminated, max 36 chars)
use byteorder::{ByteOrder, LittleEndian};
use uuid::Uuid;

use crate::error::{PartitionError, Result};
use crate::types::{PartitionEntry, PartitionKind, PartitionTable, PartitionTableKind};

const GPT_SIGNATURE: &[u8] = b"EFI PART";
const GPT_MIN_HEADER: usize = 92;
const GPT_ENTRY_MIN: usize = 128;

/// Parse a GPT partition table from raw sector data.
///
/// - `header_data`: raw bytes of the GPT header sector (LBA 1). Must be ≥ 92 bytes.
/// - `entries_data`: raw bytes of the partition entry array (starting at the LBA
///   recorded in the header). Must be ≥ `num_entries × entry_size` bytes.
pub fn parse(
    header_data: &[u8],
    entries_data: &[u8],
    disk_size_lba: u64,
    sector_size: u32,
) -> Result<PartitionTable> {
    if header_data.len() < GPT_MIN_HEADER {
        return Err(PartitionError::BufferTooSmall {
            needed: GPT_MIN_HEADER,
            got: header_data.len(),
        });
    }

    if &header_data[0..8] != GPT_SIGNATURE {
        return Err(PartitionError::InvalidSignature {
            context: "GPT",
            expected: "EFI PART",
            found: header_data[0..8].to_vec(),
        });
    }

    let header_size = LittleEndian::read_u32(&header_data[12..16]) as usize;
    if header_size < GPT_MIN_HEADER || header_size > header_data.len() {
        return Err(PartitionError::InvalidGptHeader(format!(
            "header size {header_size} out of range [92, {}]",
            header_data.len()
        )));
    }

    // ── Header CRC ───────────────────────────────────────────────────────────
    let stored_header_crc = LittleEndian::read_u32(&header_data[16..20]);
    let mut crc_buf = header_data[..header_size].to_vec();
    crc_buf[16..20].copy_from_slice(&[0u8; 4]); // zero out the CRC field
    let computed_header_crc = crc32fast::hash(&crc_buf);
    if computed_header_crc != stored_header_crc {
        return Err(PartitionError::CrcMismatch {
            expected: stored_header_crc,
            computed: computed_header_crc,
        });
    }

    // ── Entry metadata from header ────────────────────────────────────────────
    let num_entries = LittleEndian::read_u32(&header_data[80..84]) as usize;
    let entry_size = LittleEndian::read_u32(&header_data[84..88]) as usize;
    let stored_array_crc = LittleEndian::read_u32(&header_data[88..92]);

    if entry_size < GPT_ENTRY_MIN {
        return Err(PartitionError::InvalidGptHeader(format!(
            "entry size {entry_size} < minimum {GPT_ENTRY_MIN}"
        )));
    }

    // ── Partition array CRC ───────────────────────────────────────────────────
    let array_bytes = num_entries * entry_size;
    if entries_data.len() < array_bytes {
        return Err(PartitionError::BufferTooSmall {
            needed: array_bytes,
            got: entries_data.len(),
        });
    }
    let computed_array_crc = crc32fast::hash(&entries_data[..array_bytes]);
    if computed_array_crc != stored_array_crc {
        return Err(PartitionError::CrcMismatch {
            expected: stored_array_crc,
            computed: computed_array_crc,
        });
    }

    // ── Parse entries ─────────────────────────────────────────────────────────
    let mut entries = Vec::new();
    for i in 0..num_entries {
        let base = i * entry_size;
        let e = &entries_data[base..base + entry_size];

        let type_bytes: [u8; 16] = e[0..16].try_into().unwrap();
        let type_guid = Uuid::from_bytes_le(type_bytes);

        // Empty entry — nil type GUID
        if type_guid.is_nil() {
            continue;
        }

        let part_bytes: [u8; 16] = e[16..32].try_into().unwrap();
        let part_guid = Uuid::from_bytes_le(part_bytes);

        let start_lba = LittleEndian::read_u64(&e[32..40]);
        let end_lba = LittleEndian::read_u64(&e[40..48]);

        if start_lba > end_lba {
            continue;
        }

        let name = parse_utf16le_name(&e[56..entry_size.min(128)]);

        entries.push(PartitionEntry {
            index: i as u32,
            start_lba,
            end_lba,
            size_lba: end_lba - start_lba + 1,
            name: if name.is_empty() { None } else { Some(name) },
            kind: PartitionKind::Gpt {
                type_guid,
                part_guid,
            },
            bootable: false,
        });
    }

    Ok(PartitionTable {
        kind: PartitionTableKind::Gpt,
        sector_size,
        disk_size_lba,
        entries,
        note: None,
    })
}

fn parse_utf16le_name(bytes: &[u8]) -> String {
    let words: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .take_while(|&w| w != 0)
        .collect();
    String::from_utf16_lossy(&words).to_owned()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Known-good GPT header with zero entries (minimal valid structure).
    ///
    /// Header fields:
    ///   - Signature     : "EFI PART"
    ///   - Revision      : 0x00010000
    ///   - Header size   : 92
    ///   - My LBA        : 1
    ///   - Alternate LBA : 2047
    ///   - First usable  : 34
    ///   - Last usable   : 2014
    ///   - Disk GUID     : all zeros (nil)
    ///   - Entry start   : 2
    ///   - Num entries   : 0
    ///   - Entry size    : 128
    ///   - Array CRC32   : CRC32("") = 0x00000000
    ///
    /// The header CRC is computed and embedded at build time via `build_header`.
    fn build_header() -> Vec<u8> {
        let mut h = vec![0u8; 512];
        h[0..8].copy_from_slice(b"EFI PART");
        h[8..12].copy_from_slice(&[0x00, 0x00, 0x01, 0x00]); // revision 1.0
        LittleEndian::write_u32(&mut h[12..16], 92); // header size
        LittleEndian::write_u32(&mut h[16..20], 0); // CRC placeholder
        LittleEndian::write_u64(&mut h[24..32], 1); // MyLBA
        LittleEndian::write_u64(&mut h[32..40], 2047); // AlternateLBA
        LittleEndian::write_u64(&mut h[40..48], 34); // FirstUsable
        LittleEndian::write_u64(&mut h[48..56], 2014); // LastUsable
        LittleEndian::write_u64(&mut h[72..80], 2); // entry start LBA
        LittleEndian::write_u32(&mut h[80..84], 0); // num entries
        LittleEndian::write_u32(&mut h[84..88], 128); // entry size
        LittleEndian::write_u32(&mut h[88..92], 0); // array CRC (empty = 0)

        // Compute and insert header CRC
        let crc = crc32fast::hash(&h[..92]);
        LittleEndian::write_u32(&mut h[16..20], crc);
        h
    }

    /// Build a single 128-byte partition entry.
    fn build_entry(type_guid: Uuid, part_guid: Uuid, start: u64, end: u64, name: &str) -> Vec<u8> {
        let mut e = vec![0u8; 128];
        e[0..16].copy_from_slice(&type_guid.to_bytes_le());
        e[16..32].copy_from_slice(&part_guid.to_bytes_le());
        LittleEndian::write_u64(&mut e[32..40], start);
        LittleEndian::write_u64(&mut e[40..48], end);
        // Name in UTF-16LE
        for (i, c) in name.encode_utf16().enumerate() {
            if 56 + i * 2 + 1 >= 128 {
                break;
            }
            e[56 + i * 2] = c as u8;
            e[56 + i * 2 + 1] = (c >> 8) as u8;
        }
        e
    }

    /// Build a header with N entries and correct CRCs.
    fn build_header_with_entries(entries_data: &[u8], num_entries: u32) -> Vec<u8> {
        let array_crc = crc32fast::hash(entries_data);
        let mut h = build_header();
        LittleEndian::write_u32(&mut h[16..20], 0); // clear previous CRC
        LittleEndian::write_u32(&mut h[80..84], num_entries);
        LittleEndian::write_u32(&mut h[88..92], array_crc);
        let crc = crc32fast::hash(&h[..92]);
        LittleEndian::write_u32(&mut h[16..20], crc);
        h
    }

    // ── Microsoft Basic Data GUID: EBD0A0A2-B9E5-4433-87C0-68B6B72699C7 ──────
    fn ms_basic_data() -> Uuid {
        Uuid::parse_str("EBD0A0A2-B9E5-4433-87C0-68B6B72699C7").unwrap()
    }

    #[test]
    fn parse_empty_table() {
        let header = build_header();
        let table = parse(&header, &[], 2048, 512).unwrap();
        assert_eq!(table.kind, PartitionTableKind::Gpt);
        assert!(table.entries.is_empty());
    }

    #[test]
    fn parse_single_entry() {
        let type_guid = ms_basic_data();
        let part_guid = Uuid::new_v4();
        let entry_bytes = build_entry(type_guid, part_guid, 2048, 206847, "Basic data");
        let crc = crc32fast::hash(&entry_bytes);

        let mut header = build_header();
        LittleEndian::write_u32(&mut header[16..20], 0);
        LittleEndian::write_u32(&mut header[80..84], 1);
        LittleEndian::write_u32(&mut header[88..92], crc);
        let hcrc = crc32fast::hash(&header[..92]);
        LittleEndian::write_u32(&mut header[16..20], hcrc);

        let table = parse(&header, &entry_bytes, 204800, 512).unwrap();
        assert_eq!(table.entries.len(), 1);

        let e = &table.entries[0];
        assert_eq!(e.start_lba, 2048);
        assert_eq!(e.end_lba, 206847);
        assert_eq!(e.size_lba, 206847 - 2048 + 1);
        assert_eq!(e.name.as_deref(), Some("Basic data"));
        assert_eq!(
            e.kind,
            PartitionKind::Gpt {
                type_guid,
                part_guid
            }
        );
    }

    #[test]
    fn nil_entries_are_skipped() {
        let nil_entry = vec![0u8; 128]; // nil type GUID → empty
        let header = build_header_with_entries(&nil_entry, 1);
        let table = parse(&header, &nil_entry, 2048, 512).unwrap();
        assert!(table.entries.is_empty());
    }

    #[test]
    fn wrong_signature_returns_error() {
        let mut header = build_header();
        header[0..8].copy_from_slice(b"NOT PART");
        // Recompute CRC to keep that check from firing first
        let crc = crc32fast::hash(&header[..92]);
        LittleEndian::write_u32(&mut header[16..20], crc);
        let err = parse(&header, &[], 2048, 512).unwrap_err();
        assert!(matches!(
            err,
            PartitionError::InvalidSignature { context: "GPT", .. }
        ));
    }

    #[test]
    fn header_crc_mismatch_returns_error() {
        let mut header = build_header();
        header[16..20].copy_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
        let err = parse(&header, &[], 2048, 512).unwrap_err();
        assert!(matches!(err, PartitionError::CrcMismatch { .. }));
    }

    #[test]
    fn array_crc_mismatch_returns_error() {
        let type_guid = ms_basic_data();
        let entry_bytes = build_entry(type_guid, Uuid::new_v4(), 2048, 206847, "Data");
        let bad_crc: u32 = 0xDEADBEEF;

        let mut header = build_header();
        LittleEndian::write_u32(&mut header[16..20], 0);
        LittleEndian::write_u32(&mut header[80..84], 1);
        LittleEndian::write_u32(&mut header[88..92], bad_crc); // wrong
        let hcrc = crc32fast::hash(&header[..92]);
        LittleEndian::write_u32(&mut header[16..20], hcrc);

        let err = parse(&header, &entry_bytes, 204800, 512).unwrap_err();
        assert!(matches!(err, PartitionError::CrcMismatch { .. }));
    }

    #[test]
    fn utf16_name_parsed_correctly() {
        let type_guid = ms_basic_data();
        let entry = build_entry(type_guid, Uuid::new_v4(), 2048, 206847, "EFI System");
        let header = build_header_with_entries(&entry, 1);
        let table = parse(&header, &entry, 204800, 512).unwrap();
        assert_eq!(table.entries[0].name.as_deref(), Some("EFI System"));
    }
}

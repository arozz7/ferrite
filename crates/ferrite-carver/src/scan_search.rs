//! Signature-matching helpers used by [`Carver::scan_impl`].
//!
//! Kept separate from `scanner.rs` to keep both files under the 600-line limit.

use crate::pre_validate;
use crate::scanner::CarveHit;
use crate::signature::Signature;

// ── Chunk search ──────────────────────────────────────────────────────────────

/// Return all positions within `data[..report_end]` where `sig.header` begins.
///
/// Uses [`memchr`] on the first fixed (non-wildcard) byte for fast scanning,
/// then verifies the full pattern including `??` wildcard positions.
pub(crate) fn find_all(
    sig: &Signature,
    data: &[u8],
    chunk_abs_offset: u64,
    report_end: usize,
) -> Vec<CarveHit> {
    let header = &sig.header;
    if header.is_empty() || data.is_empty() {
        return vec![];
    }

    // Find the first fixed byte to use as the memchr anchor.
    let Some((anchor_idx, anchor_byte)) = header
        .iter()
        .enumerate()
        .find_map(|(i, b)| b.map(|byte| (i, byte)))
    else {
        return vec![]; // all-wildcard header — refuse to match everything
    };

    let report_end = report_end.min(data.len());
    let mut hits = Vec::new();
    // Search window starts at anchor_idx so we can back-compute the header start.
    let mut search_start = anchor_idx;

    loop {
        if search_start >= report_end + anchor_idx {
            break;
        }
        let scan_end = (report_end + anchor_idx).min(data.len());
        let window = &data[search_start..scan_end];
        let Some(rel) = memchr::memchr(anchor_byte, window) else {
            break;
        };
        // Position of the anchor byte in data[].
        let anchor_pos = search_start + rel;
        // Position where the header would start.
        let pos = anchor_pos.saturating_sub(anchor_idx);

        if pos + header.len() <= data.len()
            && pos < report_end
            && header_matches(header, data, pos)
            && sig
                .pre_validate
                .as_ref()
                .is_none_or(|kind| pre_validate::is_valid(kind, data, pos))
        {
            let magic_abs = chunk_abs_offset + pos as u64;
            // For non-zero-offset signatures the magic appears at header_offset
            // bytes into the file.  Shift back to obtain the true file start.
            // Skip hits where the file start would precede the device start.
            if magic_abs < sig.header_offset {
                search_start = anchor_pos + 1;
                continue;
            }
            hits.push(CarveHit {
                byte_offset: magic_abs - sig.header_offset,
                signature: sig.clone(),
            });
        }
        search_start = anchor_pos + 1;
    }

    hits
}

/// Check whether `header` matches `data` starting at `pos`.
///
/// `None` entries in `header` are wildcards and match any byte.
#[inline]
pub(crate) fn header_matches(header: &[Option<u8>], data: &[u8], pos: usize) -> bool {
    header.iter().enumerate().all(|(i, opt)| match opt {
        None => true,
        Some(b) => data.get(pos + i) == Some(b),
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pre_validate::PreValidate;
    use crate::signature::Signature;

    fn make_zip_lfh(fname: &str, version: u16, method: u16) -> Vec<u8> {
        let fb = fname.as_bytes();
        let mut buf = vec![0u8; 30 + fb.len()];
        buf[0..4].copy_from_slice(b"PK\x03\x04");
        buf[4..6].copy_from_slice(&version.to_le_bytes());
        buf[8..10].copy_from_slice(&method.to_le_bytes());
        buf[26..28].copy_from_slice(&(fb.len() as u16).to_le_bytes());
        buf[30..30 + fb.len()].copy_from_slice(fb);
        buf
    }

    fn zip_sig() -> Signature {
        Signature {
            name: "ZIP".into(),
            extension: "zip".into(),
            header: vec![Some(0x50), Some(0x4B), Some(0x03), Some(0x04)],
            footer: vec![0x50, 0x4B, 0x05, 0x06],
            footer_last: false,
            max_size: 1_000_000,
            size_hint: None,
            min_size: 0,
            pre_validate: Some(PreValidate::Zip),
            header_offset: 0,
        }
    }

    #[test]
    fn zip_pre_validate_drops_dir_hit_in_find_all() {
        let sig = zip_sig();
        let data = make_zip_lfh("patch/", 20, 8);
        let hits = find_all(&sig, &data, 0, data.len());
        assert!(
            hits.is_empty(),
            "directory-entry ZIP header should be rejected by find_all"
        );
    }

    #[test]
    fn zip_pre_validate_keeps_file_hit_in_find_all() {
        let sig = zip_sig();
        let data = make_zip_lfh("document.pdf", 20, 8);
        let hits = find_all(&sig, &data, 0, data.len());
        assert_eq!(hits.len(), 1, "file-entry ZIP header should pass find_all");
    }

    // ── header_offset infrastructure ──────────────────────────────────────────

    /// A simple signature whose magic appears at byte 10 within the file
    /// (like DICOM's "DICM" at offset 128, but much smaller for testing).
    fn offset_sig(magic: Vec<Option<u8>>, offset: u64) -> Signature {
        Signature {
            name: "OffsetTest".into(),
            extension: "tst".into(),
            header: magic,
            footer: vec![],
            footer_last: false,
            max_size: 1_000_000,
            size_hint: None,
            min_size: 0,
            pre_validate: None,
            header_offset: offset,
        }
    }

    #[test]
    fn header_offset_shifts_byte_offset_back() {
        // Magic 0xAB at position 10 in data; file starts at position 10 - 5 = 5.
        let sig = offset_sig(vec![Some(0xAB)], 5);
        let mut data = vec![0u8; 20];
        data[10] = 0xAB;
        let hits = find_all(&sig, &data, 100, data.len());
        assert_eq!(hits.len(), 1);
        // byte_offset = chunk_abs_offset(100) + pos(10) - header_offset(5) = 105
        assert_eq!(hits[0].byte_offset, 105);
    }

    #[test]
    fn header_offset_skips_hit_before_device_start() {
        // Magic at position 3 in data; header_offset = 5.
        // File start would be at chunk_abs_offset(0) + 3 - 5 = negative → skip.
        let sig = offset_sig(vec![Some(0xAB)], 5);
        let mut data = vec![0u8; 20];
        data[3] = 0xAB;
        let hits = find_all(&sig, &data, 0, data.len());
        assert!(hits.is_empty(), "hit before device start should be skipped");
    }

    #[test]
    fn header_offset_zero_behaves_normally() {
        // Offset 0 should work identically to existing behaviour.
        let sig = offset_sig(vec![Some(0xCD)], 0);
        let mut data = vec![0u8; 10];
        data[4] = 0xCD;
        let hits = find_all(&sig, &data, 200, data.len());
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].byte_offset, 204); // 200 + 4 - 0
    }
}

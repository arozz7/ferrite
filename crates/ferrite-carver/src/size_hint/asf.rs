//! ASF (Advanced Systems Format) size-hint handler for WMV / WMA files.
//!
//! ASF files begin with an ASF Header Object, which contains a sequence of
//! sub-objects.  One of those sub-objects is the File Properties Object,
//! which stores the exact total file size as a u64 LE field.
//!
//! ## ASF Header Object layout (at file offset 0)
//! ```text
//! [  0.. 15] GUID  (30 26 B2 75 8E 66 CF 11 A6 D9 00 AA 00 62 CE 6C)
//! [ 16.. 23] object_size  — u64 LE, total size of the header section
//! [ 24.. 27] num_headers  — u32 LE, number of sub-objects that follow
//! [ 28.. 29] reserved     — 2 bytes (always 01 02)
//! [ 30..   ] sub-objects
//! ```
//!
//! ## Sub-object layout (each)
//! ```text
//! [  0.. 15] GUID        — identifies the object type
//! [ 16.. 23] object_size — u64 LE, total size including these 24 bytes
//! [ 24..   ] object data
//! ```
//!
//! ## File Properties Object (GUID: A1 DC AB 8C 47 A9 CF 11 8E E4 00 C0 0C 20 53 65)
//! ```text
//! [  0.. 15] GUID
//! [ 16.. 23] object_size
//! [ 24.. 39] File ID GUID (16 bytes)
//! [ 40.. 47] File Size    — u64 LE  ← what we want
//! ```

use ferrite_blockdev::BlockDevice;

use crate::carver_io::read_bytes_clamped;

/// GUID of the ASF Header Object.
const HDR_GUID: [u8; 16] = [
    0x30, 0x26, 0xB2, 0x75, 0x8E, 0x66, 0xCF, 0x11, 0xA6, 0xD9, 0x00, 0xAA, 0x00, 0x62, 0xCE, 0x6C,
];

/// GUID of the File Properties Object.
const FPO_GUID: [u8; 16] = [
    0xA1, 0xDC, 0xAB, 0x8C, 0x47, 0xA9, 0xCF, 0x11, 0x8E, 0xE4, 0x00, 0xC0, 0x0C, 0x20, 0x53, 0x65,
];

/// Minimum reasonable ASF header size (30-byte outer header + at least one
/// 24-byte sub-object header).
const MIN_HDR_SIZE: u64 = 54;

/// Maximum number of sub-objects we'll walk before giving up.
const MAX_SUBOBJECTS: u32 = 256;

pub(super) fn asf_hint(device: &dyn BlockDevice, file_offset: u64) -> Option<u64> {
    // ── Read and validate the outer ASF Header Object header (30 bytes) ──────

    let hdr = read_bytes_clamped(device, file_offset, 30).ok()?;
    if hdr.len() < 30 {
        return None;
    }

    // Verify GUID.
    if hdr[..16] != HDR_GUID {
        return None;
    }

    let header_size = u64::from_le_bytes(hdr[16..24].try_into().ok()?);
    if header_size < MIN_HDR_SIZE {
        return None;
    }

    let num_headers = u32::from_le_bytes(hdr[24..28].try_into().ok()?);
    let limit = num_headers.min(MAX_SUBOBJECTS);

    // ── Walk sub-objects starting at offset 30 ────────────────────────────────

    let mut pos = file_offset + 30;
    let header_end = file_offset + header_size;

    for _ in 0..limit {
        if pos + 24 > header_end {
            break;
        }

        let sub = read_bytes_clamped(device, pos, 24).ok()?;
        if sub.len() < 24 {
            break;
        }

        let guid: [u8; 16] = sub[..16].try_into().ok()?;
        let obj_size = u64::from_le_bytes(sub[16..24].try_into().ok()?);

        // Guard against zero or impossibly small object sizes to avoid looping.
        if obj_size < 24 {
            break;
        }

        if guid == FPO_GUID {
            // File Properties Object — File Size is at offset 40 within the object
            // (GUID 16 + size 8 + File ID GUID 16 = 40).
            let fpo = read_bytes_clamped(device, pos + 40, 8).ok()?;
            if fpo.len() < 8 {
                return None;
            }
            let file_size = u64::from_le_bytes(fpo[..8].try_into().ok()?);
            // 0 means "unknown" in the ASF spec — fall back to max_size.
            if file_size == 0 {
                return None;
            }
            return Some(file_size);
        }

        pos += obj_size;
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrite_blockdev::MockBlockDevice;

    /// Build a minimal ASF buffer containing one File Properties Object with
    /// the given `file_size`.
    fn make_asf(file_size: u64) -> Vec<u8> {
        // Outer header: 30 bytes + one sub-object of 48 bytes = 78 bytes total.
        let header_size: u64 = 78;
        let num_headers: u32 = 1;

        let fpo_size: u64 = 48; // 24 header + 16 File ID GUID + 8 file size

        let mut buf = vec![0u8; header_size as usize];

        // Outer ASF Header Object
        buf[..16].copy_from_slice(&HDR_GUID);
        buf[16..24].copy_from_slice(&header_size.to_le_bytes());
        buf[24..28].copy_from_slice(&num_headers.to_le_bytes());
        buf[28] = 0x01;
        buf[29] = 0x02;

        // File Properties Object at offset 30
        buf[30..46].copy_from_slice(&FPO_GUID);
        buf[46..54].copy_from_slice(&fpo_size.to_le_bytes());
        // File ID GUID (16 bytes) at 54 — leave as zeros
        // File Size at offset 40 within the FPO = absolute offset 30 + 40 = 70
        buf[70..78].copy_from_slice(&file_size.to_le_bytes());

        buf
    }

    #[test]
    fn asf_reads_file_size() {
        let expected: u64 = 512 * 1024 * 1024; // 512 MiB
        let data = make_asf(expected);
        let device = MockBlockDevice::new(data, 512);
        assert_eq!(asf_hint(&device, 0), Some(expected));
    }

    #[test]
    fn asf_zero_file_size_returns_none() {
        let data = make_asf(0);
        let device = MockBlockDevice::new(data, 512);
        assert!(asf_hint(&device, 0).is_none());
    }

    #[test]
    fn asf_wrong_guid_returns_none() {
        let mut data = make_asf(1_000_000);
        data[0] = 0xFF; // corrupt GUID
        let device = MockBlockDevice::new(data, 512);
        assert!(asf_hint(&device, 0).is_none());
    }

    #[test]
    fn asf_nonzero_file_offset() {
        let base: u64 = 4096;
        let expected: u64 = 256 * 1024 * 1024;
        let asf_buf = make_asf(expected);
        let mut data = vec![0u8; base as usize + asf_buf.len()];
        data[base as usize..].copy_from_slice(&asf_buf);
        let device = MockBlockDevice::new(data, 512);
        assert_eq!(asf_hint(&device, base), Some(expected));
    }
}

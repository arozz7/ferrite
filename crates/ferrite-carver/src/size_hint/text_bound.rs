//! Text-boundary size-hint handler.
//!
//! Scans forward from `file_offset` reading in chunks and stops when a null
//! byte or a sustained run of non-text bytes is found.  Returns the byte
//! length of the contiguous text region, suitable for XML and similar
//! text-based formats that may be followed by unrelated binary data on disk.

use ferrite_blockdev::BlockDevice;

use crate::carver_io::read_bytes_clamped;

/// Maximum consecutive non-text bytes before we declare "end of text".
const MAX_NON_TEXT_RUN: usize = 8;

/// Bytes per read chunk (64 KiB — small enough for text scanning).
const CHUNK_SIZE: usize = 64 * 1024;

/// Returns `true` if the byte is plausible text content (printable ASCII,
/// common whitespace, or valid UTF-8 lead/continuation byte ≥ 0x80).
#[inline]
fn is_text_byte(b: u8) -> bool {
    // Null byte is never text.
    if b == 0 {
        return false;
    }
    // Printable ASCII (0x20..=0x7E) + common whitespace (tab, LF, CR).
    if matches!(b, 0x09 | 0x0A | 0x0D | 0x20..=0x7E) {
        return true;
    }
    // UTF-8 continuation / lead bytes (0x80..=0xFE) — allow them so that
    // UTF-8 encoded XML passes through.  0xFF is never valid UTF-8 but we
    // tolerate isolated instances; MAX_NON_TEXT_RUN handles sustained junk.
    b >= 0x80
}

/// Scan forward from `file_offset` and return the byte length of the
/// contiguous text region, or `None` if the very first bytes are non-text.
pub(super) fn text_bound_hint(
    device: &dyn BlockDevice,
    file_offset: u64,
    max_size: u64,
) -> Option<u64> {
    let device_size = device.size();
    let scan_end = file_offset.saturating_add(max_size).min(device_size);

    let mut pos = file_offset;
    let mut last_text_end: u64 = 0;
    let mut non_text_run: usize = 0;
    let mut found_any = false;

    while pos < scan_end {
        let to_read = CHUNK_SIZE.min((scan_end - pos) as usize);
        let data = match read_bytes_clamped(device, pos, to_read) {
            Ok(d) if !d.is_empty() => d,
            _ => break,
        };

        for (i, &b) in data.iter().enumerate() {
            if b == 0 {
                // Null byte: hard stop.
                if found_any {
                    return Some(last_text_end);
                }
                return None;
            }
            if is_text_byte(b) {
                found_any = true;
                non_text_run = 0;
                last_text_end = (pos - file_offset) + i as u64 + 1;
            } else {
                non_text_run += 1;
                if non_text_run >= MAX_NON_TEXT_RUN {
                    return if found_any { Some(last_text_end) } else { None };
                }
            }
        }

        if data.len() < to_read {
            break;
        }
        pos += data.len() as u64;
    }

    if found_any {
        Some(last_text_end)
    } else {
        None
    }
}

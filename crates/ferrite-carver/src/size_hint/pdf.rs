//! PDF linearized-length size hint.
//!
//! Linearized PDFs include a `/Linearized` dictionary as the first object,
//! with a `/L` key whose value is the total file length.  This hint reads
//! that value from the first ~256 bytes and returns it as the file size.
//!
//! Non-linearized PDFs (no `/Linearized` or no `/L` key) return `None`,
//! falling back to the normal footer-based extraction.

use ferrite_blockdev::BlockDevice;

use crate::carver_io::read_bytes_clamped;

/// Find a byte needle in a haystack, returning the starting index.
fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Read the first ~256 bytes and look for `/Linearized` + `/L <number>`.
///
/// Returns `Some(size)` if a valid linearized length is found, `None` otherwise.
pub(super) fn pdf_hint(device: &dyn BlockDevice, file_offset: u64) -> Option<u64> {
    let buf = read_bytes_clamped(device, file_offset, 256).ok()?;

    // Must contain /Linearized to be a linearized PDF.
    find_bytes(&buf, b"/Linearized")?;

    // Find /L followed by a space and a digit.
    // The /L key can appear anywhere in the linearization dictionary.
    let mut offset = 0;
    while offset + 3 < buf.len() {
        let remaining = &buf[offset..];
        let pos = match find_bytes(remaining, b"/L ") {
            Some(p) => p,
            None => break,
        };
        let after = &remaining[pos + 3..];
        // Skip whitespace after /L.
        let skip = after
            .iter()
            .take_while(|&&b| b == b' ' || b == b'\t')
            .count();
        let trimmed = &after[skip..];
        // Must start with a digit (otherwise it might be /Linearized, /Length, etc.)
        if let Some(&first) = trimmed.first() {
            if first.is_ascii_digit() {
                // Collect digit bytes and parse.
                let digits: Vec<u8> = trimmed
                    .iter()
                    .take_while(|&&b| b.is_ascii_digit())
                    .copied()
                    .collect();
                if let Ok(num_str) = std::str::from_utf8(&digits) {
                    if let Ok(size) = num_str.parse::<u64>() {
                        if size > 0 {
                            return Some(size);
                        }
                    }
                }
            }
        }
        // Not a match — advance past this /L and keep searching.
        offset += pos + 3;
    }

    None
}

//! TAR archive size-hint handler.
//!
//! A POSIX ustar TAR archive is a sequence of 512-byte blocks:
//!
//! ```text
//! ┌─────────────────────┐
//! │  File header block  │  512 bytes
//! │  offset 124: size   │  12 bytes, ASCII octal, null-terminated
//! ├─────────────────────┤
//! │  File data blocks   │  ceil(size / 512) × 512 bytes
//! ├─────────────────────┤
//! │  ... more entries   │
//! ├─────────────────────┤
//! │  Zero block         │  512 bytes of 0x00
//! │  Zero block         │  512 bytes of 0x00  ← end-of-archive marker
//! └─────────────────────┘
//! ```
//!
//! The `header_offset = 257` in the signature means the scanner hits the
//! `ustar` magic mid-header; we shift back so `file_offset` points to the
//! very start of the first 512-byte header block.

use ferrite_blockdev::BlockDevice;

use crate::carver_io::read_bytes_clamped;

const BLOCK: u64 = 512;

/// Offset of the size field within a TAR header block.
const SIZE_OFFSET: usize = 124;

/// Length of the size field (12 bytes, ASCII octal).
const SIZE_LEN: usize = 12;

/// Maximum entries to walk before giving up (avoids infinite loops on corrupt
/// data; 200 000 entries × minimum 1 block each = up to 100 MiB of headers).
const MAX_ENTRIES: u64 = 200_000;

pub(super) fn tar_hint(device: &dyn BlockDevice, file_offset: u64, max_size: u64) -> Option<u64> {
    let end = file_offset + max_size;
    let mut pos = file_offset;
    let mut consecutive_zero_blocks: u8 = 0;

    for _ in 0..MAX_ENTRIES {
        if pos + BLOCK > end {
            break;
        }

        let block = read_bytes_clamped(device, pos, BLOCK as usize).ok()?;
        if block.len() < BLOCK as usize {
            break;
        }

        // Check for zero block (end-of-archive marker).
        if block.iter().all(|&b| b == 0) {
            consecutive_zero_blocks += 1;
            pos += BLOCK;
            if consecutive_zero_blocks >= 2 {
                // Two consecutive zero blocks = end of archive.
                return Some(pos - file_offset);
            }
            continue;
        }

        // Non-zero block resets the zero-block counter.
        consecutive_zero_blocks = 0;

        // Parse the file size from the ASCII octal field at offset 124.
        let size_field = &block[SIZE_OFFSET..SIZE_OFFSET + SIZE_LEN];
        let file_size = parse_octal(size_field)?;

        // Advance past the header block and the data blocks.
        let data_blocks = file_size.div_ceil(BLOCK);
        pos += BLOCK + data_blocks * BLOCK;
    }

    // If we ran out of entries or hit max_size without finding the double-zero
    // terminator, return the farthest byte we scanned.
    if pos > file_offset {
        Some((pos - file_offset).min(max_size))
    } else {
        None
    }
}

/// Parse a null-terminated (or space-padded) ASCII octal string.
/// Returns `None` if the field contains no valid octal digits.
fn parse_octal(field: &[u8]) -> Option<u64> {
    let s = field
        .iter()
        .take_while(|&&b| b != 0 && b != b' ')
        .copied()
        .collect::<Vec<u8>>();

    if s.is_empty() {
        // Size field of zero = zero-length file, which is valid.
        return Some(0);
    }

    let mut value: u64 = 0;
    for &b in &s {
        if !(b'0'..=b'7').contains(&b) {
            return None;
        }
        value = value.checked_mul(8)?.checked_add((b - b'0') as u64)?;
    }
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrite_blockdev::MockBlockDevice;

    fn make_tar_header(filename: &[u8], file_size: u64) -> Vec<u8> {
        let mut block = vec![0u8; 512];
        // Filename (up to 100 bytes)
        let name_len = filename.len().min(99);
        block[..name_len].copy_from_slice(&filename[..name_len]);
        // File size as ASCII octal, null-terminated, right-padded with spaces
        let size_str = format!("{:011o}\0", file_size);
        block[SIZE_OFFSET..SIZE_OFFSET + 12].copy_from_slice(size_str.as_bytes());
        // ustar magic
        block[257..263].copy_from_slice(b"ustar\0");
        block
    }

    fn make_tar(entries: &[(u64,)]) -> Vec<u8> {
        let mut data = Vec::new();
        for &(size,) in entries {
            data.extend(make_tar_header(b"file.dat", size));
            let data_blocks = size.div_ceil(512) as usize;
            data.extend(vec![0xAAu8; data_blocks * 512]);
        }
        // Two zero blocks (end-of-archive)
        data.extend(vec![0u8; 1024]);
        data
    }

    #[test]
    fn tar_single_file() {
        let data = make_tar(&[(1024,)]);
        let expected = (512 + 1024 + 1024) as u64; // header + data + 2 zero blocks
        let device = MockBlockDevice::new(data.clone(), 512);
        assert_eq!(tar_hint(&device, 0, data.len() as u64), Some(expected));
    }

    #[test]
    fn tar_multiple_files() {
        // 3 files: 512 B, 1 KiB, 2 KiB
        let data = make_tar(&[(512,), (1024,), (2048,)]);
        let device = MockBlockDevice::new(data.clone(), 512);
        let result = tar_hint(&device, 0, data.len() as u64);
        assert_eq!(result, Some(data.len() as u64));
    }

    #[test]
    fn tar_zero_size_file() {
        // An empty file (size = 0) is valid in TAR.
        let data = make_tar(&[(0,)]);
        let expected = (512 + 1024) as u64; // header + 2 zero blocks (no data)
        let device = MockBlockDevice::new(data.clone(), 512);
        assert_eq!(tar_hint(&device, 0, data.len() as u64), Some(expected));
    }

    #[test]
    fn parse_octal_valid() {
        assert_eq!(parse_octal(b"00001750\0   "), Some(0o1750));
        assert_eq!(parse_octal(b"00000000000\0"), Some(0));
    }

    #[test]
    fn parse_octal_invalid_digit() {
        assert_eq!(parse_octal(b"0000008000\0\0"), None);
    }
}

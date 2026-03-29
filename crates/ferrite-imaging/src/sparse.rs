//! Sparse image file support.
//!
//! On NTFS (Windows), sparse files require a single `FSCTL_SET_SPARSE`
//! `DeviceIoControl` call immediately after the file is opened.  On
//! Linux/macOS holes are created automatically the first time the process seeks
//! past an unwritten region — no explicit setup needed.
//!
//! [`write_or_skip`] is the hot-path helper: it checks whether a buffer is
//! entirely zero and, if so, seeks past it without writing, creating a hole.

use std::fs::File;
use std::io::{self, Seek, SeekFrom, Write};

/// Enable sparse-file mode on `file`.
///
/// * **Windows (NTFS)** — sends `FSCTL_SET_SPARSE` via `DeviceIoControl`.
///   Silently proceeds when the call fails (the destination filesystem may not
///   support sparse files, e.g. FAT32 USB stick) — holes simply will not be
///   created, and the image grows to full size.
/// * **Linux / macOS / other** — no-op; holes are created automatically.
pub fn enable_sparse(file: &File) -> io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::io::AsRawHandle;
        // FSCTL_SET_SPARSE = 0x000900C4
        const FSCTL_SET_SPARSE: u32 = 0x000900C4;
        let handle = file.as_raw_handle();
        let mut bytes_returned: u32 = 0;
        // Safety: `handle` is valid for the lifetime of `file`.
        let ok = unsafe {
            windows_sys::Win32::System::IO::DeviceIoControl(
                handle as windows_sys::Win32::Foundation::HANDLE,
                FSCTL_SET_SPARSE,
                std::ptr::null(),
                0,
                std::ptr::null_mut(),
                0,
                &mut bytes_returned,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            // Non-fatal — destination FS does not support sparse files.
            tracing::warn!(
                "FSCTL_SET_SPARSE failed (non-NTFS destination?); \
                 sparse savings disabled for this session"
            );
        }
    }
    #[cfg(not(target_os = "windows"))]
    let _ = file;
    Ok(())
}

/// Write `buf` at byte `pos` in `file`, or skip it if the buffer is all zeros.
///
/// * **All-zero buffer** — seeks to `pos + buf.len()` without writing, leaving
///   a hole.  The file position is advanced past the skipped region.
/// * **Non-zero buffer** — seeks to `pos` and calls `write_all`.
pub fn write_or_skip(file: &mut File, pos: u64, buf: &[u8]) -> io::Result<()> {
    if buf.iter().all(|&b| b == 0) {
        // Sparse skip: advance position without writing.
        file.seek(SeekFrom::Start(pos + buf.len() as u64))?;
        return Ok(());
    }
    file.seek(SeekFrom::Start(pos))?;
    file.write_all(buf)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use tempfile::NamedTempFile;

    #[test]
    fn write_or_skip_all_zero_does_not_write_data() {
        let mut tmp = NamedTempFile::new().unwrap();
        let file = tmp.as_file_mut();
        let zeros = [0u8; 512];
        write_or_skip(file, 0, &zeros).unwrap();

        // File position must be at 512 after the skip.
        let pos = file.stream_position().unwrap();
        assert_eq!(pos, 512);

        // If the OS extended the file during the seek, all bytes must be zero.
        let file_len = file.metadata().unwrap().len();
        if file_len > 0 {
            file.seek(SeekFrom::Start(0)).unwrap();
            let mut contents = vec![0u8; file_len as usize];
            file.read_exact(&mut contents).unwrap();
            assert!(
                contents.iter().all(|&b| b == 0),
                "only zeros should be present after sparse skip"
            );
        }
    }

    #[test]
    fn write_or_skip_non_zero_writes_data() {
        let mut tmp = NamedTempFile::new().unwrap();
        let file = tmp.as_file_mut();
        let data = [0xABu8; 512];
        write_or_skip(file, 0, &data).unwrap();

        file.seek(SeekFrom::Start(0)).unwrap();
        let mut readback = [0u8; 512];
        file.read_exact(&mut readback).unwrap();
        assert_eq!(readback, data);
    }

    #[test]
    fn write_or_skip_non_zero_at_nonzero_offset() {
        let mut tmp = NamedTempFile::new().unwrap();
        let file = tmp.as_file_mut();
        // Skip the first 512-byte block (zeros), write the second (non-zero).
        let zeros = [0u8; 512];
        let data = [0xCDu8; 512];
        write_or_skip(file, 0, &zeros).unwrap();
        write_or_skip(file, 512, &data).unwrap();

        file.seek(SeekFrom::Start(512)).unwrap();
        let mut readback = [0u8; 512];
        file.read_exact(&mut readback).unwrap();
        assert_eq!(readback, data);
    }

    #[test]
    fn write_or_skip_mixed_buffer_writes_entirely() {
        // A buffer that starts with zeros but has a non-zero byte must be written.
        let mut tmp = NamedTempFile::new().unwrap();
        let file = tmp.as_file_mut();
        let mut buf = [0u8; 512];
        buf[511] = 0xFF;
        write_or_skip(file, 0, &buf).unwrap();

        file.seek(SeekFrom::Start(0)).unwrap();
        let mut readback = [0u8; 512];
        file.read_exact(&mut readback).unwrap();
        assert_eq!(readback, buf);
    }

    #[test]
    fn enable_sparse_does_not_error_on_regular_file() {
        let tmp = NamedTempFile::new().unwrap();
        // On Windows sends FSCTL_SET_SPARSE; on Linux/macOS is a no-op.
        // Must not return an error in either case.
        enable_sparse(tmp.as_file()).unwrap();
    }
}

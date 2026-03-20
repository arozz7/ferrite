//! Windows volume quiesce guard.
//!
//! When Ferrite opens a physical drive for imaging, Windows immediately mounts
//! any readable filesystem on it and fires background services (Search Indexer,
//! AutoPlay, Explorer thumbnail generation).  These services compete directly
//! for I/O on an already-stressed drive.
//!
//! `VolumeGuard::acquire(disk_number)` enumerates all Windows volumes that
//! live on that disk, takes each one offline via `IOCTL_VOLUME_OFFLINE`, and
//! re-onlines them automatically when dropped (RAII).
//!
//! Raw `\\.\PhysicalDriveX` access is unaffected by volume-offline state, so
//! Ferrite's imaging and carving code continues to work normally.

use std::ptr;

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ACCESS_DENIED, ERROR_NO_MORE_FILES, GENERIC_READ,
    GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FindFirstVolumeW, FindNextVolumeW, FindVolumeClose, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::IO::DeviceIoControl;

use tracing::{debug, warn};

// ── IOCTL constants (from Windows SDK winioctl.h) ────────────────────────────
// CTL_CODE(DeviceType, Function, Method, Access)
//   = (DeviceType << 16) | (Access << 14) | (Function << 2) | Method
//
// IOCTL_STORAGE_GET_DEVICE_NUMBER:
//   DeviceType = FILE_DEVICE_MASS_STORAGE (0x2D)
//   Function   = 0x420
//   Method     = METHOD_BUFFERED (0)
//   Access     = FILE_ANY_ACCESS (0)
const IOCTL_STORAGE_GET_DEVICE_NUMBER: u32 = 0x002D_1080;

// IOCTL_VOLUME_ONLINE:
//   DeviceType = IOCTL_VOLUME_BASE ('V' = 0x56)
//   Function   = 2
//   Method     = METHOD_BUFFERED (0)
//   Access     = FILE_READ_ACCESS | FILE_WRITE_ACCESS (3)
const IOCTL_VOLUME_ONLINE: u32 = 0x0056_C008;

// IOCTL_VOLUME_OFFLINE:
//   DeviceType = IOCTL_VOLUME_BASE ('V' = 0x56)
//   Function   = 3
//   Method     = METHOD_BUFFERED (0)
//   Access     = FILE_READ_ACCESS | FILE_WRITE_ACCESS (3)
const IOCTL_VOLUME_OFFLINE: u32 = 0x0056_C00C;

// ── ABI-compatible struct (from Windows SDK) ─────────────────────────────────

#[repr(C)]
struct StorageDeviceNumber {
    device_type: u32,
    device_number: u32,
    partition_number: u32,
}

// ── Public types ──────────────────────────────────────────────────────────────

/// Result of a `VolumeGuard::acquire` call — shown in the Drives tab UI.
#[derive(Debug, Clone)]
pub enum VolsStatus {
    /// All N volumes on this disk were successfully taken offline.
    Quiesced(usize),
    /// `n_ok` of `n_total` volumes offlined; others are system/boot volumes.
    Partial { n_ok: usize, n_total: usize },
    /// No mounted volumes found — disk is already quiet (raw/unformatted).
    NoVolumes,
    /// Insufficient privileges to offline volumes; run Ferrite as Administrator.
    NeedAdmin,
}

/// RAII guard that holds a set of volume GUID paths offline.
///
/// Dropping this guard re-onlines every volume that was successfully offlined,
/// restoring normal Windows filesystem access to those volumes.
pub struct VolumeGuard {
    /// Wide-char volume GUID paths (e.g. `\\?\Volume{…}\`) that were
    /// successfully taken offline.  Stored with trailing backslash; the
    /// backslash is stripped at open time.
    offlined: Vec<Vec<u16>>,
}

impl VolumeGuard {
    /// Enumerate all volumes on `disk_number` and take them offline.
    ///
    /// Always succeeds — partial/failed results are encoded in `VolsStatus`.
    /// Call this immediately after a physical drive is selected to stop Windows
    /// background services from competing for I/O.
    pub fn acquire(disk_number: u32) -> (Self, VolsStatus) {
        let candidates = volumes_on_disk(disk_number);
        if candidates.is_empty() {
            debug!(disk = disk_number, "no volumes found on disk");
            return (Self { offlined: vec![] }, VolsStatus::NoVolumes);
        }

        let n_total = candidates.len();
        let mut offlined = Vec::with_capacity(n_total);
        let mut any_access_denied = false;

        for wide_path in candidates {
            match try_offline(&wide_path) {
                OfflineResult::Ok => {
                    debug!("volume offlined successfully");
                    offlined.push(wide_path);
                }
                OfflineResult::AccessDenied => {
                    warn!("volume offline denied — system/boot volume, skipping");
                    any_access_denied = true;
                }
                OfflineResult::Err(e) => {
                    warn!(err = e, "volume offline failed");
                }
            }
        }

        let status = if offlined.is_empty() && any_access_denied {
            VolsStatus::NeedAdmin
        } else if offlined.len() == n_total {
            VolsStatus::Quiesced(n_total)
        } else {
            VolsStatus::Partial {
                n_ok: offlined.len(),
                n_total,
            }
        };

        (Self { offlined }, status)
    }
}

impl Drop for VolumeGuard {
    fn drop(&mut self) {
        for wide_path in &self.offlined {
            try_online(wide_path);
        }
    }
}

// ── Volume enumeration ────────────────────────────────────────────────────────

/// Returns wide-char volume GUID paths for all volumes whose backing disk
/// matches `disk_number`.
fn volumes_on_disk(disk_number: u32) -> Vec<Vec<u16>> {
    // MAX_PATH (260) is sufficient for a volume GUID path.
    let mut buf = vec![0u16; 260];
    let mut result = Vec::new();

    // SAFETY: buf is valid and sized to hold a volume GUID path.
    let search = unsafe { FindFirstVolumeW(buf.as_mut_ptr(), buf.len() as u32) };
    if search == INVALID_HANDLE_VALUE {
        return result;
    }

    loop {
        // Reconstruct a null-terminated wide string from the buffer.
        let nul = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        let wide_path: Vec<u16> = buf[..=nul].to_vec(); // includes the NUL

        if let Some(disk) = query_disk_number(&wide_path) {
            if disk == disk_number {
                result.push(wide_path);
            }
        }

        // SAFETY: search handle is valid.
        let ok = unsafe { FindNextVolumeW(search, buf.as_mut_ptr(), buf.len() as u32) };
        if ok == 0 {
            let err = unsafe { GetLastError() };
            if err != ERROR_NO_MORE_FILES {
                warn!(err, "FindNextVolumeW unexpected error");
            }
            break;
        }
    }

    // SAFETY: search handle is valid.
    unsafe { FindVolumeClose(search) };

    result
}

/// Returns the physical disk number that backs `vol_path`, or `None`.
///
/// `vol_path` is a null-terminated wide string like `\\?\Volume{GUID}\`.
fn query_disk_number(vol_path: &[u16]) -> Option<u32> {
    let h = open_volume_query(vol_path);
    if h == INVALID_HANDLE_VALUE {
        return None;
    }

    let mut sdn = StorageDeviceNumber {
        device_type: 0,
        device_number: 0,
        partition_number: 0,
    };
    let mut bytes_returned: u32 = 0;

    // SAFETY: h is valid; sdn is correctly sized for the IOCTL output.
    let ok = unsafe {
        DeviceIoControl(
            h,
            IOCTL_STORAGE_GET_DEVICE_NUMBER,
            ptr::null_mut(),
            0,
            &mut sdn as *mut _ as *mut _,
            std::mem::size_of::<StorageDeviceNumber>() as u32,
            &mut bytes_returned,
            ptr::null_mut(),
        )
    };

    // SAFETY: h is valid and owned here.
    unsafe { CloseHandle(h) };

    if ok != 0 {
        Some(sdn.device_number)
    } else {
        None
    }
}

// ── Volume offline / online ───────────────────────────────────────────────────

enum OfflineResult {
    Ok,
    AccessDenied,
    Err(u32),
}

fn try_offline(vol_path: &[u16]) -> OfflineResult {
    let h = open_volume_rw(vol_path);
    if h == INVALID_HANDLE_VALUE {
        let err = unsafe { GetLastError() };
        return if err == ERROR_ACCESS_DENIED {
            OfflineResult::AccessDenied
        } else {
            OfflineResult::Err(err)
        };
    }

    let mut bytes_returned: u32 = 0;
    // SAFETY: h valid; IOCTL_VOLUME_OFFLINE needs no input/output buffers.
    let ok = unsafe {
        DeviceIoControl(
            h,
            IOCTL_VOLUME_OFFLINE,
            ptr::null_mut(),
            0,
            ptr::null_mut(),
            0,
            &mut bytes_returned,
            ptr::null_mut(),
        )
    };
    let err = if ok == 0 {
        unsafe { GetLastError() }
    } else {
        0
    };

    // SAFETY: h is valid.
    unsafe { CloseHandle(h) };

    if ok != 0 {
        OfflineResult::Ok
    } else if err == ERROR_ACCESS_DENIED {
        OfflineResult::AccessDenied
    } else {
        OfflineResult::Err(err)
    }
}

fn try_online(vol_path: &[u16]) {
    let h = open_volume_rw(vol_path);
    if h == INVALID_HANDLE_VALUE {
        warn!(
            err = unsafe { GetLastError() },
            "could not open volume for re-online"
        );
        return;
    }

    let mut bytes_returned: u32 = 0;
    // SAFETY: h valid; IOCTL_VOLUME_ONLINE needs no buffers.
    let ok = unsafe {
        DeviceIoControl(
            h,
            IOCTL_VOLUME_ONLINE,
            ptr::null_mut(),
            0,
            ptr::null_mut(),
            0,
            &mut bytes_returned,
            ptr::null_mut(),
        )
    };
    if ok == 0 {
        warn!(
            err = unsafe { GetLastError() },
            "IOCTL_VOLUME_ONLINE failed"
        );
    }

    // SAFETY: h is valid.
    unsafe { CloseHandle(h) };
}

// ── Handle helpers ────────────────────────────────────────────────────────────

/// Open a volume handle with no requested access — sufficient for device-number
/// queries which use `FILE_ANY_ACCESS`.
fn open_volume_query(vol_path: &[u16]) -> HANDLE {
    let path = strip_trailing_backslash(vol_path);
    // SAFETY: path is a valid null-terminated wide string.
    unsafe {
        CreateFileW(
            path.as_ptr(),
            0, // FILE_ANY_ACCESS
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            ptr::null(),
            OPEN_EXISTING,
            0,
            ptr::null_mut(),
        )
    }
}

/// Open a volume handle with read+write access — required for
/// `IOCTL_VOLUME_OFFLINE` and `IOCTL_VOLUME_ONLINE`.
fn open_volume_rw(vol_path: &[u16]) -> HANDLE {
    let path = strip_trailing_backslash(vol_path);
    // SAFETY: path is a valid null-terminated wide string.
    unsafe {
        CreateFileW(
            path.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            ptr::null(),
            OPEN_EXISTING,
            0,
            ptr::null_mut(),
        )
    }
}

/// Strip the trailing `\` (0x005C) before the NUL terminator in a wide path.
///
/// `FindFirstVolumeW` returns `\\?\Volume{GUID}\` but `CreateFileW` requires
/// the path without that trailing slash for raw volume access.
fn strip_trailing_backslash(wide: &[u16]) -> Vec<u16> {
    let mut out = wide.to_vec();
    // Find the last non-NUL character.
    let last = out.iter().rposition(|&c| c != 0);
    if let Some(i) = last {
        if out[i] == b'\\' as u16 {
            out[i] = 0; // replace the backslash with NUL
        }
    }
    out
}

// ── Public helper ─────────────────────────────────────────────────────────────

/// Parse the physical disk number from a `\\.\PhysicalDriveN` path.
///
/// Returns `None` for image file paths (`FileBlockDevice`) and any path that
/// does not match the expected format.
pub fn parse_disk_number(path: &str) -> Option<u32> {
    let lower = path.to_lowercase();
    let n = lower.strip_prefix(r"\\.\physicaldrive")?.trim();
    n.parse().ok()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_physicaldrive_zero() {
        assert_eq!(parse_disk_number(r"\\.\PhysicalDrive0"), Some(0));
    }

    #[test]
    fn parse_physicaldrive_multi_digit() {
        assert_eq!(parse_disk_number(r"\\.\PhysicalDrive12"), Some(12));
    }

    #[test]
    fn parse_mixed_case() {
        assert_eq!(parse_disk_number(r"\\.\physicaldrive3"), Some(3));
    }

    #[test]
    fn parse_image_file_returns_none() {
        assert_eq!(parse_disk_number(r"M:\backup.img"), None);
    }

    #[test]
    fn parse_cdrom_returns_none() {
        assert_eq!(parse_disk_number(r"\\.\CdRom0"), None);
    }

    #[test]
    fn parse_empty_returns_none() {
        assert_eq!(parse_disk_number(""), None);
    }

    #[test]
    fn strip_backslash_removes_trailing() {
        // \\?\Volume{guid}\ → \\?\Volume{guid}\0
        let mut input: Vec<u16> = r"\\?\Volume{test}\".encode_utf16().collect();
        input.push(0); // NUL terminator
        let result = strip_trailing_backslash(&input);
        // Last non-NUL char should not be backslash.
        let last_non_nul = result.iter().rposition(|&c| c != 0);
        if let Some(i) = last_non_nul {
            assert_ne!(
                result[i], b'\\' as u16,
                "trailing backslash should be stripped"
            );
        }
    }

    #[test]
    fn strip_backslash_leaves_no_backslash_path_unchanged() {
        let mut input: Vec<u16> = r"\\?\Volume{test}".encode_utf16().collect();
        input.push(0);
        let result = strip_trailing_backslash(&input);
        let last_non_nul = result.iter().rposition(|&c| c != 0);
        // Should end with '}' not backslash.
        if let Some(i) = last_non_nul {
            assert_eq!(result[i], b'}' as u16);
        }
    }

    #[test]
    fn volume_guard_empty_drop_is_noop() {
        // A guard with no offlined volumes should drop without panicking.
        let guard = VolumeGuard { offlined: vec![] };
        drop(guard); // should not call any WinAPI
    }
}

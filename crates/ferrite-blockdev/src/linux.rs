//! Linux block device implementation using `libc`.
//!
//! Opens devices with `O_RDONLY | O_DIRECT | O_LARGEFILE` for unbuffered I/O.
//! Uses `pread64` for positioned reads — inherently thread-safe (no shared
//! file position).

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;

use ferrite_core::types::DeviceInfo;
use tracing::instrument;

use crate::{AlignedBuffer, BlockDevice, BlockDeviceError, Result};

// ioctl request codes (x86_64 / aarch64 / riscv64)
const BLKGETSIZE64: libc::c_ulong = 0x8008_1272; // get size in bytes → u64
const BLKSSZGET: libc::c_ulong = 0x0000_1268; // get logical sector size → i32

pub struct LinuxBlockDevice {
    fd: libc::c_int,
    info: DeviceInfo,
}

// SAFETY: `pread` is thread-safe — it does not use the file's shared position.
// The fd is read-only and immutable after construction.
unsafe impl Send for LinuxBlockDevice {}
unsafe impl Sync for LinuxBlockDevice {}

impl LinuxBlockDevice {
    pub fn open(path: &str) -> Result<Self> {
        let cpath = CString::new(path)
            .map_err(|_| BlockDeviceError::Unsupported("path contains null byte".into()))?;

        // O_DIRECT requires the buffer, offset, and size to all be sector-aligned.
        // O_LARGEFILE allows files/devices larger than 2 GiB on 32-bit; no-op on 64-bit.
        let flags = libc::O_RDONLY | libc::O_DIRECT | libc::O_LARGEFILE;

        // SAFETY: cpath is a valid null-terminated C string.
        let fd = unsafe { libc::open(cpath.as_ptr(), flags) };

        if fd < 0 {
            let e = std::io::Error::last_os_error();
            return Err(match e.kind() {
                std::io::ErrorKind::PermissionDenied => {
                    BlockDeviceError::PermissionDenied(path.to_string())
                }
                std::io::ErrorKind::NotFound => BlockDeviceError::NotFound(path.to_string()),
                _ => BlockDeviceError::Io {
                    device: path.to_string(),
                    offset: 0,
                    source: e,
                },
            });
        }

        let size = ioctl_blkgetsize64(fd, path)?;
        let sector_size = ioctl_blksszget(fd).unwrap_or(512);
        let (model, serial) = read_sysfs_info(path);

        let info = DeviceInfo {
            path: path.to_string(),
            model,
            serial,
            size_bytes: size,
            sector_size,
            logical_sector_size: sector_size,
        };

        Ok(Self { fd, info })
    }
}

impl Drop for LinuxBlockDevice {
    fn drop(&mut self) {
        // SAFETY: fd is valid and owned by this struct.
        unsafe { libc::close(self.fd) };
    }
}

impl BlockDevice for LinuxBlockDevice {
    #[instrument(skip(self, buf), fields(device = %self.info.path))]
    fn read_at(&self, offset: u64, buf: &mut AlignedBuffer) -> Result<usize> {
        let ptr = buf.as_mut_slice().as_mut_ptr().cast::<libc::c_void>();
        let count = buf.len();
        let off = offset as libc::off64_t;

        // SAFETY: fd is open and readable; ptr points to a live, aligned buffer.
        let n = unsafe { libc::pread64(self.fd, ptr, count, off) };

        if n < 0 {
            return Err(BlockDeviceError::Io {
                device: self.info.path.clone(),
                offset,
                source: std::io::Error::last_os_error(),
            });
        }

        Ok(n as usize)
    }

    fn size(&self) -> u64 {
        self.info.size_bytes
    }

    fn sector_size(&self) -> u32 {
        self.info.sector_size
    }

    fn device_info(&self) -> &DeviceInfo {
        &self.info
    }
}

// ── ioctl helpers ─────────────────────────────────────────────────────────────

fn ioctl_blkgetsize64(fd: libc::c_int, path: &str) -> Result<u64> {
    let mut size: u64 = 0;
    // SAFETY: BLKGETSIZE64 writes a u64 into &size.
    let ret = unsafe { libc::ioctl(fd, BLKGETSIZE64, &mut size) };
    if ret < 0 {
        return Err(BlockDeviceError::Io {
            device: path.to_string(),
            offset: 0,
            source: std::io::Error::last_os_error(),
        });
    }
    Ok(size)
}

fn ioctl_blksszget(fd: libc::c_int) -> Option<u32> {
    let mut sector_size: libc::c_int = 0;
    // SAFETY: BLKSSZGET writes an int into &sector_size.
    let ret = unsafe { libc::ioctl(fd, BLKSSZGET, &mut sector_size) };
    if ret < 0 || sector_size <= 0 {
        None
    } else {
        Some(sector_size as u32)
    }
}

// ── sysfs model/serial ────────────────────────────────────────────────────────

/// Read model and serial number from `/sys/block/<dev>/device/model` and
/// `/sys/block/<dev>/device/serial`.
///
/// Strips the `/dev/` prefix, e.g. `/dev/sda` → `/sys/block/sda/device/`.
fn read_sysfs_info(dev_path: &str) -> (Option<String>, Option<String>) {
    fn inner(dev_path: &str) -> Option<(Option<String>, Option<String>)> {
        let name = std::path::Path::new(dev_path).file_name()?.to_str()?;
        let base = format!("/sys/block/{name}/device");
        Some((
            read_sysfs_string(&format!("{base}/model")),
            read_sysfs_string(&format!("{base}/serial")),
        ))
    }
    inner(dev_path).unwrap_or((None, None))
}

fn read_sysfs_string(path: &str) -> Option<String> {
    let s = std::fs::read_to_string(path).ok()?;
    let trimmed = s.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

// ── Device enumeration ────────────────────────────────────────────────────────

/// Return `/dev/<name>` paths for all block devices listed in `/sys/block/`,
/// filtered to those that open without error.
pub fn enumerate_devices() -> Vec<String> {
    let Ok(entries) = std::fs::read_dir("/sys/block") else {
        return Vec::new();
    };

    entries
        .filter_map(|e| {
            let name = e.ok()?.file_name();
            let dev_path = format!("/dev/{}", name.to_str()?);
            let cpath = CString::new(dev_path.as_bytes()).ok()?;
            let flags = libc::O_RDONLY | libc::O_NONBLOCK;
            // SAFETY: cpath is a valid C string.
            let fd = unsafe { libc::open(cpath.as_ptr(), flags) };
            if fd < 0 {
                None
            } else {
                unsafe { libc::close(fd) };
                Some(dev_path)
            }
        })
        .collect()
}

pub mod aligned;
pub mod error;
pub mod file;
pub mod mock;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "windows")]
pub mod volume_guard;
#[cfg(target_os = "windows")]
mod windows;

pub use aligned::AlignedBuffer;
pub use error::{BlockDeviceError, Result};
pub use file::FileBlockDevice;
pub use mock::{ErrorPolicy, MockBlockDevice};

#[cfg(target_os = "linux")]
pub use linux::{enumerate_devices, LinuxBlockDevice};
#[cfg(target_os = "windows")]
pub use volume_guard::{parse_disk_number, VolsStatus, VolumeGuard};
#[cfg(target_os = "windows")]
pub use windows::{enumerate_devices, WindowsBlockDevice};

use std::sync::Arc;

use ferrite_core::types::DeviceInfo;

/// Core abstraction over a block storage device.
///
/// Implementors must be `Send + Sync` — the imaging engine reads from
/// multiple threads concurrently.
pub trait BlockDevice: Send + Sync {
    /// Read bytes starting at `offset` into `buf`.
    ///
    /// Returns the number of bytes actually read. A return of `0` means
    /// end-of-device. The caller must ensure `offset` and `buf.len()` are
    /// sector-aligned when the implementation requires it (e.g. `O_DIRECT`).
    fn read_at(&self, offset: u64, buf: &mut AlignedBuffer) -> Result<usize>;

    /// Total device size in bytes.
    fn size(&self) -> u64;

    /// Physical sector size in bytes — the alignment unit for direct I/O.
    fn sector_size(&self) -> u32;

    /// Static device metadata (path, model, serial, capacity).
    fn device_info(&self) -> &DeviceInfo;

    /// Try to open a second, independent handle to the same device.
    ///
    /// File-backed devices return a new handle that shares no mutex with the
    /// original, allowing a scanner thread and an extractor thread to read
    /// concurrently without blocking each other.
    ///
    /// Physical device implementations and the mock return `None`; callers
    /// must fall back to sharing the original `Arc`.
    fn try_clone_handle(&self) -> Option<Arc<dyn BlockDevice>> {
        None
    }
}

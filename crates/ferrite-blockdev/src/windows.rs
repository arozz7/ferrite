//! Windows block device implementation using `windows-sys`.
//!
//! Uses `CreateFileW` with `FILE_FLAG_NO_BUFFERING` for direct I/O.
//! Positioned reads are performed with `ReadFile` + `OVERLAPPED` (synchronous
//! mode — the handle is *not* opened with `FILE_FLAG_OVERLAPPED`, so the call
//! blocks until complete while still using the `OVERLAPPED` offset fields).

use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, FILE_FLAG_NO_BUFFERING, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Ioctl::{
    PropertyStandardQuery, StorageDeviceProperty, IOCTL_DISK_GET_DRIVE_GEOMETRY,
    IOCTL_DISK_GET_LENGTH_INFO, IOCTL_STORAGE_QUERY_PROPERTY, STORAGE_PROPERTY_QUERY,
};
use windows_sys::Win32::System::IO::{DeviceIoControl, OVERLAPPED};

use ferrite_core::types::DeviceInfo;
use tracing::instrument;

use crate::{AlignedBuffer, BlockDevice, BlockDeviceError, Result};

// ── ABI-compatible structs for DeviceIoControl results ───────────────────────
// We define our own to avoid the LARGE_INTEGER / union gymnastics in windows-sys.

#[repr(C)]
struct GetLengthInformation {
    length: i64, // LARGE_INTEGER at offset 0
}

#[repr(C)]
struct DiskGeometry {
    cylinders: i64, // LARGE_INTEGER
    media_type: u32,
    tracks_per_cylinder: u32,
    sectors_per_track: u32,
    bytes_per_sector: u32,
}

#[repr(C)]
struct StorageDeviceDescriptorHeader {
    version: u32,
    size: u32,
    device_type: u8,
    device_type_modifier: u8,
    removable_media: u8,
    command_queueing: u8,
    vendor_id_offset: u32,
    product_id_offset: u32,
    product_revision_offset: u32,
    serial_number_offset: u32,
}

// ─────────────────────────────────────────────────────────────────────────────

pub struct WindowsBlockDevice {
    handle: HANDLE,
    info: DeviceInfo,
}

// SAFETY: HANDLE is valid for concurrent ReadFile calls in synchronous mode
// (each call is self-contained via OVERLAPPED offsets). We never mutate shared
// state without synchronisation.
unsafe impl Send for WindowsBlockDevice {}
unsafe impl Sync for WindowsBlockDevice {}

impl WindowsBlockDevice {
    pub fn open(path: &str) -> Result<Self> {
        let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();

        // SAFETY: wide is a valid null-terminated UTF-16 string.
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null_mut(),
                OPEN_EXISTING,
                FILE_FLAG_NO_BUFFERING,
                std::ptr::null_mut(),
            )
        };

        if handle == INVALID_HANDLE_VALUE {
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

        let (size, sector_size) = query_geometry(handle, path)?;
        let (model, serial) = query_storage_property(handle);

        let info = DeviceInfo {
            path: path.to_string(),
            model,
            serial,
            size_bytes: size,
            sector_size,
            logical_sector_size: 512,
        };

        Ok(Self { handle, info })
    }
}

impl Drop for WindowsBlockDevice {
    fn drop(&mut self) {
        // SAFETY: handle is valid and owned exclusively by this struct.
        unsafe { CloseHandle(self.handle) };
    }
}

impl BlockDevice for WindowsBlockDevice {
    #[instrument(skip(self, buf), fields(device = %self.info.path))]
    fn read_at(&self, offset: u64, buf: &mut AlignedBuffer) -> Result<usize> {
        // Build an OVERLAPPED with the byte offset. Because the handle was NOT
        // opened with FILE_FLAG_OVERLAPPED, ReadFile uses these fields as the
        // starting position and blocks until the read completes.
        // SAFETY: OVERLAPPED is POD; we immediately initialise the Offset fields.
        // Accessing the Anonymous union's Anonymous struct variant is always valid.
        let mut ov: OVERLAPPED = unsafe {
            let mut o: OVERLAPPED = std::mem::zeroed();
            o.Anonymous.Anonymous.Offset = offset as u32;
            o.Anonymous.Anonymous.OffsetHigh = (offset >> 32) as u32;
            o
        };

        let mut bytes_read: u32 = 0;
        // SAFETY: handle valid; buf memory is live for the duration of the call.
        let ok = unsafe {
            ReadFile(
                self.handle,
                buf.as_mut_slice().as_mut_ptr().cast(),
                buf.len() as u32,
                &mut bytes_read,
                &mut ov,
            )
        };

        if ok == 0 {
            return Err(BlockDeviceError::Io {
                device: self.info.path.clone(),
                offset,
                source: std::io::Error::last_os_error(),
            });
        }

        Ok(bytes_read as usize)
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

// ── Helpers ───────────────────────────────────────────────────────────────────

fn query_geometry(handle: HANDLE, path: &str) -> Result<(u64, u32)> {
    // Size via IOCTL_DISK_GET_LENGTH_INFO
    let mut gli = GetLengthInformation { length: 0 };
    let mut returned = 0u32;
    let ok = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_DISK_GET_LENGTH_INFO,
            std::ptr::null(),
            0,
            &mut gli as *mut _ as *mut _,
            std::mem::size_of::<GetLengthInformation>() as u32,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(BlockDeviceError::Io {
            device: path.to_string(),
            offset: 0,
            source: std::io::Error::last_os_error(),
        });
    }
    let size = gli.length as u64;

    // Sector size via IOCTL_DISK_GET_DRIVE_GEOMETRY
    let mut geo: DiskGeometry = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_DISK_GET_DRIVE_GEOMETRY,
            std::ptr::null(),
            0,
            &mut geo as *mut _ as *mut _,
            std::mem::size_of::<DiskGeometry>() as u32,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    let sector_size = if ok != 0 { geo.bytes_per_sector } else { 512 };

    Ok((size, sector_size))
}

fn query_storage_property(handle: HANDLE) -> (Option<String>, Option<String>) {
    let query = STORAGE_PROPERTY_QUERY {
        PropertyId: StorageDeviceProperty,
        QueryType: PropertyStandardQuery,
        AdditionalParameters: [0],
    };

    let mut buf = vec![0u8; 1024];
    let mut returned = 0u32;

    let ok = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_STORAGE_QUERY_PROPERTY,
            &query as *const _ as *const _,
            std::mem::size_of::<STORAGE_PROPERTY_QUERY>() as u32,
            buf.as_mut_ptr().cast(),
            buf.len() as u32,
            &mut returned,
            std::ptr::null_mut(),
        )
    };

    if ok == 0 || returned < std::mem::size_of::<StorageDeviceDescriptorHeader>() as u32 {
        return (None, None);
    }

    // SAFETY: buffer is large enough for the header (checked above).
    let hdr = unsafe { &*(buf.as_ptr() as *const StorageDeviceDescriptorHeader) };
    let model = extract_ascii_string(&buf, hdr.product_id_offset as usize);
    let serial = extract_ascii_string(&buf, hdr.serial_number_offset as usize);
    (model, serial)
}

fn extract_ascii_string(buf: &[u8], offset: usize) -> Option<String> {
    if offset == 0 || offset >= buf.len() {
        return None;
    }
    let slice = &buf[offset..];
    let len = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
    let s = std::str::from_utf8(&slice[..len]).ok()?.trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

// ── Device enumeration ────────────────────────────────────────────────────────

/// Probe `\\.\PhysicalDrive0` – `\\.\PhysicalDrive31` and return paths that
/// open successfully.
pub fn enumerate_devices() -> Vec<String> {
    (0u32..32)
        .filter_map(|i| {
            let path = format!(r"\\.\PhysicalDrive{i}");
            let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
            // SAFETY: wide is a valid null-terminated UTF-16 path.
            let h = unsafe {
                CreateFileW(
                    wide.as_ptr(),
                    GENERIC_READ,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    std::ptr::null_mut(),
                    OPEN_EXISTING,
                    FILE_FLAG_NO_BUFFERING,
                    std::ptr::null_mut(),
                )
            };
            if h == INVALID_HANDLE_VALUE {
                None
            } else {
                unsafe { CloseHandle(h) };
                Some(path)
            }
        })
        .collect()
}

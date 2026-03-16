use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::sync::Mutex;

use ferrite_core::types::DeviceInfo;
use tracing::instrument;

use crate::{AlignedBuffer, BlockDevice, BlockDeviceError, Result};

/// A [`BlockDevice`] backed by a regular file.
///
/// Intended for testing and for imaging to/from disk image files.
/// Uses `Mutex<File>` so `read_at` can seek on a shared `&self` reference.
pub struct FileBlockDevice {
    inner: Mutex<File>,
    info: DeviceInfo,
}

impl FileBlockDevice {
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let path = path.as_ref();
        let file = File::open(path).map_err(|e| map_open_err(path, e))?;
        let size = file
            .metadata()
            .map_err(|e| BlockDeviceError::Io {
                device: path.display().to_string(),
                offset: 0,
                source: e,
            })?
            .len();

        let info = DeviceInfo {
            path: path.display().to_string(),
            model: None,
            serial: None,
            size_bytes: size,
            sector_size: 512,
            logical_sector_size: 512,
        };
        Ok(Self {
            inner: Mutex::new(file),
            info,
        })
    }
}

fn map_open_err(path: &std::path::Path, e: std::io::Error) -> BlockDeviceError {
    match e.kind() {
        std::io::ErrorKind::NotFound => BlockDeviceError::NotFound(path.display().to_string()),
        std::io::ErrorKind::PermissionDenied => {
            BlockDeviceError::PermissionDenied(path.display().to_string())
        }
        _ => BlockDeviceError::Io {
            device: path.display().to_string(),
            offset: 0,
            source: e,
        },
    }
}

impl BlockDevice for FileBlockDevice {
    #[instrument(skip(self, buf), fields(device = %self.info.path))]
    fn read_at(&self, offset: u64, buf: &mut AlignedBuffer) -> Result<usize> {
        let device = &self.info.path;
        let mut file = self.inner.lock().unwrap();
        file.seek(SeekFrom::Start(offset))
            .map_err(|e| BlockDeviceError::Io {
                device: device.clone(),
                offset,
                source: e,
            })?;
        let n = file
            .read(buf.as_mut_slice())
            .map_err(|e| BlockDeviceError::Io {
                device: device.clone(),
                offset,
                source: e,
            })?;
        Ok(n)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn make_device(data: &[u8]) -> (FileBlockDevice, NamedTempFile) {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(data).unwrap();
        f.flush().unwrap();
        let dev = FileBlockDevice::open(f.path()).unwrap();
        (dev, f)
    }

    #[test]
    fn read_from_start() {
        let (dev, _f) = make_device(&[0xAA_u8; 512]);
        let mut buf = AlignedBuffer::new(512, 512);
        let n = dev.read_at(0, &mut buf).unwrap();
        assert_eq!(n, 512);
        assert!(buf.as_slice().iter().all(|&b| b == 0xAA));
    }

    #[test]
    fn read_at_sector_offset() {
        let mut data = vec![0x00_u8; 1024];
        data[512..].fill(0xBB);
        let (dev, _f) = make_device(&data);
        let mut buf = AlignedBuffer::new(512, 512);
        dev.read_at(512, &mut buf).unwrap();
        assert!(buf.as_slice().iter().all(|&b| b == 0xBB));
    }

    #[test]
    fn read_past_eof_returns_zero() {
        let (dev, _f) = make_device(&[0u8; 512]);
        let mut buf = AlignedBuffer::new(512, 512);
        let n = dev.read_at(512, &mut buf).unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn size_matches_file_length() {
        let (dev, _f) = make_device(&[0u8; 4096]);
        assert_eq!(dev.size(), 4096);
    }

    #[test]
    fn not_found_returns_error() {
        let res = FileBlockDevice::open("/nonexistent/__ferrite_test__.img");
        assert!(matches!(res, Err(BlockDeviceError::NotFound(_))));
    }
}

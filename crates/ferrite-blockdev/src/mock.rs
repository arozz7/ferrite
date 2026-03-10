use std::collections::HashMap;
use std::sync::RwLock;

use ferrite_core::types::DeviceInfo;

use crate::{AlignedBuffer, BlockDevice, BlockDeviceError, Result};

/// Error injection policy for a single sector.
#[derive(Debug, Clone)]
pub enum ErrorPolicy {
    /// Always fail reads to this sector.
    AlwaysFail,
    /// Fail the first `n` reads then succeed.
    FailFirstN(u32),
}

/// A [`BlockDevice`] backed by an in-memory buffer with configurable
/// per-sector error injection.
///
/// Used in unit tests to simulate bad sectors without real hardware.
pub struct MockBlockDevice {
    data: Vec<u8>,
    sector_size: u32,
    info: DeviceInfo,
    /// Sector index → (policy, read-failure count so far).
    errors: RwLock<HashMap<u64, (ErrorPolicy, u32)>>,
}

impl MockBlockDevice {
    pub fn new(data: Vec<u8>, sector_size: u32) -> Self {
        let size = data.len() as u64;
        let info = DeviceInfo {
            path: "mock://device".to_string(),
            model: Some("MockDrive".to_string()),
            serial: Some("MOCK-0000".to_string()),
            size_bytes: size,
            sector_size,
            logical_sector_size: sector_size,
        };
        Self {
            data,
            sector_size,
            info,
            errors: RwLock::new(HashMap::new()),
        }
    }

    /// Create a zeroed device of `size` bytes.
    pub fn zeroed(size: usize, sector_size: u32) -> Self {
        Self::new(vec![0u8; size], sector_size)
    }

    /// Inject an error policy for the sector containing `byte_offset`.
    pub fn inject_error(&self, byte_offset: u64, policy: ErrorPolicy) {
        let sector = byte_offset / self.sector_size as u64;
        self.errors.write().unwrap().insert(sector, (policy, 0));
    }

    /// Remove all injected errors.
    pub fn clear_errors(&self) {
        self.errors.write().unwrap().clear();
    }

    /// Write `data` into the backing buffer starting at `sector` (for test setup).
    pub fn write_sector(&mut self, sector: u64, data: &[u8]) {
        let start = (sector * self.sector_size as u64) as usize;
        let end = (start + data.len()).min(self.data.len());
        self.data[start..end].copy_from_slice(&data[..end - start]);
    }
}

impl BlockDevice for MockBlockDevice {
    fn read_at(&self, offset: u64, buf: &mut AlignedBuffer) -> Result<usize> {
        let sector = offset / self.sector_size as u64;

        {
            let mut errors = self.errors.write().unwrap();
            if let Some((policy, count)) = errors.get_mut(&sector) {
                match policy {
                    ErrorPolicy::AlwaysFail => {
                        return Err(BlockDeviceError::Simulated(offset));
                    }
                    ErrorPolicy::FailFirstN(n) => {
                        if count < n {
                            *count += 1;
                            return Err(BlockDeviceError::Simulated(offset));
                        }
                    }
                }
            }
        }

        let start = offset as usize;
        if start >= self.data.len() {
            return Ok(0);
        }
        let end = (start + buf.len()).min(self.data.len());
        let n = end - start;
        buf.as_mut_slice()[..n].copy_from_slice(&self.data[start..end]);
        Ok(n)
    }

    fn size(&self) -> u64 {
        self.info.size_bytes
    }

    fn sector_size(&self) -> u32 {
        self.sector_size
    }

    fn device_info(&self) -> &DeviceInfo {
        &self.info
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_correct_data() {
        let mut dev = MockBlockDevice::zeroed(4096, 512);
        dev.write_sector(2, &[0xCC_u8; 512]);
        let mut buf = AlignedBuffer::new(512, 512);
        dev.read_at(1024, &mut buf).unwrap();
        assert!(buf.as_slice().iter().all(|&b| b == 0xCC));
    }

    #[test]
    fn always_fail_injection() {
        let dev = MockBlockDevice::zeroed(4096, 512);
        dev.inject_error(0, ErrorPolicy::AlwaysFail);
        let mut buf = AlignedBuffer::new(512, 512);
        assert!(matches!(
            dev.read_at(0, &mut buf),
            Err(BlockDeviceError::Simulated(0))
        ));
        assert!(dev.read_at(512, &mut buf).is_ok());
    }

    #[test]
    fn fail_first_n_then_succeed() {
        let dev = MockBlockDevice::zeroed(4096, 512);
        dev.inject_error(0, ErrorPolicy::FailFirstN(2));
        let mut buf = AlignedBuffer::new(512, 512);
        assert!(dev.read_at(0, &mut buf).is_err()); // fail 1
        assert!(dev.read_at(0, &mut buf).is_err()); // fail 2
        assert!(dev.read_at(0, &mut buf).is_ok()); // pass
    }

    #[test]
    fn read_past_end_returns_zero() {
        let dev = MockBlockDevice::zeroed(512, 512);
        let mut buf = AlignedBuffer::new(512, 512);
        assert_eq!(dev.read_at(512, &mut buf).unwrap(), 0);
    }

    #[test]
    fn clear_errors_restores_reads() {
        let dev = MockBlockDevice::zeroed(4096, 512);
        dev.inject_error(0, ErrorPolicy::AlwaysFail);
        dev.clear_errors();
        let mut buf = AlignedBuffer::new(512, 512);
        assert!(dev.read_at(0, &mut buf).is_ok());
    }
}

use serde::{Deserialize, Serialize};

/// A logical sector index (LBA).
pub type Sector = u64;

/// A contiguous range of sectors `[start, end)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SectorRange {
    pub start: Sector,
    pub end: Sector,
}

impl SectorRange {
    pub fn new(start: Sector, end: Sector) -> Self {
        assert!(end >= start, "SectorRange: end must be >= start");
        Self { start, end }
    }

    pub fn len(&self) -> u64 {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.end == self.start
    }
}

/// Human-readable byte size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ByteSize(pub u64);

impl std::fmt::Display for ByteSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        const KIB: u64 = 1024;
        const MIB: u64 = 1024 * KIB;
        const GIB: u64 = 1024 * MIB;
        const TIB: u64 = 1024 * GIB;

        let n = self.0;
        if n >= TIB {
            write!(f, "{:.2} TiB", n as f64 / TIB as f64)
        } else if n >= GIB {
            write!(f, "{:.2} GiB", n as f64 / GIB as f64)
        } else if n >= MIB {
            write!(f, "{:.2} MiB", n as f64 / MIB as f64)
        } else if n >= KIB {
            write!(f, "{:.2} KiB", n as f64 / KIB as f64)
        } else {
            write!(f, "{} B", n)
        }
    }
}

/// Static metadata about a block device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub path: String,
    pub model: Option<String>,
    pub serial: Option<String>,
    pub size_bytes: u64,
    pub sector_size: u32,
    pub logical_sector_size: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sector_range_len() {
        let r = SectorRange::new(10, 20);
        assert_eq!(r.len(), 10);
    }

    #[test]
    fn byte_size_display() {
        assert_eq!(ByteSize(1024).to_string(), "1.00 KiB");
        assert_eq!(ByteSize(1536).to_string(), "1.50 KiB");
        assert_eq!(ByteSize(1_073_741_824).to_string(), "1.00 GiB");
    }
}

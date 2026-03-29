use crate::{ImagingError, Result};

/// Configuration for an imaging run.
#[derive(Debug, Clone)]
pub struct ImagingConfig {
    /// Block size for the copy pass (bytes). Must be a multiple of sector size.
    /// Default: 512 KiB.
    pub copy_block_size: u64,

    /// Maximum retry attempts per bad sector in the retry pass. Default: 3.
    pub max_retries: u32,

    /// Mapfile auto-save interval. Default: 30 s. Set to `Duration::MAX` to
    /// disable automatic saving (useful in tests).
    pub mapfile_save_interval: std::time::Duration,

    /// Path for the output image file. Required.
    pub output_path: std::path::PathBuf,

    /// Path for the mapfile. `None` disables persistence (test mode).
    pub mapfile_path: Option<std::path::PathBuf>,

    /// Start imaging at this LBA (inclusive). `None` = beginning of device.
    pub start_lba: Option<u64>,
    /// Stop imaging at this LBA (exclusive). `None` = end of device.
    pub end_lba: Option<u64>,

    /// When `true`, the copy pass reads from the end of the device toward the
    /// beginning.  Useful when bad sectors are concentrated at the start of the
    /// disk (e.g., a corrupted partition table area) and data is at the end.
    /// Default: `false`.
    pub reverse: bool,

    /// When `true`, the output file is opened as a sparse file: all-zero blocks
    /// are skipped rather than written, and the OS leaves those ranges as
    /// unallocated holes.  The file reads back as zeros for those regions.
    ///
    /// Requires a sparse-capable destination filesystem (NTFS, ext4, XFS, APFS,
    /// …).  On FAT32 / exFAT destinations the OS silently falls back to dense
    /// allocation.  Default: `true`.
    pub sparse_output: bool,
}

impl ImagingConfig {
    /// Validate config fields against the device's sector size.
    pub fn validate(&self, sector_size: u32) -> Result<()> {
        let ss = sector_size as u64;
        if self.copy_block_size == 0 {
            return Err(ImagingError::MapfileParse {
                line: 0,
                message: "copy_block_size must be > 0".into(),
            });
        }
        if !self.copy_block_size.is_multiple_of(ss) {
            return Err(ImagingError::MapfileParse {
                line: 0,
                message: format!(
                    "copy_block_size {:#x} is not a multiple of sector size {ss}",
                    self.copy_block_size
                ),
            });
        }
        if let (Some(start), Some(end)) = (self.start_lba, self.end_lba) {
            if start >= end {
                return Err(ImagingError::MapfileParse {
                    line: 0,
                    message: format!("start_lba {start} must be less than end_lba {end}"),
                });
            }
        }
        Ok(())
    }
}

impl Default for ImagingConfig {
    fn default() -> Self {
        Self {
            copy_block_size: 512 * 1024,
            max_retries: 3,
            mapfile_save_interval: std::time::Duration::from_secs(30),
            output_path: std::path::PathBuf::new(),
            mapfile_path: None,
            start_lba: None,
            end_lba: None,
            reverse: false,
            sparse_output: true,
        }
    }
}

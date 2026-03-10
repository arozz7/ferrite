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
        }
    }
}

use crate::{ImagingError, Result};

/// Configuration for an imaging run.
#[derive(Debug, Clone)]
pub struct ImagingConfig {
    /// Per-pass read block sizes in bytes.
    ///
    /// Index 0 = Copy, 1 = Trim, 2 = Sweep, 3 = Scrape, 4 = Retry.
    ///
    /// Each value is clamped to `max(pass_block_sizes[n], sector_size)` inside
    /// the pass so you can safely leave lower passes at the sector-size sentinel
    /// (512 B) to get sector-precise recovery.  Larger values trade precision
    /// for speed; the copy pass typically benefits most from a large block size.
    ///
    /// Defaults: [512 KiB, 512 B, 512 B, 512 B, 512 B].
    pub pass_block_sizes: [u64; 5],

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
    /// ...).  On FAT32 / exFAT destinations the OS silently falls back to dense
    /// allocation.  Default: `true`.
    pub sparse_output: bool,

    /// When `true`, each successfully-read block is re-read once and compared.
    /// If the two reads disagree the block is marked `NonTrimmed` rather than
    /// written, flagging it for re-processing by the trim pass.
    ///
    /// Useful for drives that are intermittently returning corrupted data
    /// without raising an I/O error.  Roughly halves copy-pass throughput.
    /// Default: `false`.
    pub verify_reads: bool,

    /// Number of additional read passes used to verify each block when
    /// `verify_reads` is `true`.  Minimum effective value is 1.  Default: 1.
    pub verify_passes: u8,
}

impl ImagingConfig {
    /// Validate config fields against the device's sector size.
    pub fn validate(&self, _sector_size: u32) -> Result<()> {
        for (i, &size) in self.pass_block_sizes.iter().enumerate() {
            if size == 0 {
                return Err(ImagingError::MapfileParse {
                    line: 0,
                    message: format!("pass_block_sizes[{i}] must be > 0"),
                });
            }
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
            pass_block_sizes: [512 * 1024, 512, 512, 512, 512],
            max_retries: 3,
            mapfile_save_interval: std::time::Duration::from_secs(30),
            output_path: std::path::PathBuf::new(),
            mapfile_path: None,
            start_lba: None,
            end_lba: None,
            reverse: false,
            sparse_output: true,
            verify_reads: false,
            verify_passes: 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_pass_block_sizes_are_valid() {
        let cfg = ImagingConfig::default();
        assert_eq!(
            cfg.pass_block_sizes[0],
            512 * 1024,
            "copy pass default 512 KiB"
        );
        for i in 1..5 {
            assert_eq!(
                cfg.pass_block_sizes[i], 512,
                "pass {i} default sector-size sentinel"
            );
        }
        cfg.validate(512).expect("default config must be valid");
    }

    #[test]
    fn validate_rejects_zero_block_size() {
        let mut cfg = ImagingConfig::default();
        cfg.pass_block_sizes[2] = 0;
        assert!(cfg.validate(512).is_err());
    }
}

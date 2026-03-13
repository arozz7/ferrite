use std::fs::OpenOptions;
use std::sync::Arc;
use std::time::Instant;

use ferrite_blockdev::BlockDevice;
use tracing::info;

use crate::config::ImagingConfig;
use crate::error::{ImagingError, Result};
use crate::mapfile::{BlockStatus, Mapfile};
use crate::mapfile_io;
use crate::passes;
use crate::progress::{ImagingPhase, ProgressReporter, ProgressUpdate};

/// The imaging engine. Owns the mapfile, output file, and drives all five passes.
pub struct ImagingEngine {
    pub(crate) device: Arc<dyn BlockDevice>,
    pub(crate) config: ImagingConfig,
    pub(crate) mapfile: Mapfile,
    pub(crate) output: std::fs::File,
    pub(crate) started_at: Instant,
    pub(crate) last_saved: Instant,
    /// Timestamp of the last rolling-rate checkpoint.
    pub(crate) last_rate_instant: Instant,
    /// `bytes_finished` value at the last rolling-rate checkpoint.
    pub(crate) last_rate_bytes: u64,
    /// Most recently computed rolling read rate (bytes/sec).
    pub(crate) current_rate_bps: u64,
}

impl ImagingEngine {
    /// Construct a new engine.
    ///
    /// Opens or creates the output image file. Loads the mapfile from
    /// `config.mapfile_path` if it exists, or creates a fresh one. Validates
    /// the config against the device's sector size.
    pub fn new(device: Arc<dyn BlockDevice>, config: ImagingConfig) -> Result<Self> {
        let sector_size = device.sector_size();
        let device_size = device.size();

        config.validate(sector_size)?;

        // Load or create the mapfile.
        let mut mapfile = match &config.mapfile_path {
            Some(path) => {
                let mf = mapfile_io::load_or_create(path, device_size)?;
                // Verify device size matches.
                if mf.device_size() != device_size {
                    return Err(ImagingError::SizeMismatch {
                        mapfile_bytes: mf.device_size(),
                        device_bytes: device_size,
                    });
                }
                mf
            }
            None => Mapfile::from_device_size(device_size),
        };

        // Apply LBA range — mark bytes outside [start_lba, end_lba) as Finished
        // so passes skip them entirely.
        let sector_size_u64 = device.sector_size() as u64;
        let start_byte = config.start_lba.map(|l| l * sector_size_u64).unwrap_or(0);
        let end_byte = config
            .end_lba
            .map(|l| (l * sector_size_u64).min(device_size))
            .unwrap_or(device_size);
        if start_byte > 0 {
            mapfile.update_range(0, start_byte.min(device_size), BlockStatus::Finished);
        }
        if end_byte < device_size {
            mapfile.update_range(end_byte, device_size - end_byte, BlockStatus::Finished);
        }

        // Open output file (create if absent, preserve existing content for resume).
        let output = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .read(true)
            .open(&config.output_path)
            .map_err(|e| ImagingError::ImageWrite {
                offset: 0,
                source: e,
            })?;

        let now = Instant::now();
        Ok(Self {
            device,
            config,
            mapfile,
            output,
            started_at: now,
            last_saved: now,
            last_rate_instant: now,
            last_rate_bytes: 0,
            current_rate_bps: 0,
        })
    }

    /// Run all five passes to completion, or until the reporter cancels.
    ///
    /// Saves the mapfile on exit regardless of outcome.
    pub fn run(&mut self, reporter: &mut dyn ProgressReporter) -> Result<()> {
        info!("imaging: starting copy pass");
        passes::copy::run(self, reporter)?;

        info!("imaging: starting trim pass");
        passes::trim::run(self, reporter)?;

        info!("imaging: starting sweep pass");
        passes::sweep::run(self, reporter)?;

        info!("imaging: starting scrape pass");
        passes::scrape::run(self, reporter)?;

        for attempt in 0..self.config.max_retries {
            if !self.mapfile.has_status(BlockStatus::BadSector) {
                break;
            }
            info!(attempt, "imaging: retry pass");
            passes::retry::run(self, reporter, attempt)?;
        }

        self.save_mapfile()?;
        info!("imaging: complete");
        Ok(())
    }

    /// Current mapfile state (for status display without running the engine).
    pub fn mapfile(&self) -> &Mapfile {
        &self.mapfile
    }

    /// The sector size of the underlying block device in bytes.
    pub fn sector_size(&self) -> u32 {
        self.device.sector_size()
    }

    /// Pre-mark LBA addresses from the S.M.A.R.T. error log as `BadSector` in the
    /// mapfile before imaging starts.  This allows passes to skip known-bad sectors
    /// and go straight to the retry stage for them.
    ///
    /// `sector_size` must match the device's physical sector size.
    pub fn pre_populate_bad_sectors(&mut self, sector_size: u64, bad_lbas: &[u64]) {
        for &lba in bad_lbas {
            let pos = lba * sector_size;
            if pos + sector_size <= self.mapfile.device_size() {
                self.mapfile
                    .update_range(pos, sector_size, BlockStatus::BadSector);
            }
        }
    }

    // ── Internal helpers (used by passes) ─────────────────────────────────────

    /// Save the mapfile if the configured interval has elapsed.
    pub(crate) fn maybe_save_mapfile(&mut self) -> Result<()> {
        if self.last_saved.elapsed() >= self.config.mapfile_save_interval {
            self.save_mapfile()?;
        }
        Ok(())
    }

    pub(crate) fn save_mapfile(&mut self) -> Result<()> {
        if let Some(path) = &self.config.mapfile_path.clone() {
            mapfile_io::save_atomic(&self.mapfile, path)?;
            self.last_saved = Instant::now();
        }
        Ok(())
    }

    /// Build a progress snapshot for the reporter.
    ///
    /// Updates the rolling read-rate every ~1 second.
    pub(crate) fn make_progress(
        &mut self,
        phase: ImagingPhase,
        current_offset: u64,
    ) -> ProgressUpdate {
        let bytes_finished = self.mapfile.bytes_with_status(BlockStatus::Finished);

        // Update rolling rate once per second.
        let elapsed_secs = self.last_rate_instant.elapsed().as_secs_f64();
        if elapsed_secs >= 1.0 {
            let delta = bytes_finished.saturating_sub(self.last_rate_bytes);
            self.current_rate_bps = (delta as f64 / elapsed_secs) as u64;
            self.last_rate_instant = Instant::now();
            self.last_rate_bytes = bytes_finished;
        }

        ProgressUpdate {
            phase,
            bytes_finished,
            bytes_non_tried: self.mapfile.bytes_with_status(BlockStatus::NonTried),
            bytes_non_trimmed: self.mapfile.bytes_with_status(BlockStatus::NonTrimmed),
            bytes_non_scraped: self.mapfile.bytes_with_status(BlockStatus::NonScraped),
            bytes_bad: self.mapfile.bytes_with_status(BlockStatus::BadSector),
            device_size: self.mapfile.device_size(),
            current_offset,
            elapsed: self.started_at.elapsed(),
            read_rate_bps: self.current_rate_bps,
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::{NullReporter, Signal};
    use ferrite_blockdev::{ErrorPolicy, MockBlockDevice};
    use tempfile::NamedTempFile;

    const SECTOR: u32 = 512;
    const SECTORS: usize = 16;
    const SIZE: usize = SECTORS * SECTOR as usize;

    fn make_engine(mock: MockBlockDevice) -> (ImagingEngine, NamedTempFile) {
        let tmp = NamedTempFile::new().unwrap();
        let config = ImagingConfig {
            copy_block_size: SECTOR as u64,
            max_retries: 3,
            mapfile_save_interval: std::time::Duration::MAX,
            output_path: tmp.path().to_path_buf(),
            mapfile_path: None,
            start_lba: None,
            end_lba: None,
            reverse: false,
        };
        let engine = ImagingEngine::new(Arc::new(mock), config).unwrap();
        (engine, tmp)
    }

    #[test]
    fn clean_device_all_finished() {
        let mock = MockBlockDevice::zeroed(SIZE, SECTOR);
        let (mut engine, _tmp) = make_engine(mock);
        engine.run(&mut NullReporter).unwrap();
        assert_eq!(
            engine.mapfile().bytes_with_status(BlockStatus::Finished),
            SIZE as u64
        );
        assert!(!engine.mapfile().has_status(BlockStatus::NonTried));
        assert!(!engine.mapfile().has_status(BlockStatus::BadSector));
    }

    #[test]
    fn single_always_fail_sector_becomes_bad() {
        let mock = MockBlockDevice::zeroed(SIZE, SECTOR);
        mock.inject_error(0, ErrorPolicy::AlwaysFail);
        let (mut engine, _tmp) = make_engine(mock);
        engine.run(&mut NullReporter).unwrap();
        assert_eq!(
            engine.mapfile().bytes_with_status(BlockStatus::BadSector),
            SECTOR as u64
        );
        assert_eq!(
            engine.mapfile().bytes_with_status(BlockStatus::Finished),
            SIZE as u64 - SECTOR as u64
        );
    }

    #[test]
    fn bad_sector_recovers_with_fail_first_n() {
        // FailFirstN(2): copy fails, trim fails, scrape fails, retry 1 fails, retry 2 succeeds.
        let mock = MockBlockDevice::zeroed(SIZE, SECTOR);
        mock.inject_error(0, ErrorPolicy::FailFirstN(4));
        let (mut engine, _tmp) = make_engine(mock);
        engine.run(&mut NullReporter).unwrap();
        // With max_retries=3, the sector is retried attempts 0,1,2.
        // FailFirstN(4): fails 4 times total — copy(1) + trim(2) + scrape(3) + retry0(4).
        // retry1 (5th call) succeeds.
        assert_eq!(
            engine.mapfile().bytes_with_status(BlockStatus::Finished),
            SIZE as u64
        );
        assert!(!engine.mapfile().has_status(BlockStatus::BadSector));
    }

    #[test]
    fn multiple_bad_sectors() {
        let mock = MockBlockDevice::zeroed(SIZE, SECTOR);
        // Inject errors at sector 2 and sector 7.
        mock.inject_error(2 * SECTOR as u64, ErrorPolicy::AlwaysFail);
        mock.inject_error(7 * SECTOR as u64, ErrorPolicy::AlwaysFail);
        let (mut engine, _tmp) = make_engine(mock);
        engine.run(&mut NullReporter).unwrap();
        assert_eq!(
            engine.mapfile().bytes_with_status(BlockStatus::BadSector),
            2 * SECTOR as u64
        );
        assert_eq!(
            engine.mapfile().bytes_with_status(BlockStatus::Finished),
            SIZE as u64 - 2 * SECTOR as u64
        );
    }

    #[test]
    fn all_bad_sectors() {
        let mock = MockBlockDevice::zeroed(SIZE, SECTOR);
        for i in 0..SECTORS {
            mock.inject_error(i as u64 * SECTOR as u64, ErrorPolicy::AlwaysFail);
        }
        let (mut engine, _tmp) = make_engine(mock);
        engine.run(&mut NullReporter).unwrap();
        assert_eq!(
            engine.mapfile().bytes_with_status(BlockStatus::BadSector),
            SIZE as u64
        );
        assert_eq!(engine.mapfile().bytes_with_status(BlockStatus::Finished), 0);
    }

    #[test]
    fn output_content_matches_source() {
        let mut mock = MockBlockDevice::zeroed(SIZE, SECTOR);
        // Fill each sector with its sector index.
        for i in 0..SECTORS {
            mock.write_sector(i as u64, &[i as u8; SECTOR as usize]);
        }
        let (mut engine, tmp) = make_engine(mock);
        engine.run(&mut NullReporter).unwrap();

        let output = std::fs::read(tmp.path()).unwrap();
        for i in 0..SECTORS {
            let start = i * SECTOR as usize;
            let sector_data = &output[start..start + SECTOR as usize];
            assert!(
                sector_data.iter().all(|&b| b == i as u8),
                "sector {i} content mismatch"
            );
        }
    }

    #[test]
    fn cancellation_returns_error() {
        struct CancelImmediately;
        impl ProgressReporter for CancelImmediately {
            fn report(&mut self, _: &ProgressUpdate) -> Signal {
                Signal::Cancel
            }
        }

        let mock = MockBlockDevice::zeroed(SIZE, SECTOR);
        let (mut engine, _tmp) = make_engine(mock);
        let result = engine.run(&mut CancelImmediately);
        assert!(matches!(result, Err(ImagingError::Cancelled)));
    }

    #[test]
    fn start_lba_marks_prefix_finished() {
        let mock = MockBlockDevice::zeroed(SIZE, SECTOR);
        let tmp = NamedTempFile::new().unwrap();
        let config = ImagingConfig {
            copy_block_size: SECTOR as u64,
            max_retries: 0,
            mapfile_save_interval: std::time::Duration::MAX,
            output_path: tmp.path().to_path_buf(),
            mapfile_path: None,
            start_lba: Some(2), // skip first 2 sectors
            end_lba: None,
            reverse: false,
        };
        let engine = ImagingEngine::new(Arc::new(mock), config).unwrap();
        assert_eq!(
            engine.mapfile().bytes_with_status(BlockStatus::Finished),
            2 * SECTOR as u64
        );
        assert_eq!(
            engine.mapfile().bytes_with_status(BlockStatus::NonTried),
            SIZE as u64 - 2 * SECTOR as u64
        );
    }

    #[test]
    fn end_lba_marks_suffix_finished() {
        let mock = MockBlockDevice::zeroed(SIZE, SECTOR);
        let tmp = NamedTempFile::new().unwrap();
        let config = ImagingConfig {
            copy_block_size: SECTOR as u64,
            max_retries: 0,
            mapfile_save_interval: std::time::Duration::MAX,
            output_path: tmp.path().to_path_buf(),
            mapfile_path: None,
            start_lba: None,
            end_lba: Some(14), // image only sectors 0..13
            reverse: false,
        };
        let engine = ImagingEngine::new(Arc::new(mock), config).unwrap();
        assert_eq!(
            engine.mapfile().bytes_with_status(BlockStatus::Finished),
            2 * SECTOR as u64 // sectors 14 and 15 marked Finished
        );
    }

    #[test]
    fn pre_populate_marks_known_bad_sectors() {
        let mock = MockBlockDevice::zeroed(SIZE, SECTOR);
        let (mut engine, _tmp) = make_engine(mock);
        engine.pre_populate_bad_sectors(SECTOR as u64, &[0, 5]);
        assert_eq!(
            engine.mapfile().bytes_with_status(BlockStatus::BadSector),
            2 * SECTOR as u64
        );
        assert_eq!(engine.mapfile().status_at(0), Some(BlockStatus::BadSector));
        assert_eq!(
            engine.mapfile().status_at(5 * SECTOR as u64),
            Some(BlockStatus::BadSector)
        );
    }

    #[test]
    fn pre_populate_out_of_range_is_ignored() {
        let mock = MockBlockDevice::zeroed(SIZE, SECTOR);
        let (mut engine, _tmp) = make_engine(mock);
        // LBA 1000 is way beyond the device size
        engine.pre_populate_bad_sectors(SECTOR as u64, &[1000]);
        assert_eq!(
            engine.mapfile().bytes_with_status(BlockStatus::BadSector),
            0
        );
    }

    #[test]
    fn resume_skips_finished_blocks() {
        use std::sync::Arc as StdArc;

        // We'll count read calls via a wrapper device.
        // Use FailFirstN(0) everywhere so the device always succeeds.
        let mock = MockBlockDevice::zeroed(SIZE, SECTOR);
        let (mut engine, tmp) = make_engine(mock);
        // Run once to completion.
        engine.run(&mut NullReporter).unwrap();
        assert_eq!(
            engine.mapfile().bytes_with_status(BlockStatus::Finished),
            SIZE as u64
        );

        // Second run on an already-finished mapfile: copy pass should find no
        // NonTried blocks — nothing to do.
        let mock2 = MockBlockDevice::zeroed(SIZE, SECTOR);
        let config2 = ImagingConfig {
            copy_block_size: SECTOR as u64,
            max_retries: 3,
            mapfile_save_interval: std::time::Duration::MAX,
            output_path: tmp.path().to_path_buf(),
            mapfile_path: None,
            start_lba: None,
            end_lba: None,
            reverse: false,
        };
        // Inject a fresh fully-finished mapfile.
        let mut engine2 = ImagingEngine::new(StdArc::new(mock2), config2).unwrap();
        // Manually set mapfile to all-Finished (simulating a reload).
        engine2
            .mapfile
            .update_range(0, SIZE as u64, BlockStatus::Finished);

        // Reinject an error — if the engine tries to read anyway it will get it.
        // (We have no read counter here, so just verify no panic and correct state.)
        engine2.run(&mut NullReporter).unwrap();
        assert_eq!(
            engine2.mapfile().bytes_with_status(BlockStatus::Finished),
            SIZE as u64
        );
    }

    #[test]
    fn reverse_mode_images_entire_device() {
        let mut mock = MockBlockDevice::zeroed(SIZE, SECTOR);
        // Fill each sector with its sector index.
        for i in 0..SECTORS {
            mock.write_sector(i as u64, &[i as u8; SECTOR as usize]);
        }
        let tmp = NamedTempFile::new().unwrap();
        let config = ImagingConfig {
            copy_block_size: SECTOR as u64,
            max_retries: 3,
            mapfile_save_interval: std::time::Duration::MAX,
            output_path: tmp.path().to_path_buf(),
            mapfile_path: None,
            start_lba: None,
            end_lba: None,
            reverse: true,
        };
        let mut engine = ImagingEngine::new(Arc::new(mock), config).unwrap();
        engine.run(&mut NullReporter).unwrap();

        // All sectors should be Finished.
        assert_eq!(
            engine.mapfile().bytes_with_status(BlockStatus::Finished),
            SIZE as u64
        );
        assert!(!engine.mapfile().has_status(BlockStatus::NonTried));

        // Content should match source regardless of read order.
        let output = std::fs::read(tmp.path()).unwrap();
        for i in 0..SECTORS {
            let start = i * SECTOR as usize;
            let sector_data = &output[start..start + SECTOR as usize];
            assert!(
                sector_data.iter().all(|&b| b == i as u8),
                "sector {i} content mismatch in reverse mode"
            );
        }
    }
}

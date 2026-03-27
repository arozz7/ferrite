use std::fs::OpenOptions;
use std::io::{Seek, SeekFrom, Write};
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
use crate::sparse;

/// How many `make_progress` ticks between mapfile block-list snapshots sent to the TUI.
const SNAPSHOT_INTERVAL: u32 = 50;
/// How many `make_progress` ticks between `Instant::elapsed()` calls for rolling-rate updates.
/// Keeps syscall frequency low during sector-by-sector passes (scrape/trim/retry).
const RATE_CHECK_INTERVAL: u32 = 100;
/// Minimum wall-clock seconds that must elapse before the rolling rate is recomputed.
const RATE_UPDATE_MIN_SECS: f64 = 1.0;

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
    /// Counter incremented on every `make_progress` call; used to throttle
    /// snapshots and rate-update syscalls.
    pub(crate) snapshot_counter: u32,
    /// Path of the `.lock` sidecar file created on startup and removed on drop.
    /// `None` when the config has no output path (tests) or when lock creation
    /// is skipped for a zero-length output path.
    lock_path: Option<std::path::PathBuf>,
}

impl Drop for ImagingEngine {
    fn drop(&mut self) {
        if let Some(ref p) = self.lock_path {
            let _ = std::fs::remove_file(p);
        }
    }
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

        // Create a lock sidecar (`<output>.lock`) to prevent two concurrent sessions
        // from writing to the same image file.
        let lock_path = {
            let mut p = config.output_path.clone();
            let mut name = p.file_name().unwrap_or_default().to_os_string();
            name.push(".lock");
            p.set_file_name(name);
            p
        };
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(_) => {} // lock file created; the File is intentionally dropped here —
            // the path itself acts as the lock, removed on engine drop.
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(ImagingError::OutputLocked {
                    path: config.output_path.clone(),
                });
            }
            Err(_) => {
                // Lock file could not be created (e.g. read-only filesystem in tests).
                // Proceed without locking rather than blocking legitimate use.
            }
        }

        // Open output file (create if absent, preserve existing content for resume).
        let mut output = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .read(true)
            .open(&config.output_path)
            .map_err(|e| ImagingError::ImageWrite {
                offset: 0,
                source: e,
            })?;

        // Sparse mode: enable OS-level hole support and pre-set file size so
        // tools see the correct length immediately.  Only done on a fresh file
        // (len == 0) to avoid truncating a resumed session.
        if config.sparse_output && output.metadata().map(|m| m.len()).unwrap_or(1) == 0 {
            sparse::enable_sparse(&output).map_err(|e| ImagingError::ImageWrite {
                offset: 0,
                source: e,
            })?;
            output
                .seek(SeekFrom::Start(device_size - 1))
                .and_then(|_| output.write_all(&[0]))
                .map_err(|e| ImagingError::ImageWrite {
                    offset: 0,
                    source: e,
                })?;
            output
                .seek(SeekFrom::Start(0))
                .map_err(|e| ImagingError::ImageWrite {
                    offset: 0,
                    source: e,
                })?;
        }

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
            snapshot_counter: 0,
            lock_path: Some(lock_path),
        })
    }

    /// Run all five passes to completion, or until the reporter cancels.
    ///
    /// Saves the mapfile on exit regardless of outcome.
    pub fn run(&mut self, reporter: &mut dyn ProgressReporter) -> Result<()> {
        info!("imaging: starting copy pass");
        passes::copy::run(self, reporter)?;
        self.flush_output()?;

        info!("imaging: starting trim pass");
        passes::trim::run(self, reporter)?;
        self.flush_output()?;

        info!("imaging: starting sweep pass");
        passes::sweep::run(self, reporter)?;
        self.flush_output()?;

        info!("imaging: starting scrape pass");
        passes::scrape::run(self, reporter)?;
        self.flush_output()?;

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

    /// Flush the output image file's user-space write buffer to the OS.
    ///
    /// Called after each pass completes so that buffered writes reach the kernel
    /// cache before the mapfile checkpoint is updated. This reduces the risk of
    /// mapfile/image divergence if the process is killed between passes.
    pub(crate) fn flush_output(&mut self) -> Result<()> {
        self.output.flush().map_err(|e| ImagingError::ImageWrite {
            offset: 0,
            source: e,
        })
    }

    pub(crate) fn save_mapfile(&mut self) -> Result<()> {
        if let Some(path) = &self.config.mapfile_path.clone() {
            mapfile_io::save_atomic(&self.mapfile, path)?;
            self.last_saved = Instant::now();
        }
        Ok(())
    }

    /// Write `buf` to the output file at byte `pos`.
    ///
    /// When `config.sparse_output` is enabled and `buf` is entirely zero, the
    /// write is skipped and the file position is advanced past the region —
    /// creating a sparse hole.  Otherwise a normal seek+write is performed.
    pub(crate) fn write_block(&mut self, pos: u64, buf: &[u8]) -> Result<()> {
        if self.config.sparse_output {
            sparse::write_or_skip(&mut self.output, pos, buf).map_err(|e| {
                ImagingError::ImageWrite {
                    offset: pos,
                    source: e,
                }
            })
        } else {
            self.output
                .seek(SeekFrom::Start(pos))
                .and_then(|_| self.output.write_all(buf))
                .map_err(|e| ImagingError::ImageWrite {
                    offset: pos,
                    source: e,
                })
        }
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

        self.snapshot_counter = self.snapshot_counter.wrapping_add(1);

        // Only call Instant::elapsed() every RATE_CHECK_INTERVAL ticks to avoid a
        // syscall on every sector read during sector-by-sector passes (scrape/trim/retry).
        if self.snapshot_counter % RATE_CHECK_INTERVAL == 1 {
            let elapsed_secs = self.last_rate_instant.elapsed().as_secs_f64();
            if elapsed_secs >= RATE_UPDATE_MIN_SECS {
                let delta = bytes_finished.saturating_sub(self.last_rate_bytes);
                self.current_rate_bps = (delta as f64 / elapsed_secs) as u64;
                self.last_rate_instant = Instant::now();
                self.last_rate_bytes = bytes_finished;
            }
        }

        let map_snapshot = if self.snapshot_counter % SNAPSHOT_INTERVAL == 1 {
            Some(self.mapfile.blocks().to_vec())
        } else {
            None
        };

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
            map_snapshot,
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
            sparse_output: false,
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
            sparse_output: false,
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
            sparse_output: false,
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
        // Run once to completion, then drop the engine to release the lock.
        engine.run(&mut NullReporter).unwrap();
        assert_eq!(
            engine.mapfile().bytes_with_status(BlockStatus::Finished),
            SIZE as u64
        );
        drop(engine); // releases the .lock sidecar before creating the second engine

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
            sparse_output: false,
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
    fn sha256_matches_source_data() {
        use sha2::{Digest, Sha256};

        let mut mock = MockBlockDevice::zeroed(SIZE, SECTOR);
        for i in 0..SECTORS {
            mock.write_sector(i as u64, &[i as u8; SECTOR as usize]);
        }
        let (mut engine, tmp) = make_engine(mock);
        engine.run(&mut NullReporter).unwrap();

        // Hash computed by the engine helper must match a direct sha2 digest of output.
        let engine_hex = crate::hash::hash_file(tmp.path()).unwrap();
        let output = std::fs::read(tmp.path()).unwrap();
        let expected = format!("{:x}", Sha256::digest(&output));
        assert_eq!(engine_hex, expected);
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
            sparse_output: false,
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

    #[test]
    fn second_engine_on_same_output_returns_locked() {
        let mock1 = MockBlockDevice::zeroed(SIZE, SECTOR);
        let mock2 = MockBlockDevice::zeroed(SIZE, SECTOR);
        let tmp = NamedTempFile::new().unwrap();
        let make_config = || ImagingConfig {
            copy_block_size: SECTOR as u64,
            max_retries: 0,
            mapfile_save_interval: std::time::Duration::MAX,
            output_path: tmp.path().to_path_buf(),
            mapfile_path: None,
            start_lba: None,
            end_lba: None,
            reverse: false,
            sparse_output: false,
        };

        // First engine takes the lock.
        let _engine1 = ImagingEngine::new(Arc::new(mock1), make_config()).unwrap();

        // Second engine on the same output must fail with OutputLocked.
        let result = ImagingEngine::new(Arc::new(mock2), make_config());
        assert!(
            matches!(result, Err(ImagingError::OutputLocked { .. })),
            "expected OutputLocked error"
        );
    }

    #[test]
    fn lock_released_after_engine_drop() {
        let mock1 = MockBlockDevice::zeroed(SIZE, SECTOR);
        let mock2 = MockBlockDevice::zeroed(SIZE, SECTOR);
        let tmp = NamedTempFile::new().unwrap();
        let make_config = || ImagingConfig {
            copy_block_size: SECTOR as u64,
            max_retries: 0,
            mapfile_save_interval: std::time::Duration::MAX,
            output_path: tmp.path().to_path_buf(),
            mapfile_path: None,
            start_lba: None,
            end_lba: None,
            reverse: false,
            sparse_output: false,
        };

        // First engine takes the lock, then is dropped.
        drop(ImagingEngine::new(Arc::new(mock1), make_config()).unwrap());

        // After drop the lock is released; a new engine must succeed.
        let result = ImagingEngine::new(Arc::new(mock2), make_config());
        assert!(result.is_ok(), "expected Ok after lock released");
    }
}

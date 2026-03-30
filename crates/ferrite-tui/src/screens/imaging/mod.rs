//! Screen 3 — Imaging Setup + Progress: configure and run the ddrescue-style
//! imaging engine with live progress updates.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::Arc;
use std::time::Instant;

use chrono::Local;
use crossterm::event::{KeyCode, KeyModifiers};
use ferrite_blockdev::BlockDevice;
use ferrite_imaging::write_blocker;
use ferrite_imaging::{
    space_check,
    thermal::{ThermalEvent, ThermalGuard, ThermalGuardConfig},
    ImagingConfig, ImagingEngine, ProgressReporter, ProgressUpdate, Signal, SpaceInfo,
};

mod render;

// ── Path helpers ──────────────────────────────────────────────────────────────

/// Collapse consecutive backslashes to a single `\`, preserving the `\\.\`
/// and `\\?\` UNC device-path prefixes that Windows uses for raw drives.
fn normalize_path(path: &str) -> String {
    // Preserve \\.\PhysicalDriveN style prefixes verbatim.
    if path.starts_with(r"\\.\") || path.starts_with(r"\\?\") {
        return path.to_string();
    }
    let mut out = String::with_capacity(path.len());
    let mut prev_bs = false;
    for ch in path.chars() {
        if ch == '\\' {
            if !prev_bs {
                out.push(ch);
            }
            prev_bs = true;
        } else {
            out.push(ch);
            prev_bs = false;
        }
    }
    out
}

/// Return `path` unchanged if it does not exist, otherwise append `_1`, `_2`,
/// … before the extension until a non-existing path is found.
fn unique_path(path: &std::path::Path) -> std::path::PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("image");
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("img");
    let parent = path.parent().unwrap_or(std::path::Path::new("."));
    for i in 1u32..=9999 {
        let candidate = parent.join(format!("{stem}_{i}.{ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    path.to_path_buf()
}

// ── Types ─────────────────────────────────────────────────────────────────────

enum ImagingMsg {
    Progress(ProgressUpdate),
    /// Imaging finished successfully; carries the hex-encoded SHA-256 of the
    /// output image (computed immediately after the run, still in the bg thread).
    Done(Option<String>),
    Error(String),
    /// Current drive temperature from the thermal guard thread.
    Temperature(u32),
    /// Drive exceeded 55 °C — imaging paused until it cools.
    ThermalPause,
    /// Drive cooled to ≤ 50 °C — imaging resumed.
    ThermalResume,
}

#[derive(PartialEq, Clone)]
pub(crate) enum ImagingStatus {
    Idle,
    Running,
    Complete,
    Cancelled,
    Error(String),
    /// Destination has insufficient free space — ask the user to confirm before
    /// proceeding.
    ConfirmLowSpace {
        available: u64,
        required: u64,
    },
    /// The destination image exists and was created from a different drive.
    /// Holds the identity stored in the sidecar vs. the currently connected drive.
    ConfirmDriveMismatch {
        sidecar_serial: String,
        sidecar_model: String,
        sidecar_size: u64,
        current_serial: String,
        current_model: String,
        current_size: u64,
    },
}

// ── Drive identity sidecar ────────────────────────────────────────────────────

/// Tiny sidecar written alongside the destination image that records which
/// drive the image was created from.  Stored as `{dest}.ferrite-id.json`.
#[derive(serde::Serialize, serde::Deserialize)]
struct DriveIdentity {
    serial: String,
    model: String,
    size_bytes: u64,
}

impl DriveIdentity {
    fn sidecar_path(dest: &str) -> String {
        format!("{dest}.ferrite-id.json")
    }

    fn load(dest: &str) -> Option<Self> {
        let text = std::fs::read_to_string(Self::sidecar_path(dest)).ok()?;
        serde_json::from_str(&text).ok()
    }

    fn save(&self, dest: &str) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(Self::sidecar_path(dest), json);
        }
    }
}

/// Which text field is being edited.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum EditField {
    Dest,
    Mapfile,
    StartLba,
    EndLba,
    BlockSize,
}

/// `ProgressReporter` impl that forwards updates through a sync channel.
///
/// When `pause` (thermal) or `user_pause` (manual) is set, `report` spin-waits
/// until the flag is cleared or the user cancels.
struct ChannelReporter {
    tx: SyncSender<ImagingMsg>,
    cancel: Arc<AtomicBool>,
    /// Thermal pause flag — set by the thermal guard thread.
    pause: Arc<AtomicBool>,
    /// Manual pause flag — set by the user pressing `p`.
    user_pause: Arc<AtomicBool>,
}

impl ProgressReporter for ChannelReporter {
    fn report(&mut self, update: &ProgressUpdate) -> Signal {
        let _ = self.tx.try_send(ImagingMsg::Progress(update.clone()));
        // Spin-wait while thermally paused or manually paused; yield to avoid busy-looping.
        while self.pause.load(Ordering::Relaxed) || self.user_pause.load(Ordering::Relaxed) {
            if self.cancel.load(Ordering::Relaxed) {
                return Signal::Cancel;
            }
            std::thread::yield_now();
        }
        if self.cancel.load(Ordering::Relaxed) {
            Signal::Cancel
        } else {
            Signal::Continue
        }
    }
}

// ── State ─────────────────────────────────────────────────────────────────────

pub struct ImagingState {
    pub(crate) device: Option<Arc<dyn BlockDevice>>,
    /// Destination image file path (editable).
    pub dest_path: String,
    /// Mapfile path (editable, empty = no persistence).
    pub mapfile_path: String,
    /// Start LBA (editable, empty = beginning of device).
    pub start_lba_str: String,
    /// End LBA (editable, empty = end of device).
    pub end_lba_str: String,
    /// Copy block size in KiB (editable, empty = default 512 KiB).
    pub block_size_str: String,
    pub(crate) edit_field: Option<EditField>,
    pub(crate) status: ImagingStatus,
    pub(crate) latest: Option<ProgressUpdate>,
    cancel: Arc<AtomicBool>,
    rx: Option<Receiver<ImagingMsg>>,
    /// SHA-256 hex digest of the completed image (set when imaging finishes).
    pub image_sha256: Option<String>,
    /// Most recently reported drive temperature (°C).
    pub current_temp: Option<u32>,
    /// `true` while imaging is paused due to high temperature.
    pub thermal_paused: bool,
    /// Write-blocker status: `None` = not checked yet, `Some(true)` = blocked (safe),
    /// `Some(false)` = WARNING: write access was granted.
    pub write_blocked: Option<bool>,
    /// Channel carrying the result of the pre-flight write-blocker check spawned
    /// in `set_device`.  Drained by `tick` into `write_blocked`.
    wb_rx: Option<Receiver<bool>>,
    /// When `true`, the copy pass reads from end to start.
    pub reverse: bool,
    /// When `true`, all-zero blocks are skipped rather than written (sparse
    /// holes).  Default `true`; the user can toggle with `S`.
    pub sparse: bool,
    /// Most recently computed destination free-space info.  `None` while no
    /// device is selected or the path query failed.
    pub space_info: Option<SpaceInfo>,
    /// Latest mapfile block snapshot for sector-map rendering.
    pub(crate) sector_map: Vec<ferrite_imaging::mapfile::Block>,
    /// User-initiated pause flag (shared with the ChannelReporter).
    user_pause: Arc<AtomicBool>,
    /// `true` while the user has manually paused imaging.
    pub user_paused: bool,
    /// `true` when the imaging session is resuming from an existing mapfile.
    pub imaging_resumed: bool,
    /// Instant when any block was last processed (success OR failure).  Resets
    /// on thread-spawn so the watchdog counts from the start even before the
    /// first read completes.  The watchdog fires only when the engine is truly
    /// frozen — not merely working through a run of bad sectors.
    pub(crate) last_attempt_instant: Option<Instant>,
    /// Total bytes processed in any outcome (finished + bad + non_trimmed +
    /// non_scraped) at the last reset — used to detect when the engine stalls.
    pub(crate) last_attempted_bytes: u64,
    /// Last observed `bytes_finished` — kept separately for the stall message
    /// so we can report "X MiB recovered so far" while the timer uses the
    /// broader attempted-bytes metric.
    pub(crate) last_bytes_finished: u64,
    /// Seconds since the last block was processed.  Zero while not running or
    /// paused.  Rendered in the Statistics panel when ≥ 90 s.
    pub(crate) watchdog_secs: u64,
}

impl Default for ImagingState {
    fn default() -> Self {
        Self::new()
    }
}

impl ImagingState {
    pub fn new() -> Self {
        Self {
            device: None,
            dest_path: String::new(),
            mapfile_path: String::new(),
            start_lba_str: String::new(),
            end_lba_str: String::new(),
            block_size_str: String::new(),
            edit_field: None,
            status: ImagingStatus::Idle,
            latest: None,
            cancel: Arc::new(AtomicBool::new(false)),
            rx: None,
            image_sha256: None,
            current_temp: None,
            thermal_paused: false,
            write_blocked: None,
            wb_rx: None,
            reverse: false,
            sparse: true,
            space_info: None,
            sector_map: Vec::new(),
            user_pause: Arc::new(AtomicBool::new(false)),
            user_paused: false,
            imaging_resumed: false,
            last_attempt_instant: None,
            last_attempted_bytes: 0,
            last_bytes_finished: 0,
            watchdog_secs: 0,
        }
    }

    /// `true` while an imaging run is actively in progress (not paused, not done).
    pub fn is_actively_imaging(&self) -> bool {
        self.status == ImagingStatus::Running
    }

    /// Return `Some(dest_path)` when imaging is active AND the destination file
    /// already exists with at least one byte written.  Used by the Partition tab
    /// to prefer reading from the partial image rather than the physical drive.
    pub fn partial_image_path(&self) -> Option<String> {
        if !self.is_actively_imaging() {
            return None;
        }
        if self.dest_path.is_empty() {
            return None;
        }
        match std::fs::metadata(&self.dest_path) {
            Ok(m) if m.len() > 0 => Some(self.dest_path.clone()),
            _ => None,
        }
    }

    /// Recompute `space_info` from the current `dest_path` and device size.
    /// No-op when no device is selected.
    fn refresh_space_info(&mut self) {
        let Some(dev) = &self.device else {
            self.space_info = None;
            return;
        };
        let sector_size = dev.sector_size() as u64;
        let device_size = dev.size();

        let start_lba = self.start_lba_str.trim().parse::<u64>().ok().unwrap_or(0);
        let end_lba = self
            .end_lba_str
            .trim()
            .parse::<u64>()
            .ok()
            .unwrap_or_else(|| device_size / sector_size.max(1));
        let required = end_lba.saturating_sub(start_lba) * sector_size.max(1);

        let dest_str = normalize_path(&self.dest_path);
        let dest_path = if dest_str.is_empty() {
            std::path::PathBuf::from(".")
        } else {
            std::path::PathBuf::from(&dest_str)
        };

        self.space_info = space_check::check(&dest_path, required);
    }

    pub fn set_device(&mut self, device: Arc<dyn BlockDevice>) {
        let device_path = device.device_info().path.clone();
        self.device = Some(device);
        self.status = ImagingStatus::Idle;
        self.latest = None;
        self.cancel.store(false, Ordering::Relaxed);
        self.rx = None;
        self.current_temp = None;
        self.thermal_paused = false;
        self.write_blocked = None;
        self.wb_rx = None;
        self.start_lba_str = String::new();
        self.end_lba_str = String::new();
        self.sector_map = Vec::new();
        self.user_pause.store(false, Ordering::Relaxed);
        self.user_paused = false;
        self.imaging_resumed = false;
        self.last_attempt_instant = None;
        self.last_attempted_bytes = 0;
        self.last_bytes_finished = 0;
        self.watchdog_secs = 0;

        // Pre-flight: check write-blocker status in a background thread so the
        // UI stays responsive.  Result is drained by `tick()`.
        let (wb_tx, wb_rx) = mpsc::sync_channel::<bool>(1);
        self.wb_rx = Some(wb_rx);
        std::thread::spawn(move || {
            let _ = wb_tx.send(write_blocker::check(&device_path));
        });

        self.refresh_space_info();
    }

    /// Returns `true` while the user is typing into a path field.
    pub fn is_editing(&self) -> bool {
        self.edit_field.is_some()
    }

    /// Drain the background imaging channel and the write-blocker pre-flight channel.
    pub fn tick(&mut self) {
        // Drain the pre-flight write-blocker result (available soon after set_device).
        if let Some(wb_rx) = &self.wb_rx {
            if let Ok(blocked) = wb_rx.try_recv() {
                self.write_blocked = Some(blocked);
                self.wb_rx = None;
            }
        }

        // Watchdog: seconds since any block was last processed (success or failure).
        // Resets on finished bytes AND on failed bytes — if the failed counter is
        // ticking up the engine is working normally through bad sectors and should
        // not be flagged.  Only fires when the engine is truly frozen.
        if self.status == ImagingStatus::Running && !self.user_paused && !self.thermal_paused {
            self.watchdog_secs = self
                .last_attempt_instant
                .map(|t| t.elapsed().as_secs())
                .unwrap_or(0);
        } else {
            self.watchdog_secs = 0;
        }

        let rx = match &self.rx {
            Some(r) => r,
            None => return,
        };
        loop {
            match rx.try_recv() {
                Ok(ImagingMsg::Progress(u)) => {
                    if let Some(snapshot) = u.map_snapshot.clone() {
                        self.sector_map = snapshot;
                    }
                    // Reset the watchdog whenever any block is processed —
                    // success (bytes_finished) OR failure (bad/non_trimmed/
                    // non_scraped).  If the Failed counter is ticking up, the
                    // engine is making progress through a bad region and should
                    // not be flagged as frozen.
                    let total_attempted =
                        u.bytes_finished + u.bytes_bad + u.bytes_non_trimmed + u.bytes_non_scraped;
                    if total_attempted > self.last_attempted_bytes {
                        self.last_attempt_instant = Some(Instant::now());
                        self.last_attempted_bytes = total_attempted;
                        self.watchdog_secs = 0;
                    }
                    // Track finished bytes separately for the stall message display.
                    if u.bytes_finished > self.last_bytes_finished {
                        self.last_bytes_finished = u.bytes_finished;
                    }
                    self.latest = Some(u);
                }
                Ok(ImagingMsg::Done(sha256)) => {
                    self.status = ImagingStatus::Complete;
                    self.image_sha256 = sha256;
                    self.thermal_paused = false;
                    self.rx = None;
                    break;
                }
                Ok(ImagingMsg::Error(e)) => {
                    self.status = ImagingStatus::Error(e);
                    self.thermal_paused = false;
                    self.rx = None;
                    break;
                }
                Ok(ImagingMsg::Temperature(t)) => {
                    self.current_temp = Some(t);
                }
                Ok(ImagingMsg::ThermalPause) => {
                    self.thermal_paused = true;
                }
                Ok(ImagingMsg::ThermalResume) => {
                    self.thermal_paused = false;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.rx = None;
                    break;
                }
            }
        }
    }

    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        // Low-space confirmation prompt intercepts all keys.
        if matches!(self.status, ImagingStatus::ConfirmLowSpace { .. }) {
            match code {
                KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                    self.status = ImagingStatus::Idle;
                    self.start_imaging_after_space_ok();
                }
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                    self.status = ImagingStatus::Idle;
                }
                _ => {}
            }
            return;
        }

        // Drive-mismatch confirmation prompt intercepts all keys.
        if matches!(self.status, ImagingStatus::ConfirmDriveMismatch { .. }) {
            match code {
                KeyCode::Char('y') | KeyCode::Char('Y') => self.start_imaging_forced(),
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    self.status = ImagingStatus::Idle;
                }
                _ => {}
            }
            return;
        }

        if let Some(field) = self.edit_field {
            match code {
                KeyCode::Esc | KeyCode::Enter => {
                    self.edit_field = None;
                    // Re-check space whenever the dest path or LBA range is updated.
                    if matches!(
                        field,
                        EditField::Dest | EditField::StartLba | EditField::EndLba
                    ) {
                        self.refresh_space_info();
                    }
                }
                KeyCode::Backspace => {
                    let s = self.field_mut(field);
                    s.pop();
                }
                KeyCode::Char(c) if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT => {
                    let s = self.field_mut(field);
                    s.push(c);
                }
                _ => {}
            }
            return;
        }

        match code {
            KeyCode::Char('d') => self.edit_field = Some(EditField::Dest),
            KeyCode::Char('m') => self.edit_field = Some(EditField::Mapfile),
            KeyCode::Char('l') => self.edit_field = Some(EditField::StartLba),
            KeyCode::Char('e') => self.edit_field = Some(EditField::EndLba),
            KeyCode::Char('b') => self.edit_field = Some(EditField::BlockSize),
            KeyCode::Char('r') => self.reverse = !self.reverse,
            KeyCode::Char('S') => self.sparse = !self.sparse,
            KeyCode::Char('p') => {
                if self.status == ImagingStatus::Running || self.user_paused {
                    if self.user_paused {
                        self.user_pause.store(false, Ordering::Relaxed);
                        self.user_paused = false;
                    } else {
                        self.user_pause.store(true, Ordering::Relaxed);
                        self.user_paused = true;
                    }
                }
            }
            KeyCode::Char('s') => self.start_imaging(),
            KeyCode::Char('c') => self.cancel_imaging(),
            _ => {}
        }
    }

    fn field_mut(&mut self, field: EditField) -> &mut String {
        match field {
            EditField::Dest => &mut self.dest_path,
            EditField::Mapfile => &mut self.mapfile_path,
            EditField::StartLba => &mut self.start_lba_str,
            EditField::EndLba => &mut self.end_lba_str,
            EditField::BlockSize => &mut self.block_size_str,
        }
    }

    fn start_imaging(&mut self) {
        if self.status == ImagingStatus::Running {
            return;
        }
        let device = match &self.device {
            Some(d) => Arc::clone(d),
            None => return,
        };

        // Normalise path separators: collapse \\ → \ (except \\.\  UNC prefix).
        self.dest_path = normalize_path(&self.dest_path);
        self.mapfile_path = normalize_path(&self.mapfile_path);

        // Auto-generate a filename when the user left dest empty or provided
        // only a directory.  Format: <serial>_<YYYYMMDD>.img
        let dest_needs_filename = self.dest_path.is_empty()
            || self.dest_path.ends_with('\\')
            || self.dest_path.ends_with('/')
            || std::path::Path::new(&self.dest_path).is_dir();
        if dest_needs_filename {
            let info = device.device_info();
            let date_str = Local::now().format("%Y%m%d").to_string();
            let raw_serial = info.serial.as_deref().unwrap_or("disk");
            let serial: String = raw_serial
                .trim()
                .chars()
                .map(|c| if c.is_alphanumeric() { c } else { '_' })
                .collect();
            let stem = if serial.trim_matches('_').is_empty() {
                format!("disk_{date_str}")
            } else {
                format!("{serial}_{date_str}")
            };
            let base_dir = if self.dest_path.is_empty() {
                std::path::PathBuf::from(".")
            } else {
                std::path::PathBuf::from(&self.dest_path)
            };
            let img_path = unique_path(&base_dir.join(format!("{stem}.img")));
            // Auto-generate mapfile alongside image if not set.
            if self.mapfile_path.is_empty() {
                self.mapfile_path = unique_path(&img_path.with_extension("map"))
                    .to_string_lossy()
                    .to_string();
            }
            self.dest_path = img_path.to_string_lossy().to_string();
        }

        if self.dest_path.is_empty() {
            self.status = ImagingStatus::Error("Set a destination path first (press d).".into());
            return;
        }

        // Pre-flight space check: re-evaluate now that dest_path is finalised.
        self.refresh_space_info();
        if let Some(si) = self.space_info {
            if !si.sufficient() {
                self.status = ImagingStatus::ConfirmLowSpace {
                    available: si.available,
                    required: si.required,
                };
                return;
            }
        }

        self.start_imaging_after_space_ok();
    }

    /// Continue imaging setup after the space check has been passed (or
    /// overridden by the user).  Performs the drive-identity check and then
    /// hands off to [`Self::start_imaging_forced`].
    fn start_imaging_after_space_ok(&mut self) {
        let device = match &self.device {
            Some(d) => Arc::clone(d),
            None => return,
        };

        // Drive identity check: if the destination already exists and carries a
        // sidecar, confirm the connected drive matches before overwriting/appending.
        if std::path::Path::new(&self.dest_path).exists() {
            if let Some(id) = DriveIdentity::load(&self.dest_path) {
                let info = device.device_info();
                let cur_serial = info.serial.clone().unwrap_or_default();
                let cur_model = info.model.clone().unwrap_or_default();
                let cur_size = info.size_bytes;
                let serial_ok =
                    id.serial.is_empty() || cur_serial.is_empty() || id.serial == cur_serial;
                let size_ok = id.size_bytes == 0 || id.size_bytes == cur_size;
                if !serial_ok || !size_ok {
                    self.status = ImagingStatus::ConfirmDriveMismatch {
                        sidecar_serial: id.serial,
                        sidecar_model: id.model,
                        sidecar_size: id.size_bytes,
                        current_serial: cur_serial,
                        current_model: cur_model,
                        current_size: cur_size,
                    };
                    return;
                }
            }
        }

        self.start_imaging_forced();
    }

    /// Start imaging unconditionally, bypassing the drive identity check.
    /// Called either directly (no sidecar / matching drive) or after the user
    /// confirms a mismatch with `y`.
    fn start_imaging_forced(&mut self) {
        let device = match &self.device {
            Some(d) => Arc::clone(d),
            None => return,
        };

        // Write / overwrite the drive identity sidecar so future starts can verify.
        let info = device.device_info();
        DriveIdentity {
            serial: info.serial.clone().unwrap_or_default(),
            model: info.model.clone().unwrap_or_default(),
            size_bytes: info.size_bytes,
        }
        .save(&self.dest_path);

        // Detect resume: mapfile path is set and the file already exists.
        self.imaging_resumed =
            !self.mapfile_path.is_empty() && std::path::Path::new(&self.mapfile_path).exists();

        self.cancel.store(false, Ordering::Relaxed);
        self.user_pause.store(false, Ordering::Relaxed);
        self.user_paused = false;
        self.sector_map = Vec::new();
        // Watchdog clock starts at thread-spawn so it counts even before the
        // first Progress message (e.g. while the SMART pre-populate query runs).
        self.last_attempt_instant = Some(Instant::now());
        self.last_attempted_bytes = 0;
        self.last_bytes_finished = 0;
        self.watchdog_secs = 0;
        let cancel = Arc::clone(&self.cancel);
        let user_pause_reporter = Arc::clone(&self.user_pause);
        let (tx, rx) = mpsc::sync_channel::<ImagingMsg>(64);
        self.rx = Some(rx);
        self.status = ImagingStatus::Running;
        self.latest = None;
        self.current_temp = None;
        self.thermal_paused = false;
        // write_blocked is intentionally NOT reset here — the pre-flight result
        // from set_device() carries forward into the imaging session.

        let output_path = PathBuf::from(&self.dest_path);
        let copy_block_size = self
            .block_size_str
            .trim()
            .parse::<u64>()
            .ok()
            .filter(|&n| n > 0)
            .map(|kb| kb * 1024) // field is in KiB
            .unwrap_or(512 * 1024); // default 512 KiB
        let config = ImagingConfig {
            output_path: output_path.clone(),
            copy_block_size,
            mapfile_path: if self.mapfile_path.is_empty() {
                None
            } else {
                Some(PathBuf::from(&self.mapfile_path))
            },
            start_lba: self.start_lba_str.trim().parse::<u64>().ok(),
            end_lba: self.end_lba_str.trim().parse::<u64>().ok(),
            reverse: self.reverse,
            sparse_output: self.sparse,
            ..ImagingConfig::default()
        };

        let device_path_for_smart = device.device_info().path.clone();
        let reporter_tx = tx.clone();
        std::thread::spawn(move || {
            let mut engine = match ImagingEngine::new(device, config) {
                Ok(e) => e,
                Err(e) => {
                    let _ = tx.send(ImagingMsg::Error(e.to_string()));
                    return;
                }
            };

            // Pre-populate known-bad sectors from S.M.A.R.T. error log (best-effort).
            //
            // IMPORTANT: smartctl may hang indefinitely on a dead/unresponsive USB drive
            // (Command::output() blocks until the subprocess exits, and the subprocess
            // may be waiting for an ATA response that never arrives).  We run the query
            // in a sub-thread and wait at most 30 s; if it doesn't finish in time we
            // simply skip pre-population and proceed directly to imaging.
            {
                let path = device_path_for_smart.clone();
                let (smart_tx, smart_rx) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    let _ = smart_tx.send(ferrite_smart::query(&path, None));
                });
                if let Ok(Ok(smart_data)) =
                    smart_rx.recv_timeout(std::time::Duration::from_secs(30))
                {
                    if !smart_data.bad_sector_lbas.is_empty() {
                        let ss = engine.sector_size();
                        engine.pre_populate_bad_sectors(ss as u64, &smart_data.bad_sector_lbas);
                    }
                }
            }

            // ── Thermal guard ────────────────────────────────────────────────
            // Polls S.M.A.R.T. every 60 s.  Pauses imaging above 55 °C and
            // resumes after the drive cools to ≤ 50 °C.  Stopped automatically
            // when the guard is dropped at the end of this thread.
            let thermal_tx = tx.clone();
            let smart_path = device_path_for_smart.clone();
            let guard = ThermalGuard::start(
                move || {
                    ferrite_smart::query(&smart_path, None)
                        .ok()
                        .and_then(|d| d.temperature_celsius)
                },
                None, // imaging engine has its own rate throttle; no speed inference needed
                ThermalGuardConfig::default(),
                move |event| match event {
                    ThermalEvent::Temperature(t) => {
                        let _ = thermal_tx.try_send(ImagingMsg::Temperature(t));
                    }
                    ThermalEvent::Paused | ThermalEvent::SpeedThrottle => {
                        let _ = thermal_tx.try_send(ImagingMsg::ThermalPause);
                    }
                    ThermalEvent::Resumed | ThermalEvent::SpeedResumed => {
                        let _ = thermal_tx.try_send(ImagingMsg::ThermalResume);
                    }
                    ThermalEvent::SpeedBaseline(_) => {}
                },
            );

            let mut reporter = ChannelReporter {
                tx: reporter_tx,
                cancel,
                pause: guard.pause_flag(),
                user_pause: user_pause_reporter,
            };
            match engine.run(&mut reporter) {
                Ok(()) => {
                    // hash_and_save computes SHA-256 of the output file and writes
                    // a companion <image>.sha256 sidecar in sha256sum format.
                    let sha256 = ferrite_imaging::hash::hash_and_save(&output_path);
                    let _ = tx.send(ImagingMsg::Done(sha256));
                }
                Err(e) => {
                    let _ = tx.send(ImagingMsg::Error(e.to_string()));
                }
            }
        });
    }

    fn cancel_imaging(&mut self) {
        if self.status == ImagingStatus::Running {
            self.cancel.store(true, Ordering::Relaxed);
            self.status = ImagingStatus::Cancelled;
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_editing_initially_false() {
        let s = ImagingState::new();
        assert!(!s.is_editing());
    }

    #[test]
    fn image_sha256_initially_none() {
        let s = ImagingState::new();
        assert!(s.image_sha256.is_none());
    }

    #[test]
    fn d_key_enters_dest_edit_mode() {
        let mut s = ImagingState::new();
        s.handle_key(KeyCode::Char('d'), KeyModifiers::NONE);
        assert!(s.is_editing());
        assert_eq!(s.edit_field, Some(EditField::Dest));
    }

    #[test]
    fn esc_exits_edit_mode() {
        let mut s = ImagingState::new();
        s.handle_key(KeyCode::Char('d'), KeyModifiers::NONE);
        s.handle_key(KeyCode::Esc, KeyModifiers::NONE);
        assert!(!s.is_editing());
    }

    #[test]
    fn typing_appends_to_dest_path() {
        let mut s = ImagingState::new();
        s.handle_key(KeyCode::Char('d'), KeyModifiers::NONE);
        s.handle_key(KeyCode::Char('/'), KeyModifiers::NONE);
        s.handle_key(KeyCode::Char('t'), KeyModifiers::NONE);
        s.handle_key(KeyCode::Char('m'), KeyModifiers::NONE);
        s.handle_key(KeyCode::Char('p'), KeyModifiers::NONE);
        assert_eq!(s.dest_path, "/tmp");
    }

    #[test]
    fn preflight_wb_rx_sets_write_blocked() {
        let (wb_tx, wb_rx) = mpsc::sync_channel::<bool>(1);
        let mut s = ImagingState::new();
        s.wb_rx = Some(wb_rx);
        wb_tx.send(true).unwrap();
        s.tick();
        assert_eq!(s.write_blocked, Some(true));
    }

    #[test]
    fn preflight_wb_rx_not_blocked_sets_false() {
        let (wb_tx, wb_rx) = mpsc::sync_channel::<bool>(1);
        let mut s = ImagingState::new();
        s.wb_rx = Some(wb_rx);
        wb_tx.send(false).unwrap();
        s.tick();
        assert_eq!(s.write_blocked, Some(false));
    }

    // ── normalize_path tests ──────────────────────────────────────────────────

    #[test]
    fn normalize_path_collapses_double_backslash() {
        assert_eq!(
            normalize_path(r"m:\\restore\\image.img"),
            r"m:\restore\image.img"
        );
    }

    #[test]
    fn normalize_path_single_backslash_unchanged() {
        assert_eq!(
            normalize_path(r"m:\restore\image.img"),
            r"m:\restore\image.img"
        );
    }

    #[test]
    fn normalize_path_preserves_unc_device_prefix() {
        let input = r"\\.\PhysicalDrive9";
        assert_eq!(normalize_path(input), input);
    }

    #[test]
    fn normalize_path_preserves_unc_question_prefix() {
        let input = r"\\?\Volume{abc}";
        assert_eq!(normalize_path(input), input);
    }

    #[test]
    fn normalize_path_empty_string() {
        assert_eq!(normalize_path(""), "");
    }

    // ── unique_path tests ─────────────────────────────────────────────────────

    #[test]
    fn unique_path_returns_original_when_not_exists() {
        let p = std::path::Path::new(r"C:\this_path_definitely_does_not_exist_ferrite_test.img");
        assert_eq!(unique_path(p), p);
    }

    // ── watchdog tests ────────────────────────────────────────────────────────

    #[test]
    fn watchdog_secs_zero_on_new_state() {
        let s = ImagingState::new();
        assert_eq!(s.watchdog_secs, 0);
    }

    #[test]
    fn watchdog_secs_zero_when_idle_after_tick() {
        let mut s = ImagingState::new();
        s.tick(); // status is Idle
        assert_eq!(s.watchdog_secs, 0);
    }

    #[test]
    fn watchdog_resets_to_zero_on_progress_message() {
        let (tx, rx) = mpsc::sync_channel::<ImagingMsg>(8);
        let mut s = ImagingState::new();
        s.status = ImagingStatus::Running;
        s.rx = Some(rx);
        // Simulate last block-attempt being 5 s ago with no blocks processed yet.
        s.last_attempt_instant = Some(Instant::now() - std::time::Duration::from_secs(5));
        // Send a progress update
        let update = ferrite_imaging::ProgressUpdate {
            phase: ferrite_imaging::ImagingPhase::Copy,
            current_offset: 0,
            device_size: 1024,
            bytes_finished: 512,
            bytes_bad: 0,
            bytes_non_tried: 512,
            bytes_non_trimmed: 0,
            bytes_non_scraped: 0,
            read_rate_bps: 1_000_000,
            elapsed: std::time::Duration::from_secs(1),
            map_snapshot: None,
        };
        tx.send(ImagingMsg::Progress(update)).unwrap();
        s.tick();
        assert_eq!(
            s.watchdog_secs, 0,
            "watchdog should reset when progress arrives"
        );
    }

    #[test]
    fn preflight_wb_rx_cleared_after_drain() {
        let (wb_tx, wb_rx) = mpsc::sync_channel::<bool>(1);
        let mut s = ImagingState::new();
        s.wb_rx = Some(wb_rx);
        wb_tx.send(true).unwrap();
        s.tick();
        assert!(
            s.wb_rx.is_none(),
            "wb_rx should be cleared after result received"
        );
    }
}

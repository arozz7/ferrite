//! Screen 3 — Imaging Setup + Progress: configure and run the ddrescue-style
//! imaging engine with live progress updates.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyModifiers};
use ferrite_blockdev::BlockDevice;
use ferrite_imaging::write_blocker;
use ferrite_imaging::{
    thermal::{ThermalEvent, ThermalGuard, ThermalGuardConfig},
    ImagingConfig, ImagingEngine, ProgressReporter, ProgressUpdate, Signal,
};

mod render;

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
    /// Latest mapfile block snapshot for sector-map rendering.
    pub(crate) sector_map: Vec<ferrite_imaging::mapfile::Block>,
    /// User-initiated pause flag (shared with the ChannelReporter).
    user_pause: Arc<AtomicBool>,
    /// `true` while the user has manually paused imaging.
    pub user_paused: bool,
    /// `true` when the imaging session is resuming from an existing mapfile.
    pub imaging_resumed: bool,
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
            sector_map: Vec::new(),
            user_pause: Arc::new(AtomicBool::new(false)),
            user_paused: false,
            imaging_resumed: false,
        }
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

        // Pre-flight: check write-blocker status in a background thread so the
        // UI stays responsive.  Result is drained by `tick()`.
        let (wb_tx, wb_rx) = mpsc::sync_channel::<bool>(1);
        self.wb_rx = Some(wb_rx);
        std::thread::spawn(move || {
            let _ = wb_tx.send(write_blocker::check(&device_path));
        });
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
        if self.dest_path.is_empty() {
            self.status = ImagingStatus::Error("Set a destination path first (press d).".into());
            return;
        }

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
            if let Ok(smart_data) = ferrite_smart::query(&device_path_for_smart, None) {
                if !smart_data.bad_sector_lbas.is_empty() {
                    let ss = engine.sector_size();
                    engine.pre_populate_bad_sectors(ss as u64, &smart_data.bad_sector_lbas);
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
                ThermalGuardConfig::default(),
                move |event| match event {
                    ThermalEvent::Temperature(t) => {
                        let _ = thermal_tx.try_send(ImagingMsg::Temperature(t));
                    }
                    ThermalEvent::Paused => {
                        let _ = thermal_tx.try_send(ImagingMsg::ThermalPause);
                    }
                    ThermalEvent::Resumed => {
                        let _ = thermal_tx.try_send(ImagingMsg::ThermalResume);
                    }
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

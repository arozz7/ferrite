//! Screen 8 — Forensic Artifact Scanner
//!
//! Opt-in scan for PII artifacts (email, URL, CC#, IBAN, Windows path, SSN)
//! across the selected device.  Results are displayed in a scrollable hit
//! list and can be exported to CSV.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::Instant;

use ferrite_artifact::{ArtifactHit, ArtifactKind, ArtifactScanConfig, ScanMsg, ScanProgress};
use ferrite_blockdev::BlockDevice;
use ferrite_core::{ThermalGuard, ThermalGuardConfig};
use ferrite_smart;

mod input;
mod render;

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum ScanStatus {
    Idle,
    Running,
    Done,
    Error,
}

// ── State ─────────────────────────────────────────────────────────────────────

pub struct ArtifactsState {
    pub(crate) device: Option<Arc<dyn BlockDevice>>,
    pub(crate) status: ScanStatus,
    pub(crate) hits: Vec<ArtifactHit>,
    pub(crate) hit_sel: usize,
    /// Cached view: hits filtered by `filter_kind` (indices into `hits`).
    pub(crate) filtered: Vec<usize>,
    pub(crate) progress: Option<ScanProgress>,
    pub(crate) scan_start: Option<Instant>,
    pub(crate) cancel: Arc<AtomicBool>,
    pub(crate) rx: Option<Receiver<ScanMsg>>,
    /// User has explicitly accepted the consent dialog this session.
    pub(crate) consent_given: bool,
    /// Consent dialog is currently visible.
    pub(crate) show_consent: bool,
    /// Optional kind filter — `None` = show all.
    pub(crate) filter_kind: Option<ArtifactKind>,
    /// Directory where the CSV export is written.
    pub(crate) output_dir: String,
    /// Whether the output dir field is being edited.
    pub(crate) editing_dir: bool,
    /// Status line shown after export.
    pub(crate) export_status: Option<String>,
    /// Number of visible rows in the hit list (updated each render for PgUp/PgDn).
    pub(crate) hits_page_size: usize,
    /// Error message when status is `Error`.
    pub(crate) error_msg: String,
    /// Monotonic counter of bytes read by the scan thread; shared with the
    /// thermal guard for speed-based inference.
    pub(crate) bytes_read: Arc<AtomicU64>,
    /// Pause flag shared with the thermal guard and the scan thread.
    pub(crate) pause: Arc<AtomicBool>,
    /// Active thermal guard (held for the duration of the scan).
    pub(crate) thermal_guard: Option<ThermalGuard>,
}

impl Default for ArtifactsState {
    fn default() -> Self {
        Self::new()
    }
}

impl ArtifactsState {
    pub fn new() -> Self {
        Self {
            device: None,
            status: ScanStatus::Idle,
            hits: Vec::new(),
            hit_sel: 0,
            filtered: Vec::new(),
            progress: None,
            scan_start: None,
            cancel: Arc::new(AtomicBool::new(false)),
            rx: None,
            consent_given: false,
            show_consent: false,
            filter_kind: None,
            output_dir: String::new(),
            editing_dir: false,
            export_status: None,
            hits_page_size: 20,
            error_msg: String::new(),
            bytes_read: Arc::new(AtomicU64::new(0)),
            pause: Arc::new(AtomicBool::new(false)),
            thermal_guard: None,
        }
    }

    pub fn set_device(&mut self, device: Arc<dyn BlockDevice>) {
        self.device = Some(device);
        self.hits.clear();
        self.filtered.clear();
        self.hit_sel = 0;
        self.status = ScanStatus::Idle;
        self.progress = None;
        self.cancel.store(false, Ordering::Relaxed);
        self.pause.store(false, Ordering::Relaxed);
        self.bytes_read.store(0, Ordering::Relaxed);
        self.thermal_guard = None;
        self.rx = None;
        self.export_status = None;
        self.error_msg.clear();
    }

    /// Returns `true` while a text-input field is active (so `q` won't quit).
    pub fn is_editing(&self) -> bool {
        self.editing_dir || self.show_consent
    }

    /// Drain the background scan channel and update state.
    pub fn tick(&mut self) {
        // Take `rx` out of `self` so we can mutate `self` freely in the loop.
        let rx = match self.rx.take() {
            Some(r) => r,
            None => return,
        };
        let mut done = false;
        loop {
            match rx.try_recv() {
                Ok(ScanMsg::HitBatch(batch)) => {
                    for hit in batch {
                        let idx = self.hits.len();
                        self.hits.push(hit);
                        if self.filter_kind.is_none()
                            || self.filter_kind == Some(self.hits[idx].kind)
                        {
                            self.filtered.push(idx);
                        }
                    }
                }
                Ok(ScanMsg::Progress(p)) => {
                    self.progress = Some(p);
                }
                Ok(ScanMsg::Done { total_hits: _ }) => {
                    self.status = ScanStatus::Done;
                    done = true;
                    break;
                }
                Ok(ScanMsg::Error(e)) => {
                    self.status = ScanStatus::Error;
                    self.error_msg = e;
                    done = true;
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    if self.status == ScanStatus::Running {
                        self.status = ScanStatus::Done;
                    }
                    done = true;
                    break;
                }
            }
        }
        if !done {
            self.rx = Some(rx);
        }
    }

    // ── Actions ───────────────────────────────────────────────────────────────

    /// Rebuild `filtered` from scratch using the current `filter_kind`.
    pub(crate) fn rebuild_filtered(&mut self) {
        self.filtered = (0..self.hits.len())
            .filter(|&i| self.filter_kind.is_none() || self.filter_kind == Some(self.hits[i].kind))
            .collect();
        let max = self.filtered.len().saturating_sub(1);
        self.hit_sel = self.hit_sel.min(max);
    }

    /// Start a new scan (requires consent already given).
    pub(crate) fn start_scan(&mut self) {
        let device = match &self.device {
            Some(d) => Arc::clone(d),
            None => return,
        };
        self.hits.clear();
        self.filtered.clear();
        self.hit_sel = 0;
        self.progress = None;
        self.export_status = None;
        self.error_msg.clear();
        self.cancel.store(false, Ordering::Relaxed);
        self.pause.store(false, Ordering::Relaxed);
        self.bytes_read.store(0, Ordering::Relaxed);
        self.thermal_guard = None;

        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        self.status = ScanStatus::Running;
        self.scan_start = Some(Instant::now());

        let cancel = Arc::clone(&self.cancel);
        let _pause = Arc::clone(&self.pause);
        let bytes_read = Arc::clone(&self.bytes_read);
        let config = ArtifactScanConfig::default();

        // Thermal guard: SMART + speed-based inference via bytes_read counter.
        let smart_path = device.device_info().path.clone();
        let guard = ThermalGuard::start(
            move || {
                ferrite_smart::query(&smart_path, None)
                    .ok()
                    .and_then(|d| d.temperature_celsius)
            },
            Some(Arc::clone(&bytes_read)),
            ThermalGuardConfig::default(),
            |_event| {}, // pause handled via shared pause flag below
        );
        // Wire the guard's pause flag into the scan's pause param.
        let thermal_pause = guard.pause_flag();
        self.thermal_guard = Some(guard);

        std::thread::spawn(move || {
            // Merge user pause and thermal pause: scan checks both.
            // We pass the thermal guard's pause_flag as the scan pause param.
            // The user-side pause flag (self.pause) is not checked here because
            // artifacts has no user pause UX yet — thermal is the only source.
            ferrite_artifact::run_scan(device, config, tx, cancel, thermal_pause, bytes_read);
        });
    }

    /// Cancel a running scan.
    pub(crate) fn cancel_scan(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Export current hits to CSV in `output_dir`.
    pub(crate) fn export_csv(&mut self) {
        if self.hits.is_empty() {
            self.export_status = Some("Nothing to export — no hits yet.".to_string());
            return;
        }
        let dir = if self.output_dir.is_empty() {
            ".".to_string()
        } else {
            self.output_dir.clone()
        };
        let path = format!("{dir}\\ferrite_artifacts.csv");
        match ferrite_artifact::write_csv(&path, &self.hits) {
            Ok(_) => {
                self.export_status = Some(format!("Exported {} hits to {path}", self.hits.len()));
            }
            Err(e) => {
                self.export_status = Some(format!("Export failed: {e}"));
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_state_is_idle() {
        let s = ArtifactsState::new();
        assert_eq!(s.status, ScanStatus::Idle);
        assert!(s.hits.is_empty());
        assert!(!s.consent_given);
    }

    #[test]
    fn rebuild_filtered_all() {
        let mut s = ArtifactsState::new();
        s.hits.push(ArtifactHit {
            kind: ArtifactKind::Email,
            byte_offset: 0,
            value: "a@b.com".to_string(),
        });
        s.hits.push(ArtifactHit {
            kind: ArtifactKind::Url,
            byte_offset: 10,
            value: "https://x.com".to_string(),
        });
        s.filter_kind = None;
        s.rebuild_filtered();
        assert_eq!(s.filtered.len(), 2);
    }

    #[test]
    fn rebuild_filtered_by_kind() {
        let mut s = ArtifactsState::new();
        s.hits.push(ArtifactHit {
            kind: ArtifactKind::Email,
            byte_offset: 0,
            value: "a@b.com".to_string(),
        });
        s.hits.push(ArtifactHit {
            kind: ArtifactKind::Url,
            byte_offset: 10,
            value: "https://x.com".to_string(),
        });
        s.filter_kind = Some(ArtifactKind::Email);
        s.rebuild_filtered();
        assert_eq!(s.filtered.len(), 1);
        assert_eq!(s.hits[s.filtered[0]].kind, ArtifactKind::Email);
    }

    #[test]
    fn is_editing_false_by_default() {
        let s = ArtifactsState::new();
        assert!(!s.is_editing());
    }

    #[test]
    fn is_editing_true_when_consent_shown() {
        let mut s = ArtifactsState::new();
        s.show_consent = true;
        assert!(s.is_editing());
    }
}

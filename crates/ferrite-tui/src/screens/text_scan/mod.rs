//! Screen 9 — Heuristic Text Block Scanner
//!
//! Opt-in scan that identifies contiguous text regions in the raw device
//! stream, classifies them by content type, and allows bulk export.  Results
//! are variable quality and should be considered supplemental to filesystem-
//! based recovery (Tabs 4 and 7).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::Instant;

use ferrite_blockdev::BlockDevice;
use ferrite_core::{ThermalGuard, ThermalGuardConfig};
use ferrite_smart;
use ferrite_textcarver::{TextBlock, TextKind, TextScanConfig, TextScanMsg, TextScanProgress};
use whatlang::Lang;

mod input;
mod render;

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum ScanStatus {
    Idle,
    Running,
    Done,
    Cancelled,
    Error,
}

// ── State ─────────────────────────────────────────────────────────────────────

pub struct TextScanState {
    pub(crate) device: Option<Arc<dyn BlockDevice>>,
    pub(crate) status: ScanStatus,
    pub(crate) blocks: Vec<TextBlock>,
    pub(crate) block_sel: usize,
    /// Cached view: blocks filtered by `filter_kind` (indices into `blocks`).
    pub(crate) filtered: Vec<usize>,
    pub(crate) progress: Option<TextScanProgress>,
    pub(crate) scan_start: Option<Instant>,
    pub(crate) cancel: Arc<AtomicBool>,
    pub(crate) rx: Option<Receiver<TextScanMsg>>,
    /// User has accepted the consent dialog this session.
    pub(crate) consent_given: bool,
    /// Consent dialog is currently visible.
    pub(crate) show_consent: bool,
    /// Optional kind filter — `None` = show all.
    pub(crate) filter_kind: Option<TextKind>,
    /// Optional language filter — `None` = show all.
    pub(crate) filter_lang: Option<Lang>,
    /// Directory where exported files are written.
    pub(crate) output_dir: String,
    /// Whether the output dir field is being edited.
    pub(crate) editing_dir: bool,
    /// Status line shown after export.
    pub(crate) export_status: Option<String>,
    /// Number of visible rows in the block list (updated each render).
    pub(crate) blocks_page_size: usize,
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

impl Default for TextScanState {
    fn default() -> Self {
        Self::new()
    }
}

impl TextScanState {
    pub fn new() -> Self {
        Self {
            device: None,
            status: ScanStatus::Idle,
            blocks: Vec::new(),
            block_sel: 0,
            filtered: Vec::new(),
            progress: None,
            scan_start: None,
            cancel: Arc::new(AtomicBool::new(false)),
            rx: None,
            consent_given: false,
            show_consent: false,
            filter_kind: None,
            filter_lang: None,
            output_dir: String::new(),
            editing_dir: false,
            export_status: None,
            blocks_page_size: 20,
            error_msg: String::new(),
            bytes_read: Arc::new(AtomicU64::new(0)),
            pause: Arc::new(AtomicBool::new(false)),
            thermal_guard: None,
        }
    }

    pub fn set_device(&mut self, device: Arc<dyn BlockDevice>) {
        self.device = Some(device);
        self.blocks.clear();
        self.filtered.clear();
        self.block_sel = 0;
        self.status = ScanStatus::Idle;
        self.progress = None;
        self.cancel.store(false, Ordering::Relaxed);
        self.pause.store(false, Ordering::Relaxed);
        self.bytes_read.store(0, Ordering::Relaxed);
        self.thermal_guard = None;
        self.rx = None;
        self.export_status = None;
        self.error_msg.clear();
        self.filter_lang = None;
    }

    /// Returns `true` while a text-input field is active (so `q` won't quit).
    pub fn is_editing(&self) -> bool {
        self.editing_dir || self.show_consent
    }

    /// Drain the background scan channel and update state.
    pub fn tick(&mut self) {
        let rx = match self.rx.take() {
            Some(r) => r,
            None => return,
        };
        // Cap per-tick drain so the TUI render thread cannot be blocked for
        // an arbitrarily long time when many batches have queued up (e.g.
        // after switching back from another tab mid-scan).
        const MAX_MSGS_PER_TICK: usize = 100;
        let mut processed = 0;
        let mut done = false;
        loop {
            if processed >= MAX_MSGS_PER_TICK {
                break;
            }
            match rx.try_recv() {
                Ok(TextScanMsg::BlockBatch(batch)) => {
                    for block in batch {
                        let idx = self.blocks.len();
                        self.blocks.push(block);
                        if self.filter_kind.is_none()
                            || self.filter_kind == Some(self.blocks[idx].kind)
                        {
                            self.filtered.push(idx);
                        }
                    }
                    processed += 1;
                }
                Ok(TextScanMsg::Progress(p)) => {
                    self.progress = Some(p);
                    processed += 1;
                }
                Ok(TextScanMsg::Done { total_blocks: _ }) => {
                    self.status = ScanStatus::Done;
                    done = true;
                    break;
                }
                Ok(TextScanMsg::Error(e)) => {
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

    /// Rebuild `filtered` from scratch using the current `filter_kind` and `filter_lang`.
    pub(crate) fn rebuild_filtered(&mut self) {
        self.filtered = (0..self.blocks.len())
            .filter(|&i| {
                (self.filter_kind.is_none() || self.filter_kind == Some(self.blocks[i].kind))
                    && (self.filter_lang.is_none() || self.filter_lang == self.blocks[i].lang)
            })
            .collect();
        let max = self.filtered.len().saturating_sub(1);
        self.block_sel = self.block_sel.min(max);
    }

    /// Start a new scan (requires consent already given).
    pub(crate) fn start_scan(&mut self) {
        let device = match &self.device {
            Some(d) => Arc::clone(d),
            None => return,
        };
        self.blocks.clear();
        self.filtered.clear();
        self.block_sel = 0;
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
        let bytes_read = Arc::clone(&self.bytes_read);
        let config = TextScanConfig::default();

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
            |_event| {},
        );
        let thermal_pause = guard.pause_flag();
        self.thermal_guard = Some(guard);

        std::thread::spawn(move || {
            ferrite_textcarver::run_scan(device, config, tx, cancel, thermal_pause, bytes_read);
        });
    }

    /// Cancel a running scan.
    pub(crate) fn cancel_scan(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.status = ScanStatus::Cancelled;
    }

    /// Export current blocks to files in `output_dir`.
    pub(crate) fn export_files(&mut self) {
        if self.blocks.is_empty() {
            self.export_status = Some("Nothing to export — no blocks found yet.".to_string());
            return;
        }
        let dir = if self.output_dir.is_empty() {
            "./ferrite_text".to_string()
        } else {
            self.output_dir.clone()
        };
        let (written, errors) = ferrite_textcarver::write_files(&dir, &self.blocks);
        if errors.is_empty() {
            self.export_status = Some(format!("Exported {written} blocks to {dir}"));
        } else {
            self.export_status = Some(format!(
                "Exported {written} blocks; {} error(s): {}",
                errors.len(),
                errors.first().unwrap_or(&String::new())
            ));
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_state_is_idle() {
        let s = TextScanState::new();
        assert_eq!(s.status, ScanStatus::Idle);
        assert!(s.blocks.is_empty());
        assert!(!s.consent_given);
    }

    #[test]
    fn is_editing_false_by_default() {
        let s = TextScanState::new();
        assert!(!s.is_editing());
    }

    #[test]
    fn is_editing_true_when_consent_shown() {
        let mut s = TextScanState::new();
        s.show_consent = true;
        assert!(s.is_editing());
    }

    #[test]
    fn is_editing_true_when_editing_dir() {
        let mut s = TextScanState::new();
        s.editing_dir = true;
        assert!(s.is_editing());
    }

    #[test]
    fn rebuild_filtered_all() {
        let mut s = TextScanState::new();
        s.blocks.push(make_block(TextKind::Json));
        s.blocks.push(make_block(TextKind::Sql));
        s.filter_kind = None;
        s.rebuild_filtered();
        assert_eq!(s.filtered.len(), 2);
    }

    #[test]
    fn rebuild_filtered_by_kind() {
        let mut s = TextScanState::new();
        s.blocks.push(make_block(TextKind::Json));
        s.blocks.push(make_block(TextKind::Sql));
        s.filter_kind = Some(TextKind::Json);
        s.rebuild_filtered();
        assert_eq!(s.filtered.len(), 1);
        assert_eq!(s.blocks[s.filtered[0]].kind, TextKind::Json);
    }

    fn make_block(kind: TextKind) -> TextBlock {
        TextBlock {
            byte_offset: 0,
            length: 512,
            kind,
            extension: kind.extension(),
            confidence: 80,
            quality: 90,
            preview: "preview".to_string(),
            lang: None,
        }
    }
}

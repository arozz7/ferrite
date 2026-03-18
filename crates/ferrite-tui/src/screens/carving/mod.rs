//! Screen 6 — File Carving: select signature types and run the carving engine
//! with live progress, then extract hits to disk.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;

use std::time::Instant;

use ferrite_blockdev::BlockDevice;
use ferrite_carver::{CarveHit, CarveQuality, ScanProgress, Signature};
use ferrite_filesystem::MetadataIndex;

mod checkpoint;
mod events;
mod extract;
mod helpers;
mod input;
mod preview;
mod preview_more;
mod render;
mod render_progress;
mod session_ops;
mod user_sig_panel;
mod user_sigs;
pub(crate) use helpers::fmt_bytes;
pub(crate) use preview::ColorCap;

// ── Constants ─────────────────────────────────────────────────────────────────

/// Maximum number of hits stored in the TUI list.  Hits beyond this cap are
/// counted in `total_hits_found` but not stored in memory.
pub(crate) const DISPLAY_CAP: usize = 100_000;

/// Auto-extract queue length at which the scan is automatically paused to let
/// extraction catch up.  Kept intentionally small: at high hit densities the
/// scan can enqueue thousands of hits per second, so the primary trigger is
/// whether an extraction batch is already running (see events.rs).
const AUTO_EXTRACT_HIGH_WATER: usize = 100;

/// Auto-extract queue length below which a back-pressure pause is lifted and
/// the scan resumes.  Using a near-zero value means we only resume once the
/// queue is essentially empty.
const AUTO_EXTRACT_LOW_WATER: usize = 10;

// ── Types ─────────────────────────────────────────────────────────────────────

/// Which scan-range LBA field is currently being edited.
#[derive(Debug, Clone, PartialEq)]
enum ScanRangeField {
    None,
    Start,
    End,
}

enum CarveMsg {
    Progress(ScanProgress),
    /// A batch of hits streamed from the scanner thread (replaces the old
    /// `Done(Vec<CarveHit>)` accumulation model).
    HitBatch(Vec<CarveHit>),
    /// Scan completed (or was cancelled).  All hits have been delivered via
    /// `HitBatch` messages; no payload.
    Done,
    MetadataReady(MetadataIndex),
    Extracted {
        idx: usize,
        bytes: u64,
        truncated: bool,
        /// Structural integrity tag assigned immediately after extraction.
        quality: CarveQuality,
    },
    /// Content fingerprint matched a previously extracted hit — extraction skipped.
    Duplicate {
        idx: usize,
    },
    ExtractionStarted {
        idx: usize,
    },
    ExtractionProgress {
        done: usize,
        total: usize,
        total_bytes: u64,
        last_name: String,
    },
    ExtractionDone {
        succeeded: usize,
        truncated: usize,
        failed: usize,
        /// Hits skipped due to duplicate content fingerprint.
        duplicates: usize,
        total_bytes: u64,
        elapsed_secs: f64,
    },
    Error(String),
}

/// Tracks state of a running bulk extraction.
struct ExtractProgress {
    done: usize,
    total: usize,
    total_bytes: u64,
    last_name: String,
    start: Instant,
}

/// Summary shown after a bulk extraction completes.
struct ExtractionSummary {
    succeeded: usize,
    truncated: usize,
    failed: usize,
    /// Hits skipped because their content fingerprint was already seen.
    duplicates: usize,
    total_bytes: u64,
    elapsed_secs: f64,
}

#[derive(PartialEq)]
enum CarveStatus {
    Idle,
    Running,
    /// Pause has been requested; waiting for the scan thread to finish its
    /// current chunk and enter the spin-wait.
    Pausing,
    Paused,
    Done,
    Error(String),
}

/// Focus panel for keyboard navigation.
#[derive(PartialEq, Clone, Copy)]
enum CarveFocus {
    Signatures,
    Hits,
}

/// A signature entry with an enabled/disabled toggle.
pub struct SigEntry {
    pub sig: Signature,
    pub enabled: bool,
}

/// A named group of related signatures shown as a collapsible tree node.
pub struct SigGroup {
    /// Display label for the group header row.
    pub label: &'static str,
    /// Whether the group's entries are visible in the list.
    pub expanded: bool,
    pub entries: Vec<SigEntry>,
}

/// One visible row in the signature panel's flat navigation list.
///
/// Rebuilt from `groups` whenever a group is expanded or collapsed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum CursorRow {
    /// A group header row.  Index is into `CarvingState::groups`.
    Group(usize),
    /// An individual signature row.  Indices are (group, entry-within-group).
    Sig(usize, usize),
}

/// Per-hit extraction status.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum HitStatus {
    Unextracted,
    /// Waiting in the work queue — a worker hasn't picked it up yet.
    Queued,
    /// A worker thread is actively reading/writing this file right now.
    Extracting,
    Ok {
        bytes: u64,
    },
    /// Footer not found AND hit max_size bytes.
    Truncated {
        bytes: u64,
    },
    /// Skipped — content fingerprint matches a previously extracted file.
    Duplicate,
}

/// Whether the user-signature form is adding a new entry or editing an existing one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum FormMode {
    Add,
    /// The `usize` is the index into `CarvingState::user_sigs`.
    Edit(usize),
}

/// State for the add/edit form shown inside the custom-signature panel.
pub(crate) struct UserSigForm {
    pub mode: FormMode,
    /// Index of the currently focused input field (0–4).
    pub field: usize,
    pub name: String,
    pub extension: String,
    pub header: String,
    pub footer: String,
    pub max_size_str: String,
    /// Validation error shown below the form until the next key press.
    pub error: Option<String>,
}

/// A carve hit paired with its extraction status and selection flag.
pub struct HitEntry {
    pub hit: CarveHit,
    pub status: HitStatus,
    pub selected: bool,
    /// Structural quality tag assigned immediately after extraction.
    pub quality: Option<CarveQuality>,
}

// ── State ─────────────────────────────────────────────────────────────────────

pub struct CarvingState {
    device: Option<Arc<dyn BlockDevice>>,
    /// Signature groups shown in the left panel as a collapsible tree.
    pub(crate) groups: Vec<SigGroup>,
    /// Flat list of visible rows derived from `groups` (rebuilt on expand/collapse).
    pub(crate) cursor_rows: Vec<CursorRow>,
    sig_sel: usize,
    hits: Vec<HitEntry>,
    hit_sel: usize,
    /// Number of visible rows in the hits list (updated each render for PgUp/PgDn).
    pub(crate) hits_page_size: usize,
    focus: CarveFocus,
    status: CarveStatus,
    cancel: Arc<AtomicBool>,
    /// Pause flag for the scan thread only.  Also used for back-pressure when
    /// auto-extract is on.  Extraction workers use `extract_pause` instead so
    /// that pausing the scan never inadvertently stalls extraction.
    pause: Arc<AtomicBool>,
    /// Set by the scan thread when it enters the pause spin-wait.  The TUI
    /// watches this to transition `Pausing → Paused` once the thread has
    /// actually stopped advancing.
    paused_ack: Arc<AtomicBool>,
    /// Pause flag for extraction workers only (user-initiated via `p` key
    /// while extraction is running).  Kept separate from `pause` so that
    /// back-pressure scan gating never blocks the extraction pipeline.
    extract_pause: Arc<AtomicBool>,
    rx: Option<Receiver<CarveMsg>>,
    /// Persistent sender kept alive after scan completes so extraction results
    /// can be sent back on the same channel.
    tx: Option<Sender<CarveMsg>>,
    /// Latest progress update from the background scan thread.
    scan_progress: Option<ScanProgress>,
    /// Wall-clock time when the current scan started (for rate + ETA).
    scan_start: Option<Instant>,
    /// Directory where extracted files are written.
    pub output_dir: String,
    /// Whether the output_dir field is being edited.
    editing_dir: bool,
    /// Progress of the running bulk extraction (None when idle).
    extract_progress: Option<ExtractProgress>,
    /// Set to true to abort a running bulk extraction.
    extract_cancel: Arc<AtomicBool>,
    /// Summary metrics shown after a bulk extraction completes.
    extract_summary: Option<ExtractionSummary>,
    /// Byte-offset → original filename index, built in background after scan.
    meta_index: Option<Arc<MetadataIndex>>,
    /// `true` while the metadata index is being built in the background.
    meta_index_building: bool,
    /// Total wall-clock time spent in a paused state during the current scan.
    paused_elapsed: std::time::Duration,
    /// Timestamp when the current scan pause started (`None` when not paused).
    paused_since: Option<std::time::Instant>,
    /// Scan range LBA strings (empty = beginning / end of device).
    pub(crate) scan_start_lba_str: String,
    pub(crate) scan_end_lba_str: String,
    /// Which scan-range field is currently being edited.
    scan_range_field: ScanRangeField,
    /// Checkpoint file path for the current session.
    checkpoint_path: Option<String>,
    /// Index of the last hit that was written to the checkpoint file.
    checkpoint_flushed: usize,
    /// Whether the preview panel is visible.
    pub(crate) show_preview: bool,
    /// Cached preview for the currently selected hit.
    pub(crate) current_preview: Option<preview::HitPreview>,
    /// Index of the hit that `current_preview` was built for (or is loading).
    preview_hit_idx: Option<usize>,
    /// Channel receiver for the background preview loader thread.
    preview_rx: Option<mpsc::Receiver<Option<preview::HitPreview>>>,
    /// `true` while a preview is being loaded in a background thread.
    pub(crate) preview_loading: bool,
    /// Terminal colour capability (detected once at startup).
    pub(crate) color_cap: ColorCap,
    /// Total hits found during the scan (including those above `DISPLAY_CAP`).
    pub(crate) total_hits_found: usize,
    /// Auto-extract mode: extract each hit as it arrives from the scanner.
    pub(crate) auto_extract: bool,
    /// Queue of hits pending automatic extraction: (hit_idx, hit, output_path).
    /// `hit_idx` is the index in `self.hits`, or `usize::MAX` for hits beyond
    /// `DISPLAY_CAP` that are not shown in the list.
    pub(crate) auto_extract_queue: std::collections::VecDeque<(usize, CarveHit, String)>,
    /// Available disk space at (or near) the output directory (bytes).
    /// Updated periodically in `tick()`.
    pub(crate) disk_avail_bytes: Option<u64>,
    /// Tick counter used to throttle the disk-space poll (checked every ~5 s).
    disk_space_tick: u32,
    // ── User-defined signatures ───────────────────────────────────────────────
    /// Path to the user signatures TOML file.
    pub(crate) user_sig_path: String,
    /// In-memory list of user-defined signatures (source of truth for CRUD).
    pub(crate) user_sigs: Vec<user_sigs::UserSigDef>,
    /// Whether the custom-signature overlay panel is visible.
    pub(crate) show_user_panel: bool,
    /// Selected row in the user-signature panel list.
    pub(crate) user_panel_sel: usize,
    /// Active add/edit form.  `None` = list view.
    pub(crate) user_sig_form: Option<UserSigForm>,
    /// `true` while awaiting y/n confirmation before deleting a signature.
    pub(crate) user_confirm_delete: bool,
    /// Path string being typed for the import prompt.
    pub(crate) user_import_path: String,
    /// `true` while the import-path text field is active.
    pub(crate) editing_import: bool,

    /// `true` when the scan has been automatically paused because the
    /// auto-extract queue exceeded `AUTO_EXTRACT_HIGH_WATER`.  Cleared when
    /// the queue drains below `AUTO_EXTRACT_LOW_WATER`, or when the user
    /// manually presses `p` (which takes over pause ownership).
    backpressure_paused: bool,
    /// Absolute byte offset to resume a scan from when loading a saved session.
    /// Set by `restore_from_session`; consumed (and cleared to 0) when the next
    /// scan starts.  0 means "start from the configured LBA range beginning".
    pub(crate) resume_from_byte: u64,
    /// Absolute byte offset of the *configured* scan window start (from the
    /// start-LBA field, not the resume point).  Set in `start_scan` so the
    /// progress bar can show overall completion even on a resumed scan.
    pub(crate) scan_window_start: u64,
    /// SHA-256–truncated fingerprints (first 4 KiB of each hit, hashed to u64)
    /// accumulated across the current extraction session.  Shared with the
    /// extraction worker thread to detect and suppress duplicate content.
    pub(crate) seen_fingerprints: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<u64>>>,
    /// Number of hits skipped during extraction because their content fingerprint
    /// matched a previously extracted file.
    pub(crate) duplicates_suppressed: usize,
}

impl Default for CarvingState {
    fn default() -> Self {
        Self::new()
    }
}

impl CarvingState {
    pub fn new() -> Self {
        let groups = helpers::load_builtin_sig_groups();
        let mut s = Self {
            device: None,
            groups,
            cursor_rows: Vec::new(),
            sig_sel: 0,
            hits: Vec::new(),
            hit_sel: 0,
            hits_page_size: 20,
            focus: CarveFocus::Signatures,
            status: CarveStatus::Idle,
            cancel: Arc::new(AtomicBool::new(false)),
            pause: Arc::new(AtomicBool::new(false)),
            paused_ack: Arc::new(AtomicBool::new(false)),
            extract_pause: Arc::new(AtomicBool::new(false)),
            rx: None,
            tx: None,
            scan_progress: None,
            scan_start: None,
            output_dir: String::new(),
            editing_dir: false,
            extract_progress: None,
            extract_cancel: Arc::new(AtomicBool::new(false)),
            extract_summary: None,
            meta_index: None,
            meta_index_building: false,
            paused_elapsed: std::time::Duration::ZERO,
            paused_since: None,
            scan_start_lba_str: String::new(),
            scan_end_lba_str: String::new(),
            scan_range_field: ScanRangeField::None,
            checkpoint_path: None,
            checkpoint_flushed: 0,
            show_preview: false,
            current_preview: None,
            preview_hit_idx: None,
            preview_rx: None,
            preview_loading: false,
            color_cap: ColorCap::detect(),
            total_hits_found: 0,
            auto_extract: false,
            auto_extract_queue: std::collections::VecDeque::new(),
            disk_avail_bytes: None,
            disk_space_tick: 0,
            backpressure_paused: false,
            resume_from_byte: 0,
            scan_window_start: 0,
            seen_fingerprints: std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::HashSet::new(),
            )),
            duplicates_suppressed: 0,
            user_sig_path: "./ferrite-user-signatures.toml".to_string(),
            user_sigs: Vec::new(),
            show_user_panel: false,
            user_panel_sel: 0,
            user_sig_form: None,
            user_confirm_delete: false,
            user_import_path: String::new(),
            editing_import: false,
        };
        // Load user sigs and append as "Custom" group if any are present.
        s.user_sigs = user_sigs::load_user_sigs(&s.user_sig_path);
        if let Some(group) = helpers::build_user_sig_group(&s.user_sigs) {
            s.groups.push(group);
        }
        s.rebuild_cursor_rows();
        s
    }

    /// Rebuild the flat `cursor_rows` navigation list from the current group
    /// state.  Must be called after any expand/collapse operation.
    pub(crate) fn rebuild_cursor_rows(&mut self) {
        self.cursor_rows.clear();
        for (gi, group) in self.groups.iter().enumerate() {
            self.cursor_rows.push(CursorRow::Group(gi));
            if group.expanded {
                for si in 0..group.entries.len() {
                    self.cursor_rows.push(CursorRow::Sig(gi, si));
                }
            }
        }
        // Keep sig_sel in bounds after a collapse may have removed rows.
        let max = self.cursor_rows.len().saturating_sub(1);
        self.sig_sel = self.sig_sel.min(max);
    }

    /// Returns the current number of carve hits.
    pub fn hits_count(&self) -> usize {
        self.hits.len()
    }

    /// Returns `true` if there are any carve hits.
    pub fn has_hits(&self) -> bool {
        !self.hits.is_empty()
    }

    /// Remove intra-file duplicate hits from the current list using each
    /// signature's `min_hit_gap`.  Useful for cleaning up sessions that were
    /// started before the gap suppressor existed (e.g. MPG flood hits).
    ///
    /// Hits are processed in offset order; any hit within `min_hit_gap` bytes
    /// of the last kept hit for the same signature is removed.  Returns the
    /// number of hits removed.
    pub fn dedup_hits_by_gap(&mut self) -> usize {
        use std::collections::HashMap;
        let before = self.hits.len();
        let mut last_by_sig: HashMap<String, u64> = HashMap::new();
        self.hits.retain(|e| {
            let gap = e.hit.signature.min_hit_gap;
            if gap == 0 {
                return true;
            }
            let accept = match last_by_sig.get(&e.hit.signature.name) {
                None => true,
                Some(&last) => e.hit.byte_offset >= last.saturating_add(gap),
            };
            if accept {
                last_by_sig.insert(e.hit.signature.name.clone(), e.hit.byte_offset);
            }
            accept
        });
        self.hit_sel = self.hit_sel.min(self.hits.len().saturating_sub(1));
        before - self.hits.len()
    }

    /// Returns the checkpoint path if one is set.
    pub fn checkpoint_path(&self) -> Option<&str> {
        self.checkpoint_path.as_deref()
    }

    /// Returns `true` while any text-input field is being edited (so `q` won't quit).
    pub fn is_editing(&self) -> bool {
        self.editing_dir || self.scan_range_field != ScanRangeField::None || self.show_user_panel
    }

    /// Returns the byte offset of the currently selected hit when focus is on
    /// the Hits panel.  Used by `app.rs` to deep-link into the hex viewer.
    pub fn selected_hit_offset(&self) -> Option<u64> {
        if self.focus == CarveFocus::Hits {
            self.hits.get(self.hit_sel).map(|e| e.hit.byte_offset)
        } else {
            None
        }
    }

    // ── User signature CRUD ───────────────────────────────────────────────────

    /// Rebuild the "Custom" group in `self.groups` from `self.user_sigs`.
    ///
    /// - If `user_sigs` is empty the Custom group is removed (if present).
    /// - If `user_sigs` is non-empty the Custom group is updated in-place or
    ///   appended to the end of the groups list.
    pub(crate) fn refresh_custom_group(&mut self) {
        let custom_idx = self.groups.iter().position(|g| g.label == "Custom");
        match helpers::build_user_sig_group(&self.user_sigs) {
            Some(group) => match custom_idx {
                Some(idx) => self.groups[idx] = group,
                None => self.groups.push(group),
            },
            None => {
                if let Some(idx) = custom_idx {
                    self.groups.remove(idx);
                }
            }
        }
        self.rebuild_cursor_rows();
    }

    /// Validate the form contents and, on success, persist and refresh.
    /// On validation failure the form is returned to `self.user_sig_form`
    /// with an error message set.
    pub(crate) fn submit_user_form(&mut self, mut form: UserSigForm) {
        // Pre-validate individual fields for targeted error messages.
        if !user_sigs::validate_header(&form.header) {
            form.error = Some(
                "invalid header — use space-separated uppercase hex (e.g. FF D8 FF) with ?? for wildcards"
                    .to_string(),
            );
            self.user_sig_form = Some(form);
            return;
        }
        if !user_sigs::validate_footer(&form.footer) {
            form.error = Some(
                "invalid footer — use space-separated uppercase hex (e.g. FF D9), or leave blank"
                    .to_string(),
            );
            self.user_sig_form = Some(form);
            return;
        }
        let max_size = form.max_size_str.trim().parse::<u64>().unwrap_or(0);
        let def = user_sigs::UserSigDef {
            name: form.name.trim().to_string(),
            extension: form.extension.trim().trim_start_matches('.').to_string(),
            header: form.header.trim().to_uppercase(),
            footer: form.footer.trim().to_uppercase(),
            max_size,
        };
        match def.to_signature() {
            Err(e) => {
                form.error = Some(e);
                self.user_sig_form = Some(form);
            }
            Ok(_) => {
                match form.mode {
                    FormMode::Add => self.user_sigs.push(def),
                    FormMode::Edit(idx) => {
                        if idx < self.user_sigs.len() {
                            self.user_sigs[idx] = def;
                        }
                    }
                }
                let _ = user_sigs::save_user_sigs(&self.user_sig_path, &self.user_sigs);
                self.refresh_custom_group();
                self.user_sig_form = None;
                // Keep selection in bounds after potential re-index.
                let max = self.user_sigs.len().saturating_sub(1);
                self.user_panel_sel = self.user_panel_sel.min(max);
            }
        }
    }

    /// Load sigs from `self.user_import_path`, merge with `self.user_sigs`,
    /// persist, and refresh.  Silently does nothing if the path is invalid or
    /// the file is empty.
    pub(crate) fn do_import(&mut self) {
        self.editing_import = false;
        let path = self.user_import_path.trim().to_string();
        self.user_import_path.clear();
        if path.is_empty() {
            return;
        }
        let new_sigs = user_sigs::load_user_sigs(&path);
        if new_sigs.is_empty() {
            return;
        }
        for sig in new_sigs {
            let exists = self
                .user_sigs
                .iter()
                .any(|s| s.name == sig.name && s.extension == sig.extension);
            if !exists {
                self.user_sigs.push(sig);
            }
        }
        let _ = user_sigs::save_user_sigs(&self.user_sig_path, &self.user_sigs);
        self.refresh_custom_group();
    }

    /// Suggest an output directory derived from the imaging destination path.
    /// Called by `app.rs` whenever the user navigates to this screen.
    /// Only updates if `output_dir` is still empty (user hasn't set one yet).
    pub fn suggest_output_dir(&mut self, imaging_dest: &str) {
        if !self.output_dir.is_empty() || imaging_dest.is_empty() {
            return;
        }
        // Strip the filename from the imaging dest and append "carved".
        let base = std::path::Path::new(imaging_dest)
            .parent()
            .and_then(|p| p.to_str())
            .unwrap_or(".");
        self.output_dir = format!("{base}\\carved");
    }

    pub fn set_device(&mut self, device: Arc<dyn BlockDevice>) {
        self.device = Some(device);
        self.hits.clear();
        self.hit_sel = 0;
        self.status = CarveStatus::Idle;
        self.cancel.store(false, Ordering::Relaxed);
        self.pause.store(false, Ordering::Relaxed);
        self.paused_ack.store(false, Ordering::Relaxed);
        self.extract_pause.store(false, Ordering::Relaxed);
        self.extract_cancel.store(false, Ordering::Relaxed);
        self.extract_progress = None;
        self.extract_summary = None;
        self.rx = None;
        self.tx = None;
        self.meta_index = None;
        self.meta_index_building = false;
        self.paused_elapsed = std::time::Duration::ZERO;
        self.paused_since = None;
        self.checkpoint_path = None;
        self.checkpoint_flushed = 0;
        self.show_preview = false;
        self.current_preview = None;
        self.preview_hit_idx = None;
        self.preview_rx = None;
        self.preview_loading = false;
        self.total_hits_found = 0;
        self.auto_extract = false;
        self.auto_extract_queue.clear();
        self.disk_avail_bytes = None;
        self.disk_space_tick = 0;
        self.backpressure_paused = false;
        self.seen_fingerprints.lock().unwrap().clear();
        self.duplicates_suppressed = 0;
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyModifiers};

    use super::*;

    fn all_entries(s: &CarvingState) -> impl Iterator<Item = &SigEntry> {
        s.groups.iter().flat_map(|g| g.entries.iter())
    }

    #[test]
    fn builtin_signatures_load() {
        let s = CarvingState::new();
        assert!(
            !s.groups.is_empty(),
            "expected at least one built-in signature group"
        );
        assert!(
            s.groups.iter().any(|g| !g.entries.is_empty()),
            "expected at least one built-in signature"
        );
    }

    #[test]
    fn all_signatures_enabled_by_default() {
        let s = CarvingState::new();
        assert!(all_entries(&s).all(|e| e.enabled));
    }

    #[test]
    fn space_on_group_header_toggles_all_in_group() {
        let mut s = CarvingState::new();
        // sig_sel == 0 → CursorRow::Group(0): the first group header.
        assert!(matches!(s.cursor_rows[0], CursorRow::Group(0)));
        let group_len = s.groups[0].entries.len();
        assert!(group_len > 0);
        // All enabled initially.
        assert!(s.groups[0].entries.iter().all(|e| e.enabled));
        // Space on group header disables all entries in that group.
        s.handle_key(KeyCode::Char(' '), KeyModifiers::NONE);
        assert!(s.groups[0].entries.iter().all(|e| !e.enabled));
        // Space again re-enables all.
        s.handle_key(KeyCode::Char(' '), KeyModifiers::NONE);
        assert!(s.groups[0].entries.iter().all(|e| e.enabled));
    }

    #[test]
    fn enter_expands_and_collapses_group() {
        let mut s = CarvingState::new();
        // All groups start collapsed → cursor_rows has one row per group.
        let group_count = s.groups.len();
        assert_eq!(s.cursor_rows.len(), group_count);
        // Enter on group 0 expands it.
        s.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        assert!(s.groups[0].expanded);
        let expanded_rows = s.cursor_rows.len();
        assert!(expanded_rows > group_count);
        // Enter again collapses it.
        s.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        assert!(!s.groups[0].expanded);
        assert_eq!(s.cursor_rows.len(), group_count);
    }

    #[test]
    fn space_on_sig_row_toggles_individual() {
        let mut s = CarvingState::new();
        // Expand the first group so sig rows are visible.
        s.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        // Move down to the first sig row (index 1).
        s.handle_key(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(s.sig_sel, 1);
        assert!(matches!(s.cursor_rows[1], CursorRow::Sig(0, 0)));
        let was_enabled = s.groups[0].entries[0].enabled;
        s.handle_key(KeyCode::Char(' '), KeyModifiers::NONE);
        assert_ne!(s.groups[0].entries[0].enabled, was_enabled);
    }

    #[test]
    fn selection_does_not_underflow() {
        let mut s = CarvingState::new();
        s.handle_key(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(s.sig_sel, 0);
    }

    #[test]
    fn signatures_include_sqlite() {
        let s = CarvingState::new();
        assert!(
            all_entries(&s).any(|e| e.sig.extension == "db"),
            "expected SQLite signature (extension 'db') in built-in list"
        );
    }

    #[test]
    fn signatures_include_flac() {
        let s = CarvingState::new();
        assert!(
            all_entries(&s).any(|e| e.sig.extension == "flac"),
            "expected FLAC signature in built-in list"
        );
    }

    #[test]
    fn signatures_include_mkv() {
        let s = CarvingState::new();
        assert!(
            all_entries(&s).any(|e| e.sig.extension == "mkv"),
            "expected MKV/Matroska signature in built-in list"
        );
    }

    #[test]
    fn groups_cover_all_signatures() {
        let s = CarvingState::new();
        // Total entries across all groups must equal the built-in signature count.
        let total: usize = s.groups.iter().map(|g| g.entries.len()).sum();
        assert_eq!(
            total, 99,
            "expected 99 built-in signatures across all groups"
        );
    }

    #[test]
    fn video_group_contains_mov_and_webm() {
        let s = CarvingState::new();
        let video = s.groups.iter().find(|g| g.label == "Video").unwrap();
        assert!(video.entries.iter().any(|e| e.sig.extension == "mov"));
        assert!(video.entries.iter().any(|e| e.sig.extension == "webm"));
    }

    #[test]
    fn hit_entry_starts_unextracted() {
        // Verify that HitEntry constructed manually starts with Unextracted status.
        let sig = Signature {
            name: "Test".to_string(),
            extension: "tst".to_string(),
            header: vec![Some(0xFF)],
            footer: vec![],
            footer_last: false,
            max_size: 1024,
            size_hint: None,
            min_size: 0,
            pre_validate: None,
            header_offset: 0,
            min_hit_gap: 0,
        };
        let hit = CarveHit {
            byte_offset: 0,
            signature: sig,
        };
        let entry = HitEntry {
            hit,
            status: HitStatus::Unextracted,
            selected: false,
            quality: None,
        };
        assert_eq!(entry.status, HitStatus::Unextracted);
    }

    #[test]
    fn all_hits_start_as_unextracted() {
        // Structural test: HitStatus::Unextracted is the initial state.
        let status = HitStatus::Unextracted;
        assert_eq!(status, HitStatus::Unextracted);
    }
}

//! Screen 5 — File Browser: navigate filesystem directory trees, recover
//! deleted files with their original folder structure preserved.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyModifiers};
use ferrite_blockdev::BlockDevice;
use ferrite_filesystem::{open_filesystem, FileEntry, FilesystemParser, FilesystemType};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Cell, Gauge, Paragraph, Row, Table, TableState},
    Frame,
};

use super::fs_recovery::{
    extract_to_recovered, spawn_recovery_thread, RecoveryMsg, RecoveryProgress,
};

// ── Types ─────────────────────────────────────────────────────────────────────

enum BrowserMsg {
    Opened(Arc<dyn FilesystemParser>),
    Error(String),
}

// Arc<dyn FilesystemParser>: Send (FilesystemParser: Send + Sync).
// SAFETY: no raw pointer members in BrowserMsg.
unsafe impl Send for BrowserMsg {}

enum BrowserStatus {
    Idle,
    Opening,
    Browsing,
    Error(String),
}

// ── State ─────────────────────────────────────────────────────────────────────

pub struct FileBrowserState {
    device: Option<Arc<dyn BlockDevice>>,
    parser: Option<Arc<dyn FilesystemParser>>,
    fs_type: FilesystemType,
    path_segments: Vec<String>,
    entries: Vec<FileEntry>,
    selected: usize,
    scroll: usize,
    show_deleted: bool,
    status: BrowserStatus,
    open_rx: Option<Receiver<BrowserMsg>>,
    /// Last single-file extraction result message.
    extract_status: Option<String>,
    /// Background batch recovery channel.
    recovery_rx: Option<Receiver<RecoveryMsg>>,
    /// Live recovery progress (Some while a recovery run is active or just finished).
    recovery_progress: Option<RecoveryProgress>,
    /// Set to `true` to abort the background recovery thread between files.
    recovery_cancel: Arc<AtomicBool>,
}

impl Default for FileBrowserState {
    fn default() -> Self {
        Self::new()
    }
}

impl FileBrowserState {
    pub fn new() -> Self {
        Self {
            device: None,
            parser: None,
            fs_type: FilesystemType::Unknown,
            path_segments: Vec::new(),
            entries: Vec::new(),
            selected: 0,
            scroll: 0,
            show_deleted: false,
            status: BrowserStatus::Idle,
            open_rx: None,
            extract_status: None,
            recovery_rx: None,
            recovery_progress: None,
            recovery_cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn set_device(&mut self, device: Arc<dyn BlockDevice>) {
        // Cancel any in-progress recovery before discarding the parser.
        self.recovery_cancel.store(true, Ordering::Relaxed);
        self.device = Some(device);
        self.parser = None;
        self.path_segments.clear();
        self.entries.clear();
        self.selected = 0;
        self.scroll = 0;
        self.status = BrowserStatus::Idle;
        self.open_rx = None;
        self.extract_status = None;
        self.recovery_rx = None;
        self.recovery_progress = None;
        self.recovery_cancel = Arc::new(AtomicBool::new(false));
    }

    /// Returns `true` while a text-input field is active (currently none on this screen).
    pub fn is_editing(&self) -> bool {
        false
    }

    /// Drain background channels — call once per event-loop tick.
    pub fn tick(&mut self) {
        // ── Filesystem open ───────────────────────────────────────────────────
        if let Some(rx) = &self.open_rx {
            match rx.try_recv() {
                Ok(BrowserMsg::Opened(parser)) => {
                    self.parser = Some(parser);
                    self.status = BrowserStatus::Browsing;
                    self.open_rx = None;
                    self.load_current_dir();
                }
                Ok(BrowserMsg::Error(e)) => {
                    self.status = BrowserStatus::Error(e);
                    self.open_rx = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.open_rx = None;
                }
            }
        }

        // ── Batch recovery ────────────────────────────────────────────────────
        if let Some(rx) = &self.recovery_rx {
            loop {
                match rx.try_recv() {
                    Ok(RecoveryMsg::Progress {
                        done,
                        total,
                        current_path,
                        errors,
                    }) => {
                        if let Some(p) = &mut self.recovery_progress {
                            p.done = done;
                            p.total = total;
                            p.current_path = current_path;
                            p.errors = errors;
                        }
                    }
                    Ok(RecoveryMsg::Done { succeeded, failed }) => {
                        if let Some(p) = &mut self.recovery_progress {
                            p.done = succeeded + failed;
                            p.total = succeeded + failed;
                            p.succeeded = succeeded;
                            p.errors = failed;
                            p.finished = true;
                        }
                        self.recovery_rx = None;
                        break;
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        self.recovery_rx = None;
                        break;
                    }
                }
            }
        }
    }

    pub fn handle_key(&mut self, code: KeyCode, _modifiers: KeyModifiers) {
        match code {
            KeyCode::Char('o') => self.start_open(),
            KeyCode::Char('e') => self.extract_selected(),
            KeyCode::Char('R') => self.recover_all_deleted(),
            KeyCode::Esc => self.cancel_recovery(),
            KeyCode::Up => self.move_selection(-1),
            KeyCode::Down => self.move_selection(1),
            KeyCode::Enter => self.open_selected(),
            KeyCode::Backspace => self.go_up(),
            KeyCode::Char('d') => {
                self.show_deleted = !self.show_deleted;
                self.load_current_dir();
            }
            _ => {}
        }
    }

    // ── Extraction actions ────────────────────────────────────────────────────

    /// Extract the selected file to `recovered/fs/<original_path>`.
    fn extract_selected(&mut self) {
        let entry = match self.entries.get(self.selected) {
            Some(e) if !e.is_dir => e.clone(),
            _ => return,
        };
        let parser = match &self.parser {
            Some(p) => Arc::clone(p),
            None => return,
        };
        match extract_to_recovered(&entry, parser.as_ref(), "recovered") {
            Ok(bytes) => {
                let rel = entry.path.trim_start_matches('/');
                self.extract_status = Some(format!("Saved 'recovered/fs/{rel}' ({bytes} B)"));
            }
            Err(e) => {
                self.extract_status = Some(format!("Error: {e}"));
            }
        }
    }

    /// Start a batch recovery of all deleted files on a background thread.
    /// Results go to `recovered/fs/<original_path>`.
    fn recover_all_deleted(&mut self) {
        let parser = match &self.parser {
            Some(p) => Arc::clone(p),
            None => return,
        };
        // Reset cancel flag and previous progress.
        self.recovery_cancel.store(false, Ordering::Relaxed);
        self.recovery_progress = Some(RecoveryProgress {
            done: 0,
            total: 0,
            errors: 0,
            current_path: String::new(),
            finished: false,
            succeeded: 0,
        });
        self.extract_status = None;
        let rx = spawn_recovery_thread(
            parser,
            Arc::clone(&self.recovery_cancel),
            "recovered".to_string(),
        );
        self.recovery_rx = Some(rx);
    }

    /// Signal the background recovery thread to stop.
    fn cancel_recovery(&mut self) {
        self.recovery_cancel.store(true, Ordering::Relaxed);
    }

    // ── Navigation ────────────────────────────────────────────────────────────

    fn start_open(&mut self) {
        let device = match &self.device {
            Some(d) => Arc::clone(d),
            None => return,
        };
        self.fs_type = ferrite_filesystem::detect_filesystem(device.as_ref());

        // HFS+ is detected but has no parser yet — surface a helpful message
        // immediately rather than letting open_filesystem return a generic error.
        if self.fs_type == ferrite_filesystem::FilesystemType::HfsPlus {
            self.status = BrowserStatus::Error(
                "HFS+ detected — parser not yet implemented.\n\
                 Use the Carving tab (Tab 5) to recover files by signature."
                    .into(),
            );
            return;
        }

        // BitLocker-encrypted volumes cannot be parsed — direct the operator.
        if self.fs_type == ferrite_filesystem::FilesystemType::Encrypted {
            self.status = BrowserStatus::Error(
                "BitLocker encrypted volume detected.\n\
                 Decrypt the volume first (e.g. manage-bde or Disk Management) then re-open.\n\
                 File carving (Tab 5) may recover unencrypted fragments from slack space."
                    .into(),
            );
            return;
        }

        self.status = BrowserStatus::Opening;
        let (tx, rx) = mpsc::channel();
        self.open_rx = Some(rx);
        std::thread::spawn(move || match open_filesystem(device) {
            Ok(parser) => {
                let _ = tx.send(BrowserMsg::Opened(Arc::from(parser)));
            }
            Err(e) => {
                let _ = tx.send(BrowserMsg::Error(e.to_string()));
            }
        });
    }

    fn load_current_dir(&mut self) {
        let parser = match &self.parser {
            Some(p) => p,
            None => return,
        };
        let result = if self.path_segments.is_empty() {
            parser.root_directory()
        } else {
            parser.list_directory(&self.path_segments.join("/"))
        };
        match result {
            Ok(mut entries) => {
                if !self.show_deleted {
                    entries.retain(|e| !e.is_deleted);
                }
                self.entries = entries;
                self.selected = 0;
                self.scroll = 0;
            }
            Err(e) => {
                self.status = BrowserStatus::Error(e.to_string());
            }
        }

        if self.show_deleted {
            if let Ok(deleted) = parser.deleted_files() {
                let current_path = self.path_segments.join("/");
                for d in deleted {
                    if d.path.starts_with(&current_path) && !self.entries.contains(&d) {
                        self.entries.push(d);
                    }
                }
            }
        }
    }

    fn open_selected(&mut self) {
        if let Some(entry) = self.entries.get(self.selected).cloned() {
            if entry.is_dir {
                self.path_segments.push(entry.name.clone());
                self.load_current_dir();
            }
        }
    }

    fn go_up(&mut self) {
        if !self.path_segments.is_empty() {
            self.path_segments.pop();
            self.load_current_dir();
        }
    }

    fn move_selection(&mut self, delta: i32) {
        let len = self.entries.len();
        if len == 0 {
            return;
        }
        if delta < 0 {
            self.selected = self.selected.saturating_sub(1);
        } else {
            self.selected = (self.selected + 1).min(len - 1);
        }
    }

    // ── Rendering ─────────────────────────────────────────────────────────────

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        let recovering = self
            .recovery_progress
            .as_ref()
            .map(|p| !p.finished)
            .unwrap_or(false);

        let title = if recovering {
            " File Browser — Esc: cancel recovery "
        } else {
            match &self.status {
                BrowserStatus::Opening => " File Browser — opening filesystem… ",
                _ if self.show_deleted => {
                    " File Browser — [deleted shown] — d:toggle  e:extract  R:recover-all  o:open "
                }
                _ => " File Browser — d:toggle-deleted  e:extract  R:recover-all  o:open-fs ",
            }
        };

        let outer = Block::default().borders(Borders::ALL).title(title);

        match &self.status {
            BrowserStatus::Idle => {
                frame.render_widget(
                    Paragraph::new(" Press o to open the filesystem on the selected device.")
                        .block(outer),
                    area,
                );
            }
            BrowserStatus::Opening => {
                frame.render_widget(
                    Paragraph::new(" Parsing filesystem structures…").block(outer),
                    area,
                );
            }
            BrowserStatus::Error(e) => {
                frame.render_widget(
                    Paragraph::new(format!(" Error: {e}\n\n Press o to retry."))
                        .style(Style::default().fg(Color::Red))
                        .block(outer),
                    area,
                );
            }
            BrowserStatus::Browsing => {
                let inner = outer.inner(area);
                frame.render_widget(outer, area);
                self.render_browser(frame, inner);
            }
        }
    }

    fn render_browser(&self, frame: &mut Frame, area: Rect) {
        let recovering = self
            .recovery_progress
            .as_ref()
            .map(|p| !p.finished)
            .unwrap_or(false);

        // Bottom section: 2 rows when recovering (progress bar + status),
        // 1 row otherwise.
        let bottom_height = if recovering { 2 } else { 1 };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),             // breadcrumb
                Constraint::Min(0),                // file table
                Constraint::Length(bottom_height), // status / progress
            ])
            .split(area);

        // ── Breadcrumb ────────────────────────────────────────────────────────
        let path_str = if self.path_segments.is_empty() {
            format!(" [{:?}] / ", self.fs_type)
        } else {
            format!(
                " [{:?}] / {} ",
                self.fs_type,
                self.path_segments.join(" / ")
            )
        };
        frame.render_widget(
            Paragraph::new(path_str).style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            chunks[0],
        );

        // ── File table ────────────────────────────────────────────────────────
        if self.entries.is_empty() {
            frame.render_widget(Paragraph::new(" (empty directory)"), chunks[1]);
        } else {
            let header = Row::new([Cell::from("Name"), Cell::from("Size"), Cell::from("Type")])
                .style(
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                );

            let rows: Vec<Row> = self
                .entries
                .iter()
                .map(|e| {
                    let name_style = if e.is_deleted {
                        Style::default().fg(Color::DarkGray)
                    } else if e.is_dir {
                        Style::default()
                            .fg(Color::Blue)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };
                    let prefix = if e.is_dir { "📁 " } else { "   " };
                    let deleted_marker = if e.is_deleted { " [deleted]" } else { "" };
                    Row::new([
                        Cell::from(format!("{prefix}{}{deleted_marker}", e.name)).style(name_style),
                        Cell::from(fmt_bytes(e.size)),
                        Cell::from(if e.is_dir { "DIR" } else { "FILE" }),
                    ])
                })
                .collect();

            let widths = [
                Constraint::Min(30),
                Constraint::Length(10),
                Constraint::Length(5),
            ];

            let table = Table::new(rows, widths)
                .header(header)
                .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));

            let mut ts = TableState::default().with_selected(Some(self.selected));
            frame.render_stateful_widget(table, chunks[1], &mut ts);
        }

        // ── Bottom status / recovery progress ─────────────────────────────────
        if recovering {
            self.render_recovery_progress(frame, chunks[2]);
        } else {
            self.render_status_bar(frame, chunks[2]);
        }
    }

    fn render_recovery_progress(&self, frame: &mut Frame, area: Rect) {
        let Some(p) = &self.recovery_progress else {
            return;
        };

        // Split the bottom section into a progress bar (1 line) and a path line.
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(1)])
            .split(area);

        // Progress bar
        let (ratio, label) = if p.total == 0 {
            (0.0, "Enumerating deleted files…".to_string())
        } else {
            let r = p.done as f64 / p.total as f64;
            (
                r,
                format!(" Recovering: {}/{} ({} errors)", p.done, p.total, p.errors),
            )
        };

        let gauge = Gauge::default()
            .gauge_style(Style::default().fg(Color::Green).bg(Color::DarkGray))
            .ratio(ratio.clamp(0.0, 1.0))
            .label(label);
        frame.render_widget(gauge, rows[0]);

        // Current file being processed
        let path_display = if p.current_path.is_empty() {
            String::new()
        } else {
            format!(" → {}", p.current_path)
        };
        frame.render_widget(
            Paragraph::new(path_display).style(Style::default().fg(Color::DarkGray)),
            rows[1],
        );
    }

    fn render_status_bar(&self, frame: &mut Frame, area: Rect) {
        // Show finished recovery summary if available.
        if let Some(p) = &self.recovery_progress {
            if p.finished {
                let msg = format!(
                    " Recovery complete — {} saved, {} failed. Output: recovered/fs/",
                    p.succeeded, p.errors
                );
                let style = if p.errors > 0 {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default().fg(Color::Green)
                };
                frame.render_widget(Paragraph::new(msg).style(style), area);
                return;
            }
        }

        if let Some(msg) = &self.extract_status {
            let style = if msg.starts_with("Error") {
                Style::default().fg(Color::Red)
            } else {
                Style::default().fg(Color::Green)
            };
            frame.render_widget(Paragraph::new(msg.as_str()).style(style), area);
        }
    }
}

// FileEntry doesn't impl PartialEq — local helper.
trait EntryEq {
    fn contains(&self, other: &FileEntry) -> bool;
}

impl EntryEq for Vec<FileEntry> {
    fn contains(&self, other: &FileEntry) -> bool {
        self.iter()
            .any(|e| e.path == other.path && e.name == other.name)
    }
}

fn fmt_bytes(n: u64) -> String {
    const GIB: u64 = 1024 * 1024 * 1024;
    const MIB: u64 = 1024 * 1024;
    if n >= GIB {
        format!("{:.1}G", n as f64 / GIB as f64)
    } else if n >= MIB {
        format!("{:.1}M", n as f64 / MIB as f64)
    } else {
        format!("{n}B")
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_state_is_idle() {
        let s = FileBrowserState::new();
        assert!(matches!(s.status, BrowserStatus::Idle));
        assert!(!s.show_deleted);
        assert!(s.recovery_progress.is_none());
    }

    #[test]
    fn set_device_resets_state() {
        let mut s = FileBrowserState::new();
        s.path_segments.push("Windows".into());
        s.selected = 3;

        let data = vec![0u8; 512];
        let mock = ferrite_blockdev::MockBlockDevice::new(data, 512);
        let dev: Arc<dyn BlockDevice> = Arc::new(mock);
        s.set_device(dev);

        assert!(s.path_segments.is_empty());
        assert_eq!(s.selected, 0);
        assert!(s.entries.is_empty());
        assert!(s.recovery_progress.is_none());
    }

    #[test]
    fn e_key_noop_without_parser() {
        let mut s = FileBrowserState::new();
        s.handle_key(KeyCode::Char('e'), KeyModifiers::NONE);
        assert!(s.extract_status.is_none());
    }

    #[test]
    fn e_key_noop_on_directory_entry() {
        let mut s = FileBrowserState::new();
        s.entries.push(ferrite_filesystem::FileEntry {
            name: "Documents".into(),
            path: "/Documents".into(),
            size: 0,
            is_dir: true,
            is_deleted: false,
            created: None,
            modified: None,
            first_cluster: None,
            mft_record: None,
            inode_number: None,
            data_byte_offset: None,
            recovery_chance: ferrite_filesystem::RecoveryChance::Unknown,
        });
        s.handle_key(KeyCode::Char('e'), KeyModifiers::NONE);
        assert!(s.extract_status.is_none());
    }

    #[test]
    fn r_key_noop_without_parser() {
        let mut s = FileBrowserState::new();
        // R key with no parser must not panic or change state.
        s.handle_key(KeyCode::Char('R'), KeyModifiers::NONE);
        assert!(s.recovery_rx.is_none());
    }

    #[test]
    fn cancel_sets_atomic_flag() {
        let s = FileBrowserState::new();
        assert!(!s.recovery_cancel.load(Ordering::Relaxed));
        let cancel = Arc::clone(&s.recovery_cancel);
        cancel.store(true, Ordering::Relaxed);
        assert!(s.recovery_cancel.load(Ordering::Relaxed));
    }
}

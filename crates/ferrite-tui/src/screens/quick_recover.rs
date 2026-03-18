//! Screen 7 — Quick Recover: fast deleted-file recovery without a full imaging pass.
//!
//! Detects the filesystem on the selected device, enumerates deleted files,
//! scores each by recovery chance, and lets the user batch-recover them with
//! a single keypress.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyModifiers};
use ferrite_blockdev::BlockDevice;
use ferrite_filesystem::{
    detect_filesystem, open_filesystem, FileEntry, FilesystemParser, FilesystemType, RecoveryChance,
};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Gauge, Paragraph, Row, Table, TableState},
    Frame,
};

use super::fs_recovery::{spawn_recovery_thread, RecoveryMsg, RecoveryProgress};

// ── Internal message types ────────────────────────────────────────────────────

enum LoadMsg {
    Done(FilesystemType, Arc<dyn FilesystemParser>, Vec<FileEntry>),
    Error(String),
}

// Arc<dyn FilesystemParser> is Send (trait bound requires Send + Sync).
unsafe impl Send for LoadMsg {}

// ── Status types ──────────────────────────────────────────────────────────────

enum RecoverStatus {
    Idle,
    Loading,
    Recovering,
    Done,
}

// ── State ─────────────────────────────────────────────────────────────────────

pub struct QuickRecoverState {
    // device + parser state
    device: Option<Arc<dyn BlockDevice>>,
    parser: Option<Arc<dyn FilesystemParser>>,
    fs_type: FilesystemType,

    // file list (deleted files only, sorted by chance desc then name)
    entries: Vec<FileEntry>,
    selected: usize,
    scroll: usize,

    // multi-select
    checked: HashSet<usize>,

    // filter
    filter: String,
    filter_editing: bool,

    // load channel
    load_rx: Option<Receiver<LoadMsg>>,

    // extraction state
    status: RecoverStatus,
    recover_rx: Option<Receiver<RecoveryMsg>>,
    recover_progress: Option<RecoveryProgress>,
    recover_cancel: Arc<AtomicBool>,
    last_result: Option<String>,
}

impl Default for QuickRecoverState {
    fn default() -> Self {
        Self::new()
    }
}

impl QuickRecoverState {
    pub fn new() -> Self {
        Self {
            device: None,
            parser: None,
            fs_type: FilesystemType::Unknown,
            entries: Vec::new(),
            selected: 0,
            scroll: 0,
            checked: HashSet::new(),
            filter: String::new(),
            filter_editing: false,
            load_rx: None,
            status: RecoverStatus::Idle,
            recover_rx: None,
            recover_progress: None,
            recover_cancel: Arc::new(AtomicBool::new(false)),
            last_result: None,
        }
    }

    /// Attach a new device: reset state and spawn background load thread.
    pub fn set_device(&mut self, device: Arc<dyn BlockDevice>) {
        // Cancel any in-progress recovery before discarding parser.
        self.recover_cancel.store(true, Ordering::Relaxed);

        self.device = Some(Arc::clone(&device));
        self.parser = None;
        self.fs_type = FilesystemType::Unknown;
        self.entries.clear();
        self.selected = 0;
        self.scroll = 0;
        self.checked.clear();
        self.filter.clear();
        self.filter_editing = false;
        self.load_rx = None;
        self.status = RecoverStatus::Loading;
        self.recover_rx = None;
        self.recover_progress = None;
        self.recover_cancel = Arc::new(AtomicBool::new(false));
        self.last_result = None;

        let (tx, rx) = mpsc::channel();
        self.load_rx = Some(rx);

        std::thread::spawn(move || {
            let fs_type = detect_filesystem(device.as_ref());
            let parser: Arc<dyn FilesystemParser> = match open_filesystem(Arc::clone(&device)) {
                Ok(p) => Arc::from(p),
                Err(e) => {
                    let _ = tx.send(LoadMsg::Error(e.to_string()));
                    return;
                }
            };
            let deleted = match parser.deleted_files() {
                Ok(d) => d,
                Err(e) => {
                    let _ = tx.send(LoadMsg::Error(format!("deleted_files: {e}")));
                    return;
                }
            };
            let _ = tx.send(LoadMsg::Done(fs_type, parser, deleted));
        });
    }

    /// Returns `true` while the filter input field is active.
    pub fn is_editing(&self) -> bool {
        self.filter_editing
    }

    /// Drain background channels — call once per event-loop tick.
    pub fn tick(&mut self) {
        // ── Load channel ──────────────────────────────────────────────────────
        if let Some(rx) = &self.load_rx {
            match rx.try_recv() {
                Ok(LoadMsg::Done(fs_type, parser, mut deleted)) => {
                    self.fs_type = fs_type;
                    self.parser = Some(parser);
                    sort_entries(&mut deleted);
                    self.entries = deleted;
                    self.status = RecoverStatus::Idle;
                    self.load_rx = None;
                }
                Ok(LoadMsg::Error(e)) => {
                    tracing::warn!(error = %e, "quick_recover: filesystem load failed");
                    self.status = RecoverStatus::Idle;
                    self.load_rx = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.load_rx = None;
                    self.status = RecoverStatus::Idle;
                }
            }
        }

        // ── Recovery channel ──────────────────────────────────────────────────
        if let Some(rx) = &self.recover_rx {
            loop {
                match rx.try_recv() {
                    Ok(RecoveryMsg::Progress {
                        done,
                        total,
                        current_path,
                        errors,
                    }) => {
                        if let Some(p) = &mut self.recover_progress {
                            p.done = done;
                            p.total = total;
                            p.current_path = current_path;
                            p.errors = errors;
                        }
                    }
                    Ok(RecoveryMsg::Done { succeeded, failed }) => {
                        if let Some(p) = &mut self.recover_progress {
                            p.done = succeeded + failed;
                            p.total = succeeded + failed;
                            p.succeeded = succeeded;
                            p.errors = failed;
                            p.finished = true;
                        }
                        self.last_result = Some(format!(
                            "Recovered {succeeded} files successfully ({failed} failed)"
                        ));
                        self.status = RecoverStatus::Done;
                        self.recover_rx = None;
                        break;
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        self.recover_rx = None;
                        self.status = RecoverStatus::Done;
                        break;
                    }
                }
            }
        }
    }

    // ── Key handling ──────────────────────────────────────────────────────────

    pub fn handle_key(&mut self, code: KeyCode, _modifiers: KeyModifiers) {
        if self.filter_editing {
            match code {
                KeyCode::Esc => {
                    self.filter_editing = false;
                }
                KeyCode::Enter => {
                    self.filter_editing = false;
                    self.selected = 0;
                    self.scroll = 0;
                }
                KeyCode::Backspace => {
                    self.filter.pop();
                }
                KeyCode::Char(c) => {
                    self.filter.push(c);
                }
                _ => {}
            }
            return;
        }

        match code {
            KeyCode::Up => self.move_selection(-1),
            KeyCode::Down => self.move_selection(1),
            KeyCode::Char(' ') => self.toggle_check(),
            KeyCode::Char('a') => self.check_all_high(),
            KeyCode::Char('A') => self.check_all(),
            KeyCode::Esc => {
                self.checked.clear();
            }
            KeyCode::Char('R') => self.start_recovery(),
            KeyCode::Char('/') => {
                self.filter_editing = true;
            }
            _ => {}
        }
    }

    // ── Actions ───────────────────────────────────────────────────────────────

    fn move_selection(&mut self, delta: i32) {
        let visible = self.filtered_indices();
        if visible.is_empty() {
            return;
        }
        let len = visible.len();
        if delta < 0 {
            self.selected = self.selected.saturating_sub(1);
        } else {
            self.selected = (self.selected + 1).min(len - 1);
        }
    }

    fn toggle_check(&mut self) {
        let visible = self.filtered_indices();
        if let Some(&idx) = visible.get(self.selected) {
            if self.checked.contains(&idx) {
                self.checked.remove(&idx);
            } else {
                self.checked.insert(idx);
            }
        }
    }

    fn check_all_high(&mut self) {
        for (i, entry) in self.entries.iter().enumerate() {
            if entry.recovery_chance == RecoveryChance::High {
                self.checked.insert(i);
            }
        }
    }

    fn check_all(&mut self) {
        for i in 0..self.entries.len() {
            self.checked.insert(i);
        }
    }

    fn start_recovery(&mut self) {
        let parser = match &self.parser {
            Some(p) => Arc::clone(p),
            None => return,
        };

        // Collect entries to recover: checked, or all High-chance if none checked.
        let targets: Vec<FileEntry> = if self.checked.is_empty() {
            self.entries
                .iter()
                .filter(|e| e.recovery_chance == RecoveryChance::High)
                .cloned()
                .collect()
        } else {
            self.checked
                .iter()
                .filter_map(|&i| self.entries.get(i))
                .cloned()
                .collect()
        };

        if targets.is_empty() {
            return;
        }

        // Build a synthetic parser wrapper that returns `targets` as its deleted_files().
        // We spawn the standard recovery thread with a delegating wrapper.
        self.recover_cancel.store(false, Ordering::Relaxed);
        self.recover_progress = Some(RecoveryProgress {
            done: 0,
            total: targets.len(),
            errors: 0,
            current_path: String::new(),
            finished: false,
            succeeded: 0,
        });
        self.last_result = None;
        self.status = RecoverStatus::Recovering;

        let cancel = Arc::clone(&self.recover_cancel);
        let output_base = "ferrite_output/quick_recover".to_string();

        // Wrap targets + parser in a local struct so we can pass it to spawn_recovery_thread.
        let wrapper = Arc::new(TargetedParser {
            inner: parser,
            targets,
        });

        let rx = spawn_recovery_thread(wrapper, cancel, output_base);
        self.recover_rx = Some(rx);
    }

    // ── Filter helpers ────────────────────────────────────────────────────────

    /// Return indices into `self.entries` that pass the current filter.
    fn filtered_indices(&self) -> Vec<usize> {
        let lower = self.filter.to_lowercase();
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| lower.is_empty() || e.name.to_lowercase().contains(&lower))
            .map(|(i, _)| i)
            .collect()
    }

    // ── Rendering ─────────────────────────────────────────────────────────────

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        let high_count = self
            .entries
            .iter()
            .filter(|e| e.recovery_chance == RecoveryChance::High)
            .count();

        let title = match &self.status {
            RecoverStatus::Loading => " Quick Recover — scanning… ".to_string(),
            RecoverStatus::Idle | RecoverStatus::Done => format!(
                " Quick Recover — {} — {} deleted files found ",
                self.fs_type,
                self.entries.len()
            ),
            RecoverStatus::Recovering => " Quick Recover — recovering… ".to_string(),
        };

        let outer = Block::default().borders(Borders::ALL).title(title);

        if matches!(self.status, RecoverStatus::Loading) && self.entries.is_empty() {
            frame.render_widget(
                Paragraph::new(" Scanning filesystem for deleted files…").block(outer),
                area,
            );
            return;
        }

        let inner = outer.inner(area);
        frame.render_widget(outer, area);
        self.render_content(frame, inner, high_count);
    }

    fn render_content(&mut self, frame: &mut Frame, area: Rect, high_count: usize) {
        let show_filter = !self.filter.is_empty() || self.filter_editing;
        let filter_height: u16 = if show_filter { 1 } else { 0 };
        let status_height: u16 = 3;

        let constraints = if show_filter {
            vec![
                Constraint::Length(1),             // title line
                Constraint::Length(filter_height), // filter bar
                Constraint::Length(1),             // table header
                Constraint::Min(0),                // file list
                Constraint::Length(status_height), // status
            ]
        } else {
            vec![
                Constraint::Length(1),             // table header
                Constraint::Min(0),                // file list
                Constraint::Length(status_height), // status
            ]
        };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(area);

        let (_header_idx, list_idx, status_idx) = if show_filter {
            // chunk 0: title, chunk 1: filter, chunk 2: header, chunk 3: list, chunk 4: status
            self.render_filter_bar(frame, chunks[1]);
            (2usize, 3usize, 4usize)
        } else {
            (0usize, 1usize, 2usize)
        };

        // ── Table header ──────────────────────────────────────────────────────
        let header = Row::new([
            Cell::from("Sel"),
            Cell::from("Chance"),
            Cell::from("Size"),
            Cell::from("Modified"),
            Cell::from("Name"),
        ])
        .style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );

        // ── File rows ─────────────────────────────────────────────────────────
        let visible_indices = self.filtered_indices();

        let rows: Vec<Row> = visible_indices
            .iter()
            .enumerate()
            .map(|(vis_pos, &idx)| {
                let entry = &self.entries[idx];
                let checked = self.checked.contains(&idx);

                let sel_cell = Cell::from(if checked { "[x]" } else { "[ ]" });

                let (chance_text, chance_style) = chance_display(entry.recovery_chance);
                let chance_cell = Cell::from(chance_text).style(chance_style);

                let size_cell = Cell::from(fmt_bytes(entry.size));

                let date_str = entry
                    .modified
                    .map(format_unix_date)
                    .unwrap_or_else(|| "-".to_string());
                let date_cell = Cell::from(date_str);

                let name_style = if self.selected == vis_pos {
                    Style::default().add_modifier(Modifier::REVERSED)
                } else {
                    Style::default()
                };
                let name_cell = Cell::from(entry.name.clone()).style(name_style);

                Row::new([sel_cell, chance_cell, size_cell, date_cell, name_cell])
            })
            .collect();

        let widths = [
            Constraint::Length(5),  // sel
            Constraint::Length(8),  // chance
            Constraint::Length(10), // size
            Constraint::Length(12), // date
            Constraint::Min(20),    // name
        ];

        if rows.is_empty() {
            frame.render_widget(
                Paragraph::new(if self.filter.is_empty() {
                    " No deleted files found."
                } else {
                    " No files match the current filter."
                }),
                chunks[list_idx],
            );
        } else {
            let table = Table::new(rows, widths)
                .header(header)
                .row_highlight_style(Style::default());

            let mut ts = TableState::default().with_offset(self.scroll);
            frame.render_stateful_widget(table, chunks[list_idx], &mut ts);
        }

        // ── Status bar ────────────────────────────────────────────────────────
        self.render_status_bar(frame, chunks[status_idx], high_count);
    }

    fn render_filter_bar(&self, frame: &mut Frame, area: Rect) {
        let cursor = if self.filter_editing { "_" } else { "" };
        let text = format!(" Filter: {}{}", self.filter, cursor);
        frame.render_widget(
            Paragraph::new(text).style(Style::default().fg(Color::Yellow)),
            area,
        );
    }

    fn render_status_bar(&self, frame: &mut Frame, area: Rect, high_count: usize) {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(area);

        match &self.status {
            RecoverStatus::Loading => {
                frame.render_widget(
                    Paragraph::new(" Scanning filesystem for deleted files…")
                        .style(Style::default().fg(Color::Yellow)),
                    rows[0],
                );
            }
            RecoverStatus::Idle => {
                let total = self.entries.len();
                let checked = self.checked.len();
                let msg = format!(
                    " {total} files  |  {high_count} recoverable (High)  |  checked: {checked}"
                );
                frame.render_widget(
                    Paragraph::new(msg).style(Style::default().fg(Color::DarkGray)),
                    rows[0],
                );
                frame.render_widget(
                    Paragraph::new(
                        " Space: check  R: recover  /: filter  a: check-high  A: check-all  Esc: clear",
                    )
                    .style(Style::default().fg(Color::DarkGray)),
                    rows[1],
                );
            }
            RecoverStatus::Recovering => {
                if let Some(p) = &self.recover_progress {
                    let (ratio, label) = if p.total == 0 {
                        (0.0, "Preparing…".to_string())
                    } else {
                        let r = p.done as f64 / p.total as f64;
                        (
                            r,
                            format!(" Recovering: {}/{} — {}", p.done, p.total, p.current_path),
                        )
                    };
                    let gauge = Gauge::default()
                        .gauge_style(Style::default().fg(Color::Green).bg(Color::DarkGray))
                        .ratio(ratio.clamp(0.0, 1.0))
                        .label(label);
                    frame.render_widget(gauge, rows[0]);
                }
            }
            RecoverStatus::Done => {
                let style = Style::default().fg(Color::Green);
                let msg = self.last_result.as_deref().unwrap_or("Recovery complete.");
                frame.render_widget(Paragraph::new(format!(" {msg}")).style(style), rows[0]);
                frame.render_widget(
                    Paragraph::new(
                        " Output: ferrite_output/quick_recover/fs/  |  R: recover again",
                    )
                    .style(Style::default().fg(Color::DarkGray)),
                    rows[1],
                );
            }
        }

        // Line at bottom: file count summary in all states
        let total = self.entries.len();
        let summary = if total == 0 {
            Line::from(vec![Span::styled(
                " No deleted files detected.",
                Style::default().fg(Color::DarkGray),
            )])
        } else {
            let high = self
                .entries
                .iter()
                .filter(|e| e.recovery_chance == RecoveryChance::High)
                .count();
            let med = self
                .entries
                .iter()
                .filter(|e| e.recovery_chance == RecoveryChance::Medium)
                .count();
            let low = self
                .entries
                .iter()
                .filter(|e| e.recovery_chance == RecoveryChance::Low)
                .count();
            Line::from(vec![
                Span::raw(" "),
                Span::styled(
                    format!("[HIGH] {high}"),
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(
                    format!("[MED] {med}"),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(format!("[LOW] {low}"), Style::default().fg(Color::Red)),
            ])
        };
        frame.render_widget(Paragraph::new(summary), rows[2]);
    }
}

// ── Targeted parser wrapper ───────────────────────────────────────────────────

/// Wraps a real parser but overrides `deleted_files()` and `enumerate_files()`
/// to return only the caller-selected entries.  Used so we can reuse
/// `spawn_recovery_thread` unchanged.
struct TargetedParser {
    inner: Arc<dyn FilesystemParser>,
    targets: Vec<FileEntry>,
}

impl ferrite_filesystem::FilesystemParser for TargetedParser {
    fn filesystem_type(&self) -> FilesystemType {
        self.inner.filesystem_type()
    }

    fn root_directory(&self) -> ferrite_filesystem::Result<Vec<FileEntry>> {
        self.inner.root_directory()
    }

    fn list_directory(&self, path: &str) -> ferrite_filesystem::Result<Vec<FileEntry>> {
        self.inner.list_directory(path)
    }

    fn read_file(
        &self,
        entry: &FileEntry,
        writer: &mut dyn std::io::Write,
    ) -> ferrite_filesystem::Result<u64> {
        self.inner.read_file(entry, writer)
    }

    fn deleted_files(&self) -> ferrite_filesystem::Result<Vec<FileEntry>> {
        Ok(self.targets.clone())
    }

    fn enumerate_files(&self) -> ferrite_filesystem::Result<Vec<FileEntry>> {
        Ok(self.targets.clone())
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Sort entries: High first, then Medium, Low, Unknown; within each group by name.
fn sort_entries(entries: &mut [FileEntry]) {
    entries.sort_by(|a, b| {
        // RecoveryChance derives Ord: High < Medium < Low < Unknown (ascending).
        // We want High first, so reverse the order.
        b.recovery_chance
            .cmp(&a.recovery_chance)
            .reverse()
            .then_with(|| a.name.cmp(&b.name))
    });
}

fn chance_display(chance: RecoveryChance) -> (&'static str, Style) {
    match chance {
        RecoveryChance::High => (
            "[HIGH]  ",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        RecoveryChance::Medium => (
            "[MED]   ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        RecoveryChance::Low => ("[LOW]   ", Style::default().fg(Color::Red)),
        RecoveryChance::Unknown => ("[ ? ]   ", Style::default().fg(Color::DarkGray)),
    }
}

fn fmt_bytes(n: u64) -> String {
    const GIB: u64 = 1_073_741_824;
    const MIB: u64 = 1_048_576;
    const KIB: u64 = 1_024;
    if n >= GIB {
        format!("{:.1} GiB", n as f64 / GIB as f64)
    } else if n >= MIB {
        format!("{:.1} MiB", n as f64 / MIB as f64)
    } else if n >= KIB {
        format!("{:.1} KiB", n as f64 / KIB as f64)
    } else {
        format!("{n} B")
    }
}

/// Format a Unix timestamp as `YYYY-MM-DD`.
fn format_unix_date(secs: u64) -> String {
    // Simple Gregorian calendar calculation; good enough for display.
    let mut days = secs / 86_400;
    // Unix epoch = 1970-01-01
    let mut year = 1970u32;
    loop {
        let days_in_year = if is_leap(year) { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }
    let days_in_month: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut month = 1u32;
    for (m_idx, &dim) in days_in_month.iter().enumerate() {
        let dim = if m_idx == 1 && is_leap(year) {
            dim + 1
        } else {
            dim
        };
        if days < dim as u64 {
            break;
        }
        days -= dim as u64;
        month += 1;
    }
    let day = days as u32 + 1;
    format!("{year:04}-{month:02}-{day:02}")
}

fn is_leap(year: u32) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_chance_ordering() {
        assert!(RecoveryChance::High < RecoveryChance::Medium);
        assert!(RecoveryChance::Medium < RecoveryChance::Low);
        assert!(RecoveryChance::Low < RecoveryChance::Unknown);
    }

    #[test]
    fn set_device_with_no_parseable_fs_does_not_panic() {
        let data = vec![0u8; 2048];
        let mock = ferrite_blockdev::MockBlockDevice::new(data, 512);
        let dev: Arc<dyn BlockDevice> = Arc::new(mock);
        let mut state = QuickRecoverState::new();
        // set_device spawns a thread that will fail to open FS; should not panic.
        state.set_device(dev);
        // Give the thread a moment to finish, then tick.
        std::thread::sleep(std::time::Duration::from_millis(50));
        state.tick();
        // After the error, status should return to Idle.
        assert!(matches!(state.status, RecoverStatus::Idle));
    }

    #[test]
    fn sort_entries_orders_high_first() {
        let make_entry = |chance: RecoveryChance, name: &str| FileEntry {
            name: name.to_string(),
            path: format!("/{name}"),
            size: 100,
            is_dir: false,
            is_deleted: true,
            created: None,
            modified: None,
            first_cluster: None,
            mft_record: None,
            inode_number: None,
            data_byte_offset: None,
            recovery_chance: chance,
        };

        let mut entries = vec![
            make_entry(RecoveryChance::Low, "c.dat"),
            make_entry(RecoveryChance::High, "a.jpg"),
            make_entry(RecoveryChance::Medium, "b.png"),
            make_entry(RecoveryChance::Unknown, "d.tmp"),
        ];
        sort_entries(&mut entries);
        assert_eq!(entries[0].recovery_chance, RecoveryChance::High);
        assert_eq!(entries[1].recovery_chance, RecoveryChance::Medium);
        assert_eq!(entries[2].recovery_chance, RecoveryChance::Low);
        assert_eq!(entries[3].recovery_chance, RecoveryChance::Unknown);
    }

    #[test]
    fn fmt_bytes_ranges() {
        assert_eq!(fmt_bytes(0), "0 B");
        assert_eq!(fmt_bytes(512), "512 B");
        assert_eq!(fmt_bytes(1_024), "1.0 KiB");
        assert_eq!(fmt_bytes(1_048_576), "1.0 MiB");
        assert_eq!(fmt_bytes(1_073_741_824), "1.0 GiB");
    }

    #[test]
    fn format_unix_date_epoch() {
        assert_eq!(format_unix_date(0), "1970-01-01");
    }

    #[test]
    fn format_unix_date_known() {
        // 2024-03-15 = 19802 days from epoch = 1_710_460_800 s (approximate; midnight UTC)
        // Verify year/month/day loosely.
        let s = format_unix_date(1_710_460_800);
        assert!(s.starts_with("2024-"), "expected 2024, got {s}");
    }
}

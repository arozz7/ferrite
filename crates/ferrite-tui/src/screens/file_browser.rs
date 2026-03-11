//! Screen 5 — File Browser: navigate filesystem directory trees, including
//! deleted file discovery.

use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyModifiers};
use ferrite_blockdev::BlockDevice;
use ferrite_filesystem::{open_filesystem, FileEntry, FilesystemParser, FilesystemType};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
    Frame,
};

// ── Types ─────────────────────────────────────────────────────────────────────

enum BrowserMsg {
    Opened(Box<dyn FilesystemParser>),
    Error(String),
}

// SAFETY: FilesystemParser: Send + Sync, so Box<dyn FilesystemParser>: Send.
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
    parser: Option<Box<dyn FilesystemParser>>,
    fs_type: FilesystemType,
    path_segments: Vec<String>,
    entries: Vec<FileEntry>,
    selected: usize,
    scroll: usize,
    show_deleted: bool,
    status: BrowserStatus,
    open_rx: Option<Receiver<BrowserMsg>>,
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
        }
    }

    pub fn set_device(&mut self, device: Arc<dyn BlockDevice>) {
        self.device = Some(device);
        self.parser = None;
        self.path_segments.clear();
        self.entries.clear();
        self.selected = 0;
        self.scroll = 0;
        self.status = BrowserStatus::Idle;
        self.open_rx = None;
    }

    /// Returns `true` while a text-input field is active (currently none on this screen).
    pub fn is_editing(&self) -> bool {
        false
    }

    /// Drain background channels.
    pub fn tick(&mut self) {
        let rx = match &self.open_rx {
            Some(r) => r,
            None => return,
        };
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

    pub fn handle_key(&mut self, code: KeyCode, _modifiers: KeyModifiers) {
        match code {
            KeyCode::Char('o') => self.start_open(),
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

    fn start_open(&mut self) {
        let device = match &self.device {
            Some(d) => Arc::clone(d),
            None => return,
        };
        // Detect filesystem type before spawning.
        self.fs_type = ferrite_filesystem::detect_filesystem(device.as_ref());
        self.status = BrowserStatus::Opening;
        let (tx, rx) = mpsc::channel();
        self.open_rx = Some(rx);
        std::thread::spawn(move || match open_filesystem(device) {
            Ok(parser) => {
                let _ = tx.send(BrowserMsg::Opened(parser));
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
                    // Show deleted files at the current directory level.
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

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        let title = match &self.status {
            BrowserStatus::Opening => " File Browser — opening filesystem… ",
            _ if self.show_deleted => " File Browser — [deleted shown] — d: toggle  o: open fs ",
            _ => " File Browser — d: toggle deleted  o: open filesystem ",
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
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0)])
            .split(area);

        // Breadcrumb bar
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

        if self.entries.is_empty() {
            frame.render_widget(Paragraph::new(" (empty directory)"), chunks[1]);
            return;
        }

        let header = Row::new([Cell::from("Name"), Cell::from("Size"), Cell::from("Type")]).style(
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
}

// FileEntry doesn't impl PartialEq — add a local helper.
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
    }

    #[test]
    fn set_device_resets_state() {
        let mut s = FileBrowserState::new();
        s.path_segments.push("Windows".into());
        s.selected = 3;

        // We can't easily create a real Arc<dyn BlockDevice> in a unit test,
        // so we verify that calling set_device with a mock resets fields.
        // Use the existing MockBlockDevice for this.
        let data = vec![0u8; 512];
        let mock = ferrite_blockdev::MockBlockDevice::new(data, 512);
        let dev: Arc<dyn BlockDevice> = Arc::new(mock);
        s.set_device(dev);

        assert!(s.path_segments.is_empty());
        assert_eq!(s.selected, 0);
        assert!(s.entries.is_empty());
    }
}

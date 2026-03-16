//! Screen 1 — Drive Selection: enumerate block devices and select one.

use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyModifiers};
use ferrite_blockdev::BlockDevice;
use ferrite_core::types::DeviceInfo;
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
    Frame,
};
use tracing::debug;

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct DriveEntry {
    pub path: String,
    pub info: Option<DeviceInfo>,
    pub error: Option<String>,
}

enum DriveMsg {
    Entries(Vec<DriveEntry>),
}

#[derive(PartialEq)]
enum DriveStatus {
    Idle,
    Loading,
    Loaded,
    Error(String),
}

#[derive(Debug, Clone, PartialEq)]
enum SortKey {
    Path,
    SizeDesc,
}

impl SortKey {
    fn next(&self) -> Self {
        match self {
            SortKey::Path => SortKey::SizeDesc,
            SortKey::SizeDesc => SortKey::Path,
        }
    }

    fn label(&self) -> &'static str {
        match self {
            SortKey::Path => "Path",
            SortKey::SizeDesc => "Size ↓",
        }
    }
}

// ── State ─────────────────────────────────────────────────────────────────────

pub struct DriveSelectState {
    entries: Vec<DriveEntry>,
    selected: usize,
    status: DriveStatus,
    rx: Option<Receiver<DriveMsg>>,
    sort_key: SortKey,
    filter_input: String,
    filtering: bool,
}

impl Default for DriveSelectState {
    fn default() -> Self {
        Self::new()
    }
}

impl DriveSelectState {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            selected: 0,
            status: DriveStatus::Idle,
            rx: None,
            sort_key: SortKey::Path,
            filter_input: String::new(),
            filtering: false,
        }
    }

    /// Returns indices into `self.entries` in the current sort + filter order.
    fn display_indices(&self) -> Vec<usize> {
        let filter = self.filter_input.to_lowercase();
        let mut indices: Vec<usize> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                if filter.is_empty() {
                    return true;
                }
                if e.path.to_lowercase().contains(&filter) {
                    return true;
                }
                if let Some(info) = &e.info {
                    if info
                        .model
                        .as_deref()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&filter)
                    {
                        return true;
                    }
                    if info
                        .serial
                        .as_deref()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&filter)
                    {
                        return true;
                    }
                }
                false
            })
            .map(|(i, _)| i)
            .collect();

        match self.sort_key {
            SortKey::Path => {
                indices.sort_by(|&a, &b| self.entries[a].path.cmp(&self.entries[b].path))
            }
            SortKey::SizeDesc => indices.sort_by(|&a, &b| {
                let sa = self.entries[a].info.as_ref().map_or(0, |i| i.size_bytes);
                let sb = self.entries[b].info.as_ref().map_or(0, |i| i.size_bytes);
                sb.cmp(&sa)
            }),
        }
        indices
    }

    /// Drain the background enumeration channel.
    pub fn tick(&mut self) {
        let rx = match &self.rx {
            Some(r) => r,
            None => return,
        };
        match rx.try_recv() {
            Ok(DriveMsg::Entries(entries)) => {
                self.entries = entries;
                self.selected = 0;
                self.status = DriveStatus::Loaded;
                self.rx = None;
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                self.rx = None;
                if self.status == DriveStatus::Loading {
                    self.status = DriveStatus::Error("enumeration thread disconnected".into());
                }
            }
        }
    }

    /// Returns `true` while the filter bar is open (so `q` won't quit).
    pub fn is_filtering(&self) -> bool {
        self.filtering
    }

    /// Handle key events.  Returns `Some(Arc<dyn BlockDevice>)` when the user
    /// presses Enter to select a device.
    pub fn handle_key(
        &mut self,
        code: KeyCode,
        _modifiers: KeyModifiers,
    ) -> Option<Arc<dyn BlockDevice>> {
        if self.filtering {
            match code {
                KeyCode::Esc => {
                    self.filtering = false;
                    self.filter_input.clear();
                    self.selected = 0;
                }
                KeyCode::Enter => {
                    self.filtering = false;
                }
                KeyCode::Backspace => {
                    self.filter_input.pop();
                    self.selected = 0;
                }
                KeyCode::Char(c) => {
                    self.filter_input.push(c);
                    self.selected = 0;
                }
                _ => {}
            }
            return None;
        }

        match code {
            KeyCode::Up => {
                if self.selected > 0 {
                    self.selected -= 1;
                }
            }
            KeyCode::Down => {
                let count = self.display_indices().len();
                if self.selected + 1 < count {
                    self.selected += 1;
                }
            }
            KeyCode::Char('r') => self.start_enumerate(),
            KeyCode::Char('s') => {
                self.sort_key = self.sort_key.next();
                self.selected = 0;
            }
            KeyCode::Char('/') => {
                self.filtering = true;
                self.filter_input.clear();
                self.selected = 0;
            }
            KeyCode::Esc => {
                if !self.filter_input.is_empty() {
                    self.filter_input.clear();
                    self.selected = 0;
                }
            }
            KeyCode::Enter => return self.open_selected(),
            _ => {}
        }
        None
    }

    fn start_enumerate(&mut self) {
        if self.status == DriveStatus::Loading {
            return;
        }
        self.status = DriveStatus::Loading;
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        std::thread::spawn(move || {
            let paths = platform_enumerate();
            debug!(count = paths.len(), "enumeration started");
            let entries: Vec<DriveEntry> = paths
                .into_iter()
                .map(|path| {
                    let (info, error) = match platform_get_info(&path) {
                        Some(info) => (Some(info), None),
                        None => (None, Some("open failed (admin required?)".into())),
                    };
                    DriveEntry { path, info, error }
                })
                .collect();
            debug!(count = entries.len(), "enumeration complete");
            let _ = tx.send(DriveMsg::Entries(entries));
        });
    }

    fn open_selected(&self) -> Option<Arc<dyn BlockDevice>> {
        let indices = self.display_indices();
        let entry_idx = *indices.get(self.selected)?;
        let entry = self.entries.get(entry_idx)?;
        platform_open(&entry.path)
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        // Auto-enumerate on first render.
        if self.status == DriveStatus::Idle && self.rx.is_none() {
            self.start_enumerate();
        }

        let sort_label = self.sort_key.label();
        let title = format!(
            " Drive Selection — r: refresh  s: sort [{}]  /: filter  o: sessions  Enter: select ",
            sort_label
        );
        let block = Block::default().borders(Borders::ALL).title(title);

        match &self.status {
            DriveStatus::Idle | DriveStatus::Loading => {
                frame.render_widget(
                    Paragraph::new(" Scanning for block devices…").block(block),
                    area,
                );
            }
            DriveStatus::Error(e) => {
                let msg = format!(" Error: {e}\n Press r to retry.");
                frame.render_widget(
                    Paragraph::new(msg)
                        .style(Style::default().fg(Color::Red))
                        .block(block),
                    area,
                );
            }
            DriveStatus::Loaded => {
                let indices = self.display_indices();

                // Split off a filter bar row at the bottom when filtering is
                // active or a filter string is set.
                let show_filter_bar = self.filtering || !self.filter_input.is_empty();
                let (table_area, filter_area_opt) = if show_filter_bar {
                    let chunks =
                        Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).split(area);
                    (chunks[0], Some(chunks[1]))
                } else {
                    (area, None)
                };

                if indices.is_empty() {
                    let msg = if self.filter_input.is_empty() {
                        " No block devices found.\n Press r to refresh.".to_string()
                    } else {
                        format!(
                            " No matches for \"{}\". Press Esc to clear filter.",
                            self.filter_input
                        )
                    };
                    frame.render_widget(Paragraph::new(msg).block(block), table_area);
                } else {
                    let header = Row::new([
                        Cell::from("  #"),
                        Cell::from("Path"),
                        Cell::from("Model"),
                        Cell::from("Serial"),
                        Cell::from("Size"),
                    ])
                    .style(
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    );

                    let selected = self.selected;
                    let rows: Vec<Row> = indices
                        .iter()
                        .enumerate()
                        .map(|(display_i, &entry_i)| {
                            let e = &self.entries[entry_i];
                            let (model, serial, size) = match &e.info {
                                Some(info) => (
                                    info.model.as_deref().unwrap_or("—").to_string(),
                                    info.serial.as_deref().unwrap_or("—").to_string(),
                                    fmt_bytes(info.size_bytes),
                                ),
                                None => (
                                    e.error.as_deref().unwrap_or("—").to_string(),
                                    "—".into(),
                                    "—".into(),
                                ),
                            };
                            Row::new([
                                Cell::from(format!("{:>3}", display_i)),
                                Cell::from(e.path.clone()),
                                Cell::from(model),
                                Cell::from(serial),
                                Cell::from(size),
                            ])
                        })
                        .collect();

                    let widths = [
                        Constraint::Length(4),
                        Constraint::Length(26),
                        Constraint::Min(18),
                        Constraint::Length(22),
                        Constraint::Length(10),
                    ];

                    let table = Table::new(rows, widths)
                        .header(header)
                        .block(block)
                        .row_highlight_style(
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD),
                        );

                    let mut state = TableState::default().with_selected(Some(selected));
                    frame.render_stateful_widget(table, table_area, &mut state);
                }

                // Render filter bar.
                if let Some(filter_area) = filter_area_opt {
                    let cursor = if self.filtering { "█" } else { "" };
                    let filter_text = format!(" Filter: {}{} ", self.filter_input, cursor);
                    let style = if self.filtering {
                        Style::default().fg(Color::Yellow)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    };
                    frame.render_widget(Paragraph::new(filter_text).style(style), filter_area);
                }
            }
        }
    }
}

// ── Platform helpers ──────────────────────────────────────────────────────────

pub(crate) fn platform_enumerate() -> Vec<String> {
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    {
        ferrite_blockdev::enumerate_devices()
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        Vec::new()
    }
}

pub(crate) fn platform_get_info(path: &str) -> Option<DeviceInfo> {
    #[cfg(target_os = "windows")]
    {
        ferrite_blockdev::WindowsBlockDevice::open(path)
            .ok()
            .map(|d| d.device_info().clone())
    }
    #[cfg(target_os = "linux")]
    {
        ferrite_blockdev::LinuxBlockDevice::open(path)
            .ok()
            .map(|d| d.device_info().clone())
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        let _ = path;
        None
    }
}

pub(crate) fn platform_open(path: &str) -> Option<Arc<dyn BlockDevice>> {
    #[cfg(target_os = "windows")]
    {
        ferrite_blockdev::WindowsBlockDevice::open(path)
            .ok()
            .map(|d| Arc::new(d) as Arc<dyn BlockDevice>)
    }
    #[cfg(target_os = "linux")]
    {
        ferrite_blockdev::LinuxBlockDevice::open(path)
            .ok()
            .map(|d| Arc::new(d) as Arc<dyn BlockDevice>)
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        let _ = path;
        None
    }
}

// ── Formatting ────────────────────────────────────────────────────────────────

fn fmt_bytes(n: u64) -> String {
    const GIB: u64 = 1024 * 1024 * 1024;
    const MIB: u64 = 1024 * 1024;
    if n >= GIB {
        format!("{:.1} GiB", n as f64 / GIB as f64)
    } else if n >= MIB {
        format!("{:.1} MiB", n as f64 / MIB as f64)
    } else {
        format!("{} B", n)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(path: &str, size: u64) -> DriveEntry {
        DriveEntry {
            path: path.into(),
            info: Some(DeviceInfo {
                path: path.into(),
                model: None,
                serial: None,
                size_bytes: size,
                sector_size: 512,
                logical_sector_size: 512,
            }),
            error: None,
        }
    }

    #[test]
    fn navigation_does_not_underflow() {
        let mut s = DriveSelectState::new();
        s.entries = vec![
            DriveEntry {
                path: "/dev/sda".into(),
                info: None,
                error: None,
            },
            DriveEntry {
                path: "/dev/sdb".into(),
                info: None,
                error: None,
            },
        ];
        s.status = DriveStatus::Loaded;
        s.handle_key(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn navigation_does_not_overflow() {
        let mut s = DriveSelectState::new();
        s.entries = vec![DriveEntry {
            path: "/dev/sda".into(),
            info: None,
            error: None,
        }];
        s.status = DriveStatus::Loaded;
        s.handle_key(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn sort_size_desc_orders_largest_first() {
        let mut s = DriveSelectState::new();
        s.entries = vec![
            make_entry("/dev/sda", 100),
            make_entry("/dev/sdb", 500),
            make_entry("/dev/sdc", 200),
        ];
        s.status = DriveStatus::Loaded;
        s.sort_key = SortKey::SizeDesc;
        let indices = s.display_indices();
        let sizes: Vec<u64> = indices
            .iter()
            .map(|&i| s.entries[i].info.as_ref().unwrap().size_bytes)
            .collect();
        assert_eq!(sizes, vec![500, 200, 100]);
    }

    #[test]
    fn filter_matches_path() {
        let mut s = DriveSelectState::new();
        s.entries = vec![make_entry("/dev/sda", 100), make_entry("/dev/nvme0n1", 500)];
        s.status = DriveStatus::Loaded;
        s.filter_input = "nvme".into();
        let indices = s.display_indices();
        assert_eq!(indices.len(), 1);
        assert_eq!(s.entries[indices[0]].path, "/dev/nvme0n1");
    }

    #[test]
    fn filter_clear_on_esc() {
        let mut s = DriveSelectState::new();
        s.entries = vec![make_entry("/dev/sda", 100)];
        s.status = DriveStatus::Loaded;
        s.filtering = true;
        s.filter_input = "sd".into();
        s.handle_key(KeyCode::Esc, KeyModifiers::NONE);
        assert!(!s.filtering);
        assert!(s.filter_input.is_empty());
    }

    #[test]
    fn fmt_bytes_gib() {
        let s = fmt_bytes(2 * 1024 * 1024 * 1024);
        assert_eq!(s, "2.0 GiB");
    }
}

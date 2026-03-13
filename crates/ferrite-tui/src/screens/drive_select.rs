//! Screen 1 — Drive Selection: enumerate block devices and select one.

use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyModifiers};
use ferrite_blockdev::BlockDevice;
use ferrite_core::types::DeviceInfo;
use ratatui::{
    layout::{Constraint, Rect},
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

// ── State ─────────────────────────────────────────────────────────────────────

pub struct DriveSelectState {
    entries: Vec<DriveEntry>,
    selected: usize,
    status: DriveStatus,
    rx: Option<Receiver<DriveMsg>>,
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
        }
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

    /// Handle key events.  Returns `Some(Arc<dyn BlockDevice>)` when the user
    /// presses Enter to select a device.
    pub fn handle_key(
        &mut self,
        code: KeyCode,
        _modifiers: KeyModifiers,
    ) -> Option<Arc<dyn BlockDevice>> {
        match code {
            KeyCode::Up => {
                if self.selected > 0 {
                    self.selected -= 1;
                }
            }
            KeyCode::Down => {
                if self.selected + 1 < self.entries.len() {
                    self.selected += 1;
                }
            }
            KeyCode::Char('r') => self.start_enumerate(),
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
        let entry = self.entries.get(self.selected)?;
        platform_open(&entry.path)
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        // Auto-enumerate on first render.
        if self.status == DriveStatus::Idle && self.rx.is_none() {
            self.start_enumerate();
        }

        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Drive Selection — press r to refresh ");

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
                if self.entries.is_empty() {
                    frame.render_widget(
                        Paragraph::new(" No block devices found.\n Press r to refresh.")
                            .block(block),
                        area,
                    );
                    return;
                }

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
                let rows: Vec<Row> = self
                    .entries
                    .iter()
                    .enumerate()
                    .map(|(i, e)| {
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
                            Cell::from(format!("{:>3}", i)),
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
                frame.render_stateful_widget(table, area, &mut state);
            }
        }
    }
}

// ── Platform helpers ──────────────────────────────────────────────────────────

fn platform_enumerate() -> Vec<String> {
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    {
        ferrite_blockdev::enumerate_devices()
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        Vec::new()
    }
}

fn platform_get_info(path: &str) -> Option<DeviceInfo> {
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

fn platform_open(path: &str) -> Option<Arc<dyn BlockDevice>> {
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
    fn fmt_bytes_gib() {
        let s = fmt_bytes(2 * 1024 * 1024 * 1024);
        assert_eq!(s, "2.0 GiB");
    }
}

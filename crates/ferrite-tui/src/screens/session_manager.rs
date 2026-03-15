//! Session manager overlay — browse, resume, or delete saved carving sessions.
//!
//! Rendered as a centred popup on top of the current screen.

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyModifiers};
use ferrite_blockdev::BlockDevice;
use ferrite_core::types::DeviceInfo;
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState},
    Frame,
};

use crate::carving_session::CarvingSession;
use crate::screens::drive_select::{platform_enumerate, platform_get_info, platform_open};

// ── Public API ────────────────────────────────────────────────────────────────

/// Message returned by [`SessionManagerState::handle_key`].
pub enum SessionMsg {
    /// User pressed Enter on a session whose drive is connected.
    Resume {
        session: CarvingSession,
        device: Arc<dyn BlockDevice>,
    },
    /// User dismissed the overlay without resuming.
    Dismissed,
}

// ── Internal state ────────────────────────────────────────────────────────────

enum VerifyState {
    Unknown,
    Matched(String), // device path
    NotFound,
}

pub struct SessionManagerState {
    pub visible: bool,
    sessions: Vec<CarvingSession>,
    selected: usize,
    connected: Vec<(String, DeviceInfo)>,
    verify: VerifyState,
}

impl Default for SessionManagerState {
    fn default() -> Self {
        Self {
            visible: false,
            sessions: Vec::new(),
            selected: 0,
            connected: Vec::new(),
            verify: VerifyState::Unknown,
        }
    }
}

impl SessionManagerState {
    /// Open the overlay and populate the session list.
    pub fn open(&mut self) {
        self.sessions = CarvingSession::load_all();
        self.selected = 0;
        self.visible = true;
        self.refresh_drives();
    }

    fn refresh_drives(&mut self) {
        self.connected = platform_enumerate()
            .into_iter()
            .filter_map(|p| platform_get_info(&p).map(|info| (p, info)))
            .collect();
        self.update_verify();
    }

    fn update_verify(&mut self) {
        let Some(s) = self.sessions.get(self.selected) else {
            self.verify = VerifyState::Unknown;
            return;
        };
        if let Some((path, _)) = self.connected.iter().find(|(_, info)| {
            s.matches_drive(info.serial.as_deref().unwrap_or(""), info.size_bytes)
        }) {
            self.verify = VerifyState::Matched(path.clone());
        } else {
            self.verify = VerifyState::NotFound;
        }
    }

    /// Handle a key event.  Returns a [`SessionMsg`] when an action is taken.
    pub fn handle_key(&mut self, code: KeyCode, _mods: KeyModifiers) -> Option<SessionMsg> {
        match code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.visible = false;
                return Some(SessionMsg::Dismissed);
            }
            KeyCode::Up => {
                if self.selected > 0 {
                    self.selected -= 1;
                    self.update_verify();
                }
            }
            KeyCode::Down => {
                if self.selected + 1 < self.sessions.len() {
                    self.selected += 1;
                    self.update_verify();
                }
            }
            KeyCode::Char('r') => self.refresh_drives(),
            KeyCode::Char('d') => {
                if let Some(s) = self.sessions.get(self.selected) {
                    let _ = s.delete();
                    self.sessions.remove(self.selected);
                    if self.selected > 0 && self.selected >= self.sessions.len() {
                        self.selected -= 1;
                    }
                    self.update_verify();
                }
            }
            KeyCode::Enter => {
                if let VerifyState::Matched(path) = &self.verify {
                    let path = path.clone();
                    let session = self.sessions[self.selected].clone();
                    if let Some(device) = platform_open(&path) {
                        self.visible = false;
                        return Some(SessionMsg::Resume { session, device });
                    }
                }
            }
            _ => {}
        }
        None
    }

    /// Render the session manager popup over `full_area`.
    pub fn render(&self, frame: &mut Frame, full_area: Rect) {
        // Centred popup: 80 % width, 60 % height, with sane minimums.
        let width = (full_area.width * 4 / 5).max(50).min(full_area.width);
        let height = (full_area.height * 3 / 5).max(20).min(full_area.height);
        let x = (full_area.width.saturating_sub(width)) / 2;
        let y = (full_area.height.saturating_sub(height)) / 2;
        let area = Rect {
            x: full_area.x + x,
            y: full_area.y + y,
            width,
            height,
        };

        // Clear the area beneath the popup.
        frame.render_widget(Clear, area);

        let block = Block::default()
            .borders(Borders::ALL)
            .title(
                " Saved Sessions  \u{2191}\u{2193}: navigate  Enter: resume  d: delete  r: refresh  Esc: close ",
            )
            .style(Style::default().fg(Color::Cyan));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if self.sessions.is_empty() {
            frame.render_widget(
                Paragraph::new(
                    " No saved sessions found.\n\n Sessions are created automatically \
                     when you carve a drive and quit.",
                )
                .style(Style::default().fg(Color::DarkGray)),
                inner,
            );
            return;
        }

        // Split inner: table on top, verify panel at bottom (3 lines).
        let chunks = Layout::vertical([Constraint::Min(4), Constraint::Length(3)]).split(inner);

        // Build the table.
        let header = Row::new([
            Cell::from("Drive"),
            Cell::from("Size"),
            Cell::from("Hits"),
            Cell::from("Scan range"),
            Cell::from("Saved"),
        ])
        .style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );

        let rows: Vec<Row> = self
            .sessions
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let drive = if s.drive_model.is_empty() {
                    s.drive_serial.clone()
                } else {
                    format!("{} ({})", s.drive_model, &s.drive_serial)
                };
                let range = if s.scan_start_lba == 0 && s.scan_end_lba == 0 {
                    "full device".into()
                } else if s.scan_end_lba == 0 {
                    format!("LBA {}→end", s.scan_start_lba)
                } else {
                    format!("LBA {}→{}", s.scan_start_lba, s.scan_end_lba)
                };
                let style = if i == self.selected {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                Row::new([
                    Cell::from(drive),
                    Cell::from(CarvingSession::fmt_bytes(s.drive_size)),
                    Cell::from(s.hits_count.to_string()),
                    Cell::from(range),
                    Cell::from(s.age_str()),
                ])
                .style(style)
            })
            .collect();

        let widths = [
            Constraint::Min(24),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Min(16),
            Constraint::Length(10),
        ];
        let mut ts = TableState::default().with_selected(Some(self.selected));
        frame.render_stateful_widget(Table::new(rows, widths).header(header), chunks[0], &mut ts);

        // Drive verification panel.
        let msg = match &self.verify {
            VerifyState::Unknown => " Select a session to check drive status.".to_string(),
            VerifyState::Matched(path) => {
                format!(" \u{2713} Drive connected at {path}  \u{2014}  press Enter to resume")
            }
            VerifyState::NotFound => {
                " Drive not found.  Connect the drive and press r to refresh.".to_string()
            }
        };
        let color = match &self.verify {
            VerifyState::Matched(_) => Color::Green,
            VerifyState::NotFound => Color::Yellow,
            VerifyState::Unknown => Color::DarkGray,
        };
        frame.render_widget(
            Paragraph::new(msg)
                .style(Style::default().fg(color))
                .block(Block::default().borders(Borders::TOP)),
            chunks[1],
        );
    }
}

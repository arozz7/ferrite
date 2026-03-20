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

use ferrite_blockdev::FileBlockDevice;

use crate::carving_session::CarvingSession;
use crate::screens::drive_select::{platform_enumerate, platform_get_info, platform_open};

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Returns `true` for `\\.\PhysicalDriveN` paths, `false` for image files.
fn is_physical_drive(path: &str) -> bool {
    path.to_lowercase().starts_with(r"\\.\physicaldrive")
}

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
    /// When `Some`, the image-link overlay is active and holds the typed path.
    image_input: Option<String>,
    /// Error from the last failed image-link attempt.
    image_error: Option<String>,
}

impl Default for SessionManagerState {
    fn default() -> Self {
        Self {
            visible: false,
            sessions: Vec::new(),
            selected: 0,
            connected: Vec::new(),
            verify: VerifyState::Unknown,
            image_input: None,
            image_error: None,
        }
    }
}

impl SessionManagerState {
    /// Open the overlay and populate the session list.
    pub fn open(&mut self) {
        self.sessions = CarvingSession::load_all();
        self.selected = 0;
        self.visible = true;
        self.image_input = None;
        self.image_error = None;
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
        // Physical drive: match by serial + size.
        if let Some((path, _)) = self.connected.iter().find(|(_, info)| {
            s.matches_drive(info.serial.as_deref().unwrap_or(""), info.size_bytes)
        }) {
            self.verify = VerifyState::Matched(path.clone());
            return;
        }
        // Image file: check that the stored path still exists on disk.
        if !s.device_path.is_empty()
            && !is_physical_drive(&s.device_path)
            && std::path::Path::new(&s.device_path).exists()
        {
            self.verify = VerifyState::Matched(s.device_path.clone());
            return;
        }
        self.verify = VerifyState::NotFound;
    }

    /// Returns `true` when the image-link overlay is open and capturing input.
    pub fn image_input_active(&self) -> bool {
        self.image_input.is_some()
    }

    /// Append pasted text into the image-link input if it is active.
    pub fn handle_paste(&mut self, text: &str) {
        if let Some(ref mut input) = self.image_input {
            let clean: String = text.chars().filter(|&c| c != '\n' && c != '\r').collect();
            input.push_str(&clean);
            self.image_error = None;
        }
    }

    /// Handle a key event.  Returns a [`SessionMsg`] when an action is taken.
    pub fn handle_key(&mut self, code: KeyCode, mods: KeyModifiers) -> Option<SessionMsg> {
        // Image-link overlay takes priority.
        if let Some(ref mut input) = self.image_input {
            match code {
                KeyCode::Esc => {
                    self.image_input = None;
                    self.image_error = None;
                }
                KeyCode::Backspace => {
                    input.pop();
                    self.image_error = None;
                }
                // Only push printable characters; modifier combos (Ctrl+V etc.)
                // are handled at the App level before reaching here.
                KeyCode::Char(c) if mods.is_empty() => {
                    input.push(c);
                    self.image_error = None;
                }
                KeyCode::Enter => {
                    let path = input.trim().to_owned();
                    if std::path::Path::new(&path).exists() {
                        // Update the session's device_path and save to disk.
                        if let Some(s) = self.sessions.get_mut(self.selected) {
                            s.device_path = path;
                            let _ = s.save();
                        }
                        self.image_input = None;
                        self.image_error = None;
                        self.update_verify();
                    } else {
                        self.image_error = Some(format!("Not found: {path}"));
                    }
                }
                _ => {}
            }
            return None;
        }

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
            KeyCode::Char('f') => {
                self.image_input = Some(String::new());
                self.image_error = None;
            }
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
                    // Open as image file or physical drive depending on path type.
                    let device = if is_physical_drive(&path) {
                        platform_open(&path)
                    } else {
                        FileBlockDevice::open(&path)
                            .ok()
                            .map(|d| Arc::new(d) as Arc<dyn BlockDevice>)
                    };
                    if let Some(device) = device {
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
                " Saved Sessions  \u{2191}\u{2193}: navigate  Enter: resume  f: link image  d: delete  r: refresh  Esc: close ",
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
                    if s.device_path.is_empty() {
                        s.drive_serial.clone()
                    } else {
                        // Show the filename portion of the image path.
                        std::path::Path::new(&s.device_path)
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or(&s.device_path)
                            .to_string()
                    }
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
                format!(" \u{2713} Ready at {path}  \u{2014}  press Enter to resume")
            }
            VerifyState::NotFound => {
                " Drive or image file not found.  Press f to link an image file, or r to refresh."
                    .to_string()
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

        // Image-link overlay — floats above everything inside the popup.
        if let Some(ref input) = self.image_input {
            let overlay_w = area.width.saturating_sub(4).min(72);
            let overlay_h = 5u16;
            let overlay_x = area.x + (area.width.saturating_sub(overlay_w)) / 2;
            let overlay_y = area.y + area.height.saturating_sub(overlay_h + 2);
            let overlay = Rect {
                x: overlay_x,
                y: overlay_y,
                width: overlay_w,
                height: overlay_h,
            };
            let hint = match &self.image_error {
                Some(e) => format!(" {e} "),
                None => " Enter path to .img file  ·  Esc: cancel  ·  Ctrl+V: paste ".into(),
            };
            let hint_color = if self.image_error.is_some() {
                Color::Red
            } else {
                Color::DarkGray
            };
            let text = format!("\n  Path: {}█\n\n  {}", input, hint.trim());
            frame.render_widget(Clear, overlay);
            frame.render_widget(
                Paragraph::new(text)
                    .style(Style::default().fg(Color::Yellow))
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(" Link Image File ")
                            .border_style(Style::default().fg(Color::Cyan)),
                    ),
                overlay,
            );
            // Render hint in its own colour on the last interior line.
            if overlay.height >= 4 {
                let hint_area = Rect {
                    x: overlay.x + 1,
                    y: overlay.y + overlay.height - 2,
                    width: overlay.width.saturating_sub(2),
                    height: 1,
                };
                frame.render_widget(
                    Paragraph::new(format!("  {}", hint.trim()))
                        .style(Style::default().fg(hint_color)),
                    hint_area,
                );
            }
        }
    }
}

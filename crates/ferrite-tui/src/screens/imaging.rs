//! Screen 3 — Imaging Setup + Progress: configure and run the ddrescue-style
//! imaging engine with live progress updates.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyModifiers};
use ferrite_blockdev::BlockDevice;
use ferrite_imaging::{ImagingConfig, ImagingEngine, ProgressReporter, ProgressUpdate, Signal};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph},
    Frame,
};

// ── Types ─────────────────────────────────────────────────────────────────────

enum ImagingMsg {
    Progress(ProgressUpdate),
    Done,
    Error(String),
}

#[derive(PartialEq, Clone)]
enum ImagingStatus {
    Idle,
    Running,
    Complete,
    Cancelled,
    Error(String),
}

/// Which text field is being edited.
#[derive(Debug, PartialEq, Clone, Copy)]
enum EditField {
    Dest,
    Mapfile,
}

/// `ProgressReporter` impl that forwards updates through a sync channel.
struct ChannelReporter {
    tx: SyncSender<ImagingMsg>,
    cancel: Arc<AtomicBool>,
}

impl ProgressReporter for ChannelReporter {
    fn report(&mut self, update: &ProgressUpdate) -> Signal {
        let _ = self.tx.try_send(ImagingMsg::Progress(update.clone()));
        if self.cancel.load(Ordering::Relaxed) {
            Signal::Cancel
        } else {
            Signal::Continue
        }
    }
}

// ── State ─────────────────────────────────────────────────────────────────────

pub struct ImagingState {
    device: Option<Arc<dyn BlockDevice>>,
    /// Destination image file path (editable).
    pub dest_path: String,
    /// Mapfile path (editable, empty = no persistence).
    pub mapfile_path: String,
    edit_field: Option<EditField>,
    status: ImagingStatus,
    latest: Option<ProgressUpdate>,
    cancel: Arc<AtomicBool>,
    rx: Option<Receiver<ImagingMsg>>,
}

impl Default for ImagingState {
    fn default() -> Self {
        Self::new()
    }
}

impl ImagingState {
    pub fn new() -> Self {
        Self {
            device: None,
            dest_path: String::new(),
            mapfile_path: String::new(),
            edit_field: None,
            status: ImagingStatus::Idle,
            latest: None,
            cancel: Arc::new(AtomicBool::new(false)),
            rx: None,
        }
    }

    pub fn set_device(&mut self, device: Arc<dyn BlockDevice>) {
        self.device = Some(device);
        self.status = ImagingStatus::Idle;
        self.latest = None;
        self.cancel.store(false, Ordering::Relaxed);
        self.rx = None;
    }

    /// Returns `true` while the user is typing into a path field.
    pub fn is_editing(&self) -> bool {
        self.edit_field.is_some()
    }

    /// Drain the background imaging channel.
    pub fn tick(&mut self) {
        let rx = match &self.rx {
            Some(r) => r,
            None => return,
        };
        loop {
            match rx.try_recv() {
                Ok(ImagingMsg::Progress(u)) => {
                    self.latest = Some(u);
                }
                Ok(ImagingMsg::Done) => {
                    self.status = ImagingStatus::Complete;
                    self.rx = None;
                    break;
                }
                Ok(ImagingMsg::Error(e)) => {
                    self.status = ImagingStatus::Error(e);
                    self.rx = None;
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.rx = None;
                    break;
                }
            }
        }
    }

    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        if let Some(field) = self.edit_field {
            match code {
                KeyCode::Esc | KeyCode::Enter => {
                    self.edit_field = None;
                }
                KeyCode::Backspace => {
                    let s = self.field_mut(field);
                    s.pop();
                }
                KeyCode::Char(c) if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT => {
                    let s = self.field_mut(field);
                    s.push(c);
                }
                _ => {}
            }
            return;
        }

        match code {
            KeyCode::Char('d') => self.edit_field = Some(EditField::Dest),
            KeyCode::Char('m') => self.edit_field = Some(EditField::Mapfile),
            KeyCode::Char('s') => self.start_imaging(),
            KeyCode::Char('c') => self.cancel_imaging(),
            _ => {}
        }
    }

    fn field_mut(&mut self, field: EditField) -> &mut String {
        match field {
            EditField::Dest => &mut self.dest_path,
            EditField::Mapfile => &mut self.mapfile_path,
        }
    }

    fn start_imaging(&mut self) {
        if self.status == ImagingStatus::Running {
            return;
        }
        let device = match &self.device {
            Some(d) => Arc::clone(d),
            None => return,
        };
        if self.dest_path.is_empty() {
            self.status = ImagingStatus::Error("Set a destination path first (press d).".into());
            return;
        }

        self.cancel.store(false, Ordering::Relaxed);
        let cancel = Arc::clone(&self.cancel);
        let (tx, rx) = mpsc::sync_channel::<ImagingMsg>(64);
        self.rx = Some(rx);
        self.status = ImagingStatus::Running;
        self.latest = None;

        let config = ImagingConfig {
            output_path: PathBuf::from(&self.dest_path),
            mapfile_path: if self.mapfile_path.is_empty() {
                None
            } else {
                Some(PathBuf::from(&self.mapfile_path))
            },
            ..ImagingConfig::default()
        };

        let reporter_tx = tx.clone();
        std::thread::spawn(move || {
            let mut engine = match ImagingEngine::new(device, config) {
                Ok(e) => e,
                Err(e) => {
                    let _ = tx.send(ImagingMsg::Error(e.to_string()));
                    return;
                }
            };
            let mut reporter = ChannelReporter {
                tx: reporter_tx,
                cancel,
            };
            match engine.run(&mut reporter) {
                Ok(()) => {
                    let _ = tx.send(ImagingMsg::Done);
                }
                Err(e) => {
                    let _ = tx.send(ImagingMsg::Error(e.to_string()));
                }
            }
        });
    }

    fn cancel_imaging(&mut self) {
        if self.status == ImagingStatus::Running {
            self.cancel.store(true, Ordering::Relaxed);
            self.status = ImagingStatus::Cancelled;
        }
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        let outer = Block::default()
            .borders(Borders::ALL)
            .title(" Imaging Engine ");
        let inner = outer.inner(area);
        frame.render_widget(outer, area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(5), // config fields
                Constraint::Length(3), // progress bar
                Constraint::Min(0),    // stats / messages
            ])
            .split(inner);

        // ── Config fields ────────────────────────────────────────────────────
        let editing_dest = self.edit_field == Some(EditField::Dest);
        let editing_map = self.edit_field == Some(EditField::Mapfile);

        let dest_style = if editing_dest {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let map_style = if editing_map {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };

        let source_label = self
            .device
            .as_ref()
            .map(|d| d.device_info().path.clone())
            .unwrap_or_else(|| "—".into());

        let config_text = vec![
            Line::from(format!(" Source  : {source_label}")),
            Line::from(vec![
                Span::raw(" Dest    : "),
                Span::styled(
                    if editing_dest {
                        format!("{}█", self.dest_path)
                    } else {
                        self.dest_path.clone()
                    },
                    dest_style,
                ),
            ]),
            Line::from(vec![
                Span::raw(" Mapfile : "),
                Span::styled(
                    if editing_map {
                        format!("{}█", self.mapfile_path)
                    } else if self.mapfile_path.is_empty() {
                        "(none — progress won't be saved)".into()
                    } else {
                        self.mapfile_path.clone()
                    },
                    map_style,
                ),
            ]),
        ];
        frame.render_widget(
            Paragraph::new(config_text).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Configuration "),
            ),
            chunks[0],
        );

        // ── Progress bar ─────────────────────────────────────────────────────
        let ratio = self
            .latest
            .as_ref()
            .map(|u| u.fraction_done())
            .unwrap_or(0.0)
            .clamp(0.0, 1.0);

        let phase_label = self.latest.as_ref().map(|u| {
            use ferrite_imaging::ImagingPhase;
            match u.phase {
                ImagingPhase::Copy => "Copy",
                ImagingPhase::Trim => "Trim",
                ImagingPhase::Sweep => "Sweep",
                ImagingPhase::Scrape => "Scrape",
                ImagingPhase::Retry { attempt, max } => {
                    let _ = (attempt, max);
                    "Retry"
                }
                ImagingPhase::Complete => "Complete",
            }
        });

        let bar_label = match &self.status {
            ImagingStatus::Idle => "Not started — press s to start".into(),
            ImagingStatus::Running => {
                let phase = phase_label.unwrap_or("Copy");
                format!("{phase} — {:.1}%", ratio * 100.0)
            }
            ImagingStatus::Complete => "Complete ✓".into(),
            ImagingStatus::Cancelled => "Cancelled".into(),
            ImagingStatus::Error(e) => format!("Error: {e}"),
        };

        let bar_style = match &self.status {
            ImagingStatus::Running => Style::default().fg(Color::Green),
            ImagingStatus::Complete => Style::default().fg(Color::Green),
            ImagingStatus::Error(_) => Style::default().fg(Color::Red),
            _ => Style::default().fg(Color::DarkGray),
        };

        let gauge = Gauge::default()
            .block(Block::default().borders(Borders::ALL).title(" Progress "))
            .ratio(ratio)
            .label(bar_label)
            .gauge_style(bar_style);
        frame.render_widget(gauge, chunks[1]);

        // ── Stats ─────────────────────────────────────────────────────────────
        if let Some(u) = &self.latest {
            let elapsed = u.elapsed.as_secs();
            let stats = format!(
                " Finished: {}  Bad: {}  Non-tried: {}  Elapsed: {:02}:{:02}:{:02}",
                fmt_bytes(u.bytes_finished),
                fmt_bytes(u.bytes_bad),
                fmt_bytes(u.bytes_non_tried),
                elapsed / 3600,
                (elapsed % 3600) / 60,
                elapsed % 60,
            );
            frame.render_widget(
                Paragraph::new(stats)
                    .block(Block::default().borders(Borders::ALL).title(" Statistics ")),
                chunks[2],
            );
        } else {
            frame.render_widget(
                Paragraph::new(" Press s to start imaging, d to set destination path.")
                    .block(Block::default().borders(Borders::ALL).title(" Statistics ")),
                chunks[2],
            );
        }
    }
}

fn fmt_bytes(n: u64) -> String {
    const GIB: u64 = 1024 * 1024 * 1024;
    const MIB: u64 = 1024 * 1024;
    if n >= GIB {
        format!("{:.1}GiB", n as f64 / GIB as f64)
    } else if n >= MIB {
        format!("{:.1}MiB", n as f64 / MIB as f64)
    } else {
        format!("{n}B")
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_editing_initially_false() {
        let s = ImagingState::new();
        assert!(!s.is_editing());
    }

    #[test]
    fn d_key_enters_dest_edit_mode() {
        let mut s = ImagingState::new();
        s.handle_key(KeyCode::Char('d'), KeyModifiers::NONE);
        assert!(s.is_editing());
        assert_eq!(s.edit_field, Some(EditField::Dest));
    }

    #[test]
    fn esc_exits_edit_mode() {
        let mut s = ImagingState::new();
        s.handle_key(KeyCode::Char('d'), KeyModifiers::NONE);
        s.handle_key(KeyCode::Esc, KeyModifiers::NONE);
        assert!(!s.is_editing());
    }

    #[test]
    fn typing_appends_to_dest_path() {
        let mut s = ImagingState::new();
        s.handle_key(KeyCode::Char('d'), KeyModifiers::NONE);
        s.handle_key(KeyCode::Char('/'), KeyModifiers::NONE);
        s.handle_key(KeyCode::Char('t'), KeyModifiers::NONE);
        s.handle_key(KeyCode::Char('m'), KeyModifiers::NONE);
        s.handle_key(KeyCode::Char('p'), KeyModifiers::NONE);
        assert_eq!(s.dest_path, "/tmp");
    }
}

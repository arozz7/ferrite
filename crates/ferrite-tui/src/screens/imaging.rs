//! Screen 3 — Imaging Setup + Progress: configure and run the ddrescue-style
//! imaging engine with live progress updates.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyModifiers};
use ferrite_blockdev::BlockDevice;
use ferrite_imaging::{ImagingConfig, ImagingEngine, ProgressReporter, ProgressUpdate, Signal};
use ferrite_smart;
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
    /// Imaging finished successfully; carries the hex-encoded SHA-256 of the
    /// output image (computed immediately after the run, still in the bg thread).
    Done(Option<String>),
    Error(String),
    /// Current drive temperature from the thermal guard thread.
    Temperature(u32),
    /// Drive exceeded 55 °C — imaging paused until it cools.
    ThermalPause,
    /// Drive cooled to ≤ 50 °C — imaging resumed.
    ThermalResume,
    /// Write-blocker check result: `true` = write-blocked (safe), `false` = not blocked.
    WriteBlockerResult(bool),
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
    StartLba,
    EndLba,
    BlockSize,
}

/// `ProgressReporter` impl that forwards updates through a sync channel.
///
/// When `pause` is set (by the thermal guard thread), `report` spin-waits
/// until the flag is cleared or the user cancels.
struct ChannelReporter {
    tx: SyncSender<ImagingMsg>,
    cancel: Arc<AtomicBool>,
    /// Thermal pause flag — set by the thermal guard thread.
    pause: Arc<AtomicBool>,
}

impl ProgressReporter for ChannelReporter {
    fn report(&mut self, update: &ProgressUpdate) -> Signal {
        let _ = self.tx.try_send(ImagingMsg::Progress(update.clone()));
        // Spin-wait while thermally paused; yield to avoid busy-looping.
        while self.pause.load(Ordering::Relaxed) {
            if self.cancel.load(Ordering::Relaxed) {
                return Signal::Cancel;
            }
            std::thread::yield_now();
        }
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
    /// Start LBA (editable, empty = beginning of device).
    pub start_lba_str: String,
    /// End LBA (editable, empty = end of device).
    pub end_lba_str: String,
    /// Copy block size in KiB (editable, empty = default 512 KiB).
    pub block_size_str: String,
    edit_field: Option<EditField>,
    status: ImagingStatus,
    latest: Option<ProgressUpdate>,
    cancel: Arc<AtomicBool>,
    /// Thermal pause flag — shared with the `ChannelReporter` and thermal thread.
    pause: Arc<AtomicBool>,
    rx: Option<Receiver<ImagingMsg>>,
    /// SHA-256 hex digest of the completed image (set when imaging finishes).
    pub image_sha256: Option<String>,
    /// Most recently reported drive temperature (°C).
    pub current_temp: Option<u32>,
    /// `true` while imaging is paused due to high temperature.
    pub thermal_paused: bool,
    /// Write-blocker status: `None` = not checked, `Some(true)` = blocked (safe),
    /// `Some(false)` = WARNING: write access was granted.
    pub write_blocked: Option<bool>,
    /// When `true`, the copy pass reads from end to start.
    pub reverse: bool,
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
            start_lba_str: String::new(),
            end_lba_str: String::new(),
            block_size_str: String::new(),
            edit_field: None,
            status: ImagingStatus::Idle,
            latest: None,
            cancel: Arc::new(AtomicBool::new(false)),
            pause: Arc::new(AtomicBool::new(false)),
            rx: None,
            image_sha256: None,
            current_temp: None,
            thermal_paused: false,
            write_blocked: None,
            reverse: false,
        }
    }

    pub fn set_device(&mut self, device: Arc<dyn BlockDevice>) {
        self.device = Some(device);
        self.status = ImagingStatus::Idle;
        self.latest = None;
        self.cancel.store(false, Ordering::Relaxed);
        self.pause.store(false, Ordering::Relaxed);
        self.rx = None;
        self.current_temp = None;
        self.thermal_paused = false;
        self.write_blocked = None;
        self.start_lba_str = String::new();
        self.end_lba_str = String::new();
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
                Ok(ImagingMsg::Done(sha256)) => {
                    self.status = ImagingStatus::Complete;
                    self.image_sha256 = sha256;
                    self.thermal_paused = false;
                    self.rx = None;
                    break;
                }
                Ok(ImagingMsg::Error(e)) => {
                    self.status = ImagingStatus::Error(e);
                    self.thermal_paused = false;
                    self.rx = None;
                    break;
                }
                Ok(ImagingMsg::Temperature(t)) => {
                    self.current_temp = Some(t);
                }
                Ok(ImagingMsg::ThermalPause) => {
                    self.thermal_paused = true;
                }
                Ok(ImagingMsg::ThermalResume) => {
                    self.thermal_paused = false;
                }
                Ok(ImagingMsg::WriteBlockerResult(blocked)) => {
                    self.write_blocked = Some(blocked);
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
            KeyCode::Char('l') => self.edit_field = Some(EditField::StartLba),
            KeyCode::Char('e') => self.edit_field = Some(EditField::EndLba),
            KeyCode::Char('b') => self.edit_field = Some(EditField::BlockSize),
            KeyCode::Char('r') => self.reverse = !self.reverse,
            KeyCode::Char('s') => self.start_imaging(),
            KeyCode::Char('c') => self.cancel_imaging(),
            _ => {}
        }
    }

    fn field_mut(&mut self, field: EditField) -> &mut String {
        match field {
            EditField::Dest => &mut self.dest_path,
            EditField::Mapfile => &mut self.mapfile_path,
            EditField::StartLba => &mut self.start_lba_str,
            EditField::EndLba => &mut self.end_lba_str,
            EditField::BlockSize => &mut self.block_size_str,
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
        self.pause.store(false, Ordering::Relaxed);
        let cancel = Arc::clone(&self.cancel);
        let pause = Arc::clone(&self.pause);
        let (tx, rx) = mpsc::sync_channel::<ImagingMsg>(64);
        self.rx = Some(rx);
        self.status = ImagingStatus::Running;
        self.latest = None;
        self.current_temp = None;
        self.thermal_paused = false;
        self.write_blocked = None;

        let output_path = PathBuf::from(&self.dest_path);
        let copy_block_size = self
            .block_size_str
            .trim()
            .parse::<u64>()
            .ok()
            .filter(|&n| n > 0)
            .map(|kb| kb * 1024) // field is in KiB
            .unwrap_or(512 * 1024); // default 512 KiB
        let config = ImagingConfig {
            output_path: output_path.clone(),
            copy_block_size,
            mapfile_path: if self.mapfile_path.is_empty() {
                None
            } else {
                Some(PathBuf::from(&self.mapfile_path))
            },
            start_lba: self.start_lba_str.trim().parse::<u64>().ok(),
            end_lba: self.end_lba_str.trim().parse::<u64>().ok(),
            reverse: self.reverse,
            ..ImagingConfig::default()
        };

        // ── Thermal guard thread ─────────────────────────────────────────────
        // Polls S.M.A.R.T. every 60 seconds.  Pauses imaging above 55 °C and
        // resumes after the drive cools to ≤ 50 °C.
        let device_path_for_smart = device.device_info().path.clone();
        let device_path = device_path_for_smart.clone();
        let thermal_tx = tx.clone();
        let thermal_cancel = Arc::clone(&cancel);
        let thermal_pause = Arc::clone(&pause);
        std::thread::spawn(move || {
            loop {
                if thermal_cancel.load(Ordering::Relaxed) {
                    break;
                }
                if let Ok(data) = ferrite_smart::query(&device_path, None) {
                    if let Some(temp) = data.temperature_celsius {
                        let _ = thermal_tx.try_send(ImagingMsg::Temperature(temp));
                        if temp >= 55 && !thermal_pause.load(Ordering::Relaxed) {
                            thermal_pause.store(true, Ordering::Relaxed);
                            let _ = thermal_tx.try_send(ImagingMsg::ThermalPause);
                        } else if temp <= 50 && thermal_pause.load(Ordering::Relaxed) {
                            thermal_pause.store(false, Ordering::Relaxed);
                            let _ = thermal_tx.try_send(ImagingMsg::ThermalResume);
                        }
                    }
                }
                // Check cancel every second for 60 seconds between polls.
                for _ in 0..60 {
                    std::thread::sleep(std::time::Duration::from_secs(1));
                    if thermal_cancel.load(Ordering::Relaxed) {
                        return;
                    }
                }
            }
        });

        let reporter_tx = tx.clone();
        std::thread::spawn(move || {
            let mut engine = match ImagingEngine::new(device, config) {
                Ok(e) => e,
                Err(e) => {
                    let _ = tx.send(ImagingMsg::Error(e.to_string()));
                    return;
                }
            };
            // Write-blocker check: attempt to open the source device for writing.
            // If the open succeeds, write-blocking is NOT active (suspicious).
            {
                let write_blocked = std::fs::OpenOptions::new()
                    .write(true)
                    .open(&device_path_for_smart)
                    .map(|_| false) // opened for write → not blocked
                    .unwrap_or(true); // error opening for write → blocked (or denied)
                let _ = tx.send(ImagingMsg::WriteBlockerResult(write_blocked));
            }

            // Pre-populate known-bad sectors from S.M.A.R.T. error log (best-effort).
            if let Ok(smart_data) = ferrite_smart::query(&device_path_for_smart, None) {
                if !smart_data.bad_sector_lbas.is_empty() {
                    let ss = engine.sector_size();
                    engine.pre_populate_bad_sectors(ss as u64, &smart_data.bad_sector_lbas);
                }
            }
            let mut reporter = ChannelReporter {
                tx: reporter_tx,
                cancel,
                pause,
            };
            match engine.run(&mut reporter) {
                Ok(()) => {
                    let sha256 = compute_sha256(&output_path);
                    let _ = tx.send(ImagingMsg::Done(sha256));
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
                Constraint::Length(11), // config fields + hint
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
        } else if self.dest_path.is_empty() {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::Green)
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
                    } else if self.dest_path.is_empty() {
                        "(not set — press d)  e.g. D:\\recovery\\disk.img".into()
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
            Line::from(vec![
                Span::raw(" Start   : "),
                Span::styled(
                    if self.edit_field == Some(EditField::StartLba) {
                        format!("{}█", self.start_lba_str)
                    } else if self.start_lba_str.is_empty() {
                        "(beginning)".into()
                    } else {
                        format!("LBA {}", self.start_lba_str)
                    },
                    if self.edit_field == Some(EditField::StartLba) {
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    },
                ),
            ]),
            Line::from(vec![
                Span::raw(" End     : "),
                Span::styled(
                    if self.edit_field == Some(EditField::EndLba) {
                        format!("{}█", self.end_lba_str)
                    } else if self.end_lba_str.is_empty() {
                        "(end of device)".into()
                    } else {
                        format!("LBA {}", self.end_lba_str)
                    },
                    if self.edit_field == Some(EditField::EndLba) {
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    },
                ),
            ]),
            Line::from(vec![
                Span::raw(" BlockSz : "),
                Span::styled(
                    if self.edit_field == Some(EditField::BlockSize) {
                        format!("{}█ KiB", self.block_size_str)
                    } else if self.block_size_str.is_empty() {
                        "(default 512 KiB)".into()
                    } else {
                        format!("{} KiB", self.block_size_str)
                    },
                    if self.edit_field == Some(EditField::BlockSize) {
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    },
                ),
            ]),
            Line::from(vec![
                Span::raw(" Reverse : "),
                Span::styled(
                    if self.reverse { "YES" } else { "NO" }.to_string(),
                    if self.reverse {
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    },
                ),
                Span::raw("  (r to toggle)"),
            ]),
            Line::from(Span::styled(
                " Dest is the output image file path, e.g. D:\\recovery\\disk.img  \
                 Mapfile saves progress so imaging can resume after interruption.",
                Style::default().fg(Color::DarkGray),
            )),
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
        // ── Write-blocker status line ─────────────────────────────────────────
        let wb_line: Option<ratatui::text::Line> =
            if self.status == ImagingStatus::Running || self.write_blocked.is_some() {
                match self.write_blocked {
                    None => Some(ratatui::text::Line::from(Span::styled(
                        " Write-blocker: checking…",
                        Style::default().fg(Color::DarkGray),
                    ))),
                    Some(true) => Some(ratatui::text::Line::from(Span::styled(
                        " Write-blocker: OK",
                        Style::default().fg(Color::Green),
                    ))),
                    Some(false) => Some(ratatui::text::Line::from(Span::styled(
                        " Write-blocker: WARNING — not blocked!",
                        Style::default().fg(Color::Red),
                    ))),
                }
            } else {
                None
            };

        if let Some(u) = &self.latest {
            let elapsed = u.elapsed.as_secs();
            let rate_mbps = u.read_rate_bps as f64 / (1024.0 * 1024.0);
            let rate_str = if u.read_rate_bps == 0 {
                " Rate: —".to_string()
            } else if rate_mbps < 5.0 {
                format!(" Rate: {rate_mbps:.1} MB/s ⚠ SLOW")
            } else {
                format!(" Rate: {rate_mbps:.1} MB/s")
            };

            let temp_str = match (self.current_temp, self.thermal_paused) {
                (Some(t), true) => format!("  Temp: {t}°C ⚠ PAUSED (>55°C)"),
                (Some(t), false) => format!("  Temp: {t}°C"),
                (None, _) => String::new(),
            };

            let mut stats = format!(
                " Finished: {}  Bad: {}  Non-tried: {}  Elapsed: {:02}:{:02}:{:02}\n{}{}",
                fmt_bytes(u.bytes_finished),
                fmt_bytes(u.bytes_bad),
                fmt_bytes(u.bytes_non_tried),
                elapsed / 3600,
                (elapsed % 3600) / 60,
                elapsed % 60,
                rate_str,
                temp_str,
            );
            if let Some(hash) = &self.image_sha256 {
                stats.push_str(&format!("\n SHA-256: {hash}"));
            }
            if let Some(wbl) = wb_line {
                use ratatui::text::Text;
                let mut text = Text::from(stats);
                text.push_line(wbl);
                frame.render_widget(
                    Paragraph::new(text)
                        .block(Block::default().borders(Borders::ALL).title(" Statistics ")),
                    chunks[2],
                );
            } else {
                frame.render_widget(
                    Paragraph::new(stats)
                        .block(Block::default().borders(Borders::ALL).title(" Statistics ")),
                    chunks[2],
                );
            }
        } else {
            let base_msg = " Press s to start imaging, d to set destination path.";
            if let Some(wbl) = wb_line {
                use ratatui::text::Text;
                let mut text = Text::from(base_msg);
                text.push_line(wbl);
                frame.render_widget(
                    Paragraph::new(text)
                        .block(Block::default().borders(Borders::ALL).title(" Statistics ")),
                    chunks[2],
                );
            } else {
                frame.render_widget(
                    Paragraph::new(base_msg)
                        .block(Block::default().borders(Borders::ALL).title(" Statistics ")),
                    chunks[2],
                );
            }
        }
    }
}

/// Compute the SHA-256 digest of the file at `path` and return it as a
/// lowercase hex string.  Returns `None` on any I/O error.
fn compute_sha256(path: &std::path::Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65_536];
    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Some(format!("{:x}", hasher.finalize()))
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
    fn image_sha256_initially_none() {
        let s = ImagingState::new();
        assert!(s.image_sha256.is_none());
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

    #[test]
    fn write_blocker_result_message_sets_state() {
        let (tx, rx) = mpsc::sync_channel::<ImagingMsg>(8);
        let mut s = ImagingState::new();
        s.rx = Some(rx);
        tx.send(ImagingMsg::WriteBlockerResult(true)).unwrap();
        s.tick();
        assert_eq!(s.write_blocked, Some(true));
    }
}

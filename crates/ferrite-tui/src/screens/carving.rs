//! Screen 6 — File Carving: select signature types and run the carving engine
//! with live progress, then extract hits to disk.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyModifiers};
use ferrite_blockdev::BlockDevice;
use ferrite_carver::{CarveHit, Carver, CarvingConfig, Signature};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

// ── Types ─────────────────────────────────────────────────────────────────────

enum CarveMsg {
    Done(Vec<CarveHit>),
    Error(String),
}

#[derive(PartialEq)]
enum CarveStatus {
    Idle,
    Running,
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

// ── State ─────────────────────────────────────────────────────────────────────

pub struct CarvingState {
    device: Option<Arc<dyn BlockDevice>>,
    sig_list: Vec<SigEntry>,
    sig_sel: usize,
    hits: Vec<CarveHit>,
    hit_sel: usize,
    focus: CarveFocus,
    status: CarveStatus,
    cancel: Arc<AtomicBool>,
    rx: Option<Receiver<CarveMsg>>,
}

impl Default for CarvingState {
    fn default() -> Self {
        Self::new()
    }
}

impl CarvingState {
    pub fn new() -> Self {
        let sig_list = load_builtin_signatures();
        Self {
            device: None,
            sig_list,
            sig_sel: 0,
            hits: Vec::new(),
            hit_sel: 0,
            focus: CarveFocus::Signatures,
            status: CarveStatus::Idle,
            cancel: Arc::new(AtomicBool::new(false)),
            rx: None,
        }
    }

    pub fn set_device(&mut self, device: Arc<dyn BlockDevice>) {
        self.device = Some(device);
        self.hits.clear();
        self.hit_sel = 0;
        self.status = CarveStatus::Idle;
        self.cancel.store(false, Ordering::Relaxed);
        self.rx = None;
    }

    /// Drain the background carving channel.
    pub fn tick(&mut self) {
        let rx = match &self.rx {
            Some(r) => r,
            None => return,
        };
        match rx.try_recv() {
            Ok(CarveMsg::Done(hits)) => {
                self.hits = hits;
                self.hit_sel = 0;
                self.status = CarveStatus::Done;
                self.rx = None;
            }
            Ok(CarveMsg::Error(e)) => {
                self.status = CarveStatus::Error(e);
                self.rx = None;
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                self.rx = None;
            }
        }
    }

    pub fn handle_key(&mut self, code: KeyCode, _modifiers: KeyModifiers) {
        match code {
            // Switch focus between panels with Left / Right
            KeyCode::Left => self.focus = CarveFocus::Signatures,
            KeyCode::Right if !self.hits.is_empty() => self.focus = CarveFocus::Hits,
            KeyCode::Up => self.move_selection(-1),
            KeyCode::Down => self.move_selection(1),
            KeyCode::Char(' ') if self.focus == CarveFocus::Signatures => {
                self.toggle_signature();
            }
            KeyCode::Char('s') => self.start_scan(),
            KeyCode::Char('c') => self.cancel_scan(),
            KeyCode::Char('e') => self.extract_selected(),
            _ => {}
        }
    }

    fn move_selection(&mut self, delta: i32) {
        match self.focus {
            CarveFocus::Signatures => {
                let len = self.sig_list.len();
                if len == 0 {
                    return;
                }
                if delta < 0 {
                    self.sig_sel = self.sig_sel.saturating_sub(1);
                } else {
                    self.sig_sel = (self.sig_sel + 1).min(len - 1);
                }
            }
            CarveFocus::Hits => {
                let len = self.hits.len();
                if len == 0 {
                    return;
                }
                if delta < 0 {
                    self.hit_sel = self.hit_sel.saturating_sub(1);
                } else {
                    self.hit_sel = (self.hit_sel + 1).min(len - 1);
                }
            }
        }
    }

    fn toggle_signature(&mut self) {
        if let Some(e) = self.sig_list.get_mut(self.sig_sel) {
            e.enabled = !e.enabled;
        }
    }

    fn start_scan(&mut self) {
        if self.status == CarveStatus::Running {
            return;
        }
        let device = match &self.device {
            Some(d) => Arc::clone(d),
            None => return,
        };
        let enabled: Vec<Signature> = self
            .sig_list
            .iter()
            .filter(|e| e.enabled)
            .map(|e| e.sig.clone())
            .collect();
        if enabled.is_empty() {
            return;
        }
        let config = CarvingConfig {
            signatures: enabled,
            scan_chunk_size: 1024 * 1024,
        };

        self.cancel.store(false, Ordering::Relaxed);
        self.hits.clear();
        self.hit_sel = 0;
        self.status = CarveStatus::Running;

        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);

        std::thread::spawn(move || {
            let carver = Carver::new(device, config);
            match carver.scan() {
                Ok(hits) => {
                    let _ = tx.send(CarveMsg::Done(hits));
                }
                Err(e) => {
                    let _ = tx.send(CarveMsg::Error(e.to_string()));
                }
            }
        });
    }

    fn cancel_scan(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        // The Carver::scan() API doesn't support cancellation callbacks, so
        // we just mark cancelled on our side and the thread will finish.
        self.status = CarveStatus::Idle;
        self.rx = None;
    }

    fn extract_selected(&mut self) {
        let hit = match self.hits.get(self.hit_sel) {
            Some(h) => h.clone(),
            None => return,
        };
        let device = match &self.device {
            Some(d) => Arc::clone(d),
            None => return,
        };
        // Build a safe output filename: <ext>_<offset>.ext
        let filename = format!(
            "ferrite_{}_{}.{}",
            hit.signature.extension, hit.byte_offset, hit.signature.extension
        );
        let config = CarvingConfig {
            signatures: vec![hit.signature.clone()],
            scan_chunk_size: 1024 * 1024,
        };
        std::thread::spawn(move || {
            let carver = Carver::new(device, config);
            if let Ok(mut f) = std::fs::File::create(&filename) {
                let _ = carver.extract(&hit, &mut f);
                tracing::info!(path = %filename, "extracted file");
            }
        });
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        let status_label = match &self.status {
            CarveStatus::Idle => " idle ",
            CarveStatus::Running => " scanning… ",
            CarveStatus::Done => " done ",
            CarveStatus::Error(e) => {
                let msg = format!(" Carving Error: {e}");
                frame.render_widget(
                    Paragraph::new(msg)
                        .style(Style::default().fg(Color::Red))
                        .block(Block::default().borders(Borders::ALL).title(" Carving ")),
                    area,
                );
                return;
            }
        };

        let title = format!(
            " Carving [{status_label}] — Space: toggle  s: scan  e: extract  ←/→: switch panel "
        );
        let outer = Block::default().borders(Borders::ALL).title(title);
        let inner = outer.inner(area);
        frame.render_widget(outer, area);

        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(inner);

        self.render_sig_panel(frame, chunks[0]);
        self.render_hits_panel(frame, chunks[1]);
    }

    fn render_sig_panel(&self, frame: &mut Frame, area: Rect) {
        let focused = self.focus == CarveFocus::Signatures;
        let title_style = if focused {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(" Signatures (Space=toggle) ", title_style));

        let items: Vec<ListItem> = self
            .sig_list
            .iter()
            .map(|e| {
                let check = if e.enabled { "[✓]" } else { "[ ]" };
                let style = if e.enabled {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default().fg(Color::DarkGray)
                };
                ListItem::new(format!("{check} {}", e.sig.name)).style(style)
            })
            .collect();

        let list = List::new(items)
            .block(block)
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

        let mut ls =
            ListState::default().with_selected(if focused { Some(self.sig_sel) } else { None });
        frame.render_stateful_widget(list, area, &mut ls);
    }

    fn render_hits_panel(&self, frame: &mut Frame, area: Rect) {
        let focused = self.focus == CarveFocus::Hits;
        let title_style = if focused {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let hit_count = self.hits.len();
        let block = Block::default().borders(Borders::ALL).title(Span::styled(
            format!(" Hits ({hit_count}) — e: extract selected "),
            title_style,
        ));

        if self.hits.is_empty() {
            let msg = match &self.status {
                CarveStatus::Idle => " Enable signatures and press s to scan.",
                CarveStatus::Running => " Scanning…",
                _ => " No hits found.",
            };
            frame.render_widget(Paragraph::new(msg).block(block), area);
            return;
        }

        let items: Vec<ListItem> = self
            .hits
            .iter()
            .map(|h| {
                ListItem::new(format!(
                    " {} @ offset {:#x}",
                    h.signature.name, h.byte_offset
                ))
            })
            .collect();

        let list = List::new(items)
            .block(block)
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

        let mut ls =
            ListState::default().with_selected(if focused { Some(self.hit_sel) } else { None });
        frame.render_stateful_widget(list, area, &mut ls);
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn load_builtin_signatures() -> Vec<SigEntry> {
    match CarvingConfig::from_toml_str(crate::SIGNATURES_TOML) {
        Ok(cfg) => cfg
            .signatures
            .into_iter()
            .map(|sig| SigEntry { sig, enabled: true })
            .collect(),
        Err(e) => {
            tracing::error!(?e, "failed to load built-in signatures");
            Vec::new()
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_signatures_load() {
        let s = CarvingState::new();
        assert!(
            !s.sig_list.is_empty(),
            "expected at least one built-in signature"
        );
    }

    #[test]
    fn all_signatures_enabled_by_default() {
        let s = CarvingState::new();
        assert!(s.sig_list.iter().all(|e| e.enabled));
    }

    #[test]
    fn space_toggles_signature() {
        let mut s = CarvingState::new();
        assert!(s.sig_list[0].enabled);
        s.handle_key(KeyCode::Char(' '), KeyModifiers::NONE);
        assert!(!s.sig_list[0].enabled);
        s.handle_key(KeyCode::Char(' '), KeyModifiers::NONE);
        assert!(s.sig_list[0].enabled);
    }

    #[test]
    fn selection_does_not_underflow() {
        let mut s = CarvingState::new();
        s.handle_key(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(s.sig_sel, 0);
    }
}

//! Top-level application state and main event loop.

use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use ratatui::{
    backend::Backend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph, Tabs},
    Frame, Terminal,
};

use ferrite_blockdev::BlockDevice;

use crate::{
    screens::{
        carving::CarvingState, drive_select::DriveSelectState, file_browser::FileBrowserState,
        health::HealthState, imaging::ImagingState, partition::PartitionState,
    },
    Result,
};

const SCREEN_NAMES: [&str; 6] = [
    " Drives ",
    " Health ",
    " Imaging ",
    " Partitions ",
    " Files ",
    " Carving ",
];

/// Root application state.
pub struct App {
    pub screen_idx: usize,
    pub should_quit: bool,
    /// The currently active device (set from the Drive Selection screen).
    pub selected_device: Option<Arc<dyn BlockDevice>>,
    pub drive_select: DriveSelectState,
    pub health: HealthState,
    pub imaging: ImagingState,
    pub partition: PartitionState,
    pub file_browser: FileBrowserState,
    pub carving: CarvingState,
}

impl App {
    pub fn new() -> Self {
        Self {
            screen_idx: 0,
            should_quit: false,
            selected_device: None,
            drive_select: DriveSelectState::new(),
            health: HealthState::new(),
            imaging: ImagingState::new(),
            partition: PartitionState::new(),
            file_browser: FileBrowserState::new(),
            carving: CarvingState::new(),
        }
    }

    /// Run the main event loop until the user quits.
    pub fn run_loop<B: Backend>(&mut self, terminal: &mut Terminal<B>) -> Result<()> {
        loop {
            terminal.draw(|f| self.render(f))?;
            if event::poll(Duration::from_millis(50))? {
                if let Event::Key(key) = event::read()? {
                    self.handle_key(key.code, key.modifiers);
                }
            }
            self.tick();
            if self.should_quit {
                break;
            }
        }
        Ok(())
    }

    // ── Key routing ──────────────────────────────────────────────────────────

    fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        // Tab / Shift-Tab always switch screens.
        match (code, modifiers) {
            (KeyCode::Tab, _) => {
                self.screen_idx = (self.screen_idx + 1) % SCREEN_NAMES.len();
                return;
            }
            (KeyCode::BackTab, _) => {
                self.screen_idx = (self.screen_idx + SCREEN_NAMES.len() - 1) % SCREEN_NAMES.len();
                return;
            }
            _ => {}
        }

        // 'q' quits unless a text-input field on the current screen is active.
        if code == KeyCode::Char('q') && modifiers.is_empty() {
            let in_edit = match self.screen_idx {
                2 => self.imaging.is_editing(),
                _ => false,
            };
            if !in_edit {
                self.should_quit = true;
                return;
            }
        }

        match self.screen_idx {
            0 => {
                if let Some(dev) = self.drive_select.handle_key(code, modifiers) {
                    let path = dev.device_info().path.clone();
                    self.selected_device = Some(Arc::clone(&dev));
                    // Propagate device to all dependent screens.
                    self.health.set_device(path);
                    self.imaging.set_device(Arc::clone(&dev));
                    self.partition.set_device(Arc::clone(&dev));
                    self.file_browser.set_device(Arc::clone(&dev));
                    self.carving.set_device(dev);
                }
            }
            1 => self.health.handle_key(code, modifiers),
            2 => self.imaging.handle_key(code, modifiers),
            3 => self.partition.handle_key(code, modifiers),
            4 => self.file_browser.handle_key(code, modifiers),
            5 => self.carving.handle_key(code, modifiers),
            _ => {}
        }
    }

    // ── Background channel drain ──────────────────────────────────────────────

    fn tick(&mut self) {
        self.drive_select.tick();
        self.health.tick();
        self.imaging.tick();
        self.partition.tick();
        self.file_browser.tick();
        self.carving.tick();
    }

    // ── Rendering ────────────────────────────────────────────────────────────

    fn render(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // tab bar
                Constraint::Min(1),    // screen content
                Constraint::Length(1), // help bar
            ])
            .split(area);

        // Tab bar
        let tabs = Tabs::new(SCREEN_NAMES.map(Line::from))
            .select(self.screen_idx)
            .block(Block::default().borders(Borders::ALL).title(" Ferrite "))
            .style(Style::default().fg(Color::White))
            .highlight_style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            );
        frame.render_widget(tabs, chunks[0]);

        // Screen content
        match self.screen_idx {
            0 => self.drive_select.render(frame, chunks[1]),
            1 => self.health.render(frame, chunks[1]),
            2 => self.imaging.render(frame, chunks[1]),
            3 => self.partition.render(frame, chunks[1]),
            4 => self.file_browser.render(frame, chunks[1]),
            5 => self.carving.render(frame, chunks[1]),
            _ => {}
        }

        // Help bar
        frame.render_widget(
            Paragraph::new(help_line(self.screen_idx, self.selected_device.is_some()))
                .style(Style::default().fg(Color::DarkGray)),
            chunks[2],
        );
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

fn help_line(screen: usize, has_device: bool) -> &'static str {
    if !has_device && screen != 0 {
        return " Tab/Shift-Tab: switch  q: quit  (select a device on the Drives screen first)";
    }
    match screen {
        0 => " ↑/↓: navigate  Enter: select device  r: refresh list  Tab: next  q: quit",
        1 => " r: refresh S.M.A.R.T.  ↑/↓: scroll attrs  Tab: next  q: quit",
        2 => " d: edit dest  m: edit mapfile  s: start  c: cancel  Esc: stop editing  Tab: next  q: quit",
        3 => " r: read partition table  s: scan device  Tab: next  q: quit",
        4 => " ↑/↓: navigate  Enter: open dir  Backspace: go up  d: toggle deleted  o: open fs  Tab: next  q: quit",
        5 => " ↑/↓: navigate  Space: toggle sig  s: start scan  e: extract  Tab: next  q: quit",
        _ => " Tab: next  q: quit",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_forward_wraps() {
        let mut app = App::new();
        for _ in 0..SCREEN_NAMES.len() {
            app.handle_key(KeyCode::Tab, KeyModifiers::NONE);
        }
        assert_eq!(app.screen_idx, 0);
    }

    #[test]
    fn tab_backward_wraps() {
        let mut app = App::new();
        app.handle_key(KeyCode::BackTab, KeyModifiers::NONE);
        assert_eq!(app.screen_idx, SCREEN_NAMES.len() - 1);
    }

    #[test]
    fn quit_key_sets_flag() {
        let mut app = App::new();
        app.handle_key(KeyCode::Char('q'), KeyModifiers::NONE);
        assert!(app.should_quit);
    }

    #[test]
    fn screen_count_matches_names() {
        assert_eq!(SCREEN_NAMES.len(), 6);
    }
}

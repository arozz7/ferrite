//! Top-level application state and main event loop.

use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    backend::Backend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph, Tabs},
    Frame, Terminal,
};

use ferrite_blockdev::BlockDevice;
#[cfg(target_os = "windows")]
use ferrite_blockdev::{parse_disk_number, VolumeGuard};

use crate::session::Session;
use crate::{
    screens::{
        artifacts::ArtifactsState,
        carving::CarvingState,
        drive_select::DriveSelectState,
        file_browser::FileBrowserState,
        health::HealthState,
        hex_viewer::HexViewerState,
        imaging::ImagingState,
        partition::PartitionState,
        quick_recover::QuickRecoverState,
        report::generate_report,
        session_manager::{SessionManagerState, SessionMsg},
        text_scan::TextScanState,
    },
    Result,
};

const SCREEN_NAMES: [&str; 10] = [
    " Drives ",
    " Health ",
    " Imaging ",
    " Partitions ",
    " Files ",
    " Carving ",
    " Hex ",
    " Quick Recover ",
    " Artifacts ",
    " Text Scan ",
];

/// Root application state.
pub struct App {
    pub screen_idx: usize,
    pub should_quit: bool,
    /// The currently active device (set from the Drive Selection screen).
    pub selected_device: Option<Arc<dyn BlockDevice>>,
    /// Holds volumes offline for the currently selected physical drive.
    /// Dropped automatically when the device changes or Ferrite exits,
    /// which re-onlines all volumes via the RAII `Drop` impl.
    #[cfg(target_os = "windows")]
    volume_guard: Option<VolumeGuard>,
    pub drive_select: DriveSelectState,
    pub health: HealthState,
    pub imaging: ImagingState,
    pub partition: PartitionState,
    pub file_browser: FileBrowserState,
    pub carving: CarvingState,
    pub hex_viewer: HexViewerState,
    pub quick_recover: QuickRecoverState,
    pub artifacts: ArtifactsState,
    pub text_scan: TextScanState,
    /// Saved-session manager overlay.
    pub session_manager: SessionManagerState,
    /// Status message from the last report generation.
    pub report_status: Option<String>,
}

impl App {
    pub fn new() -> Self {
        let session = Session::load();
        let mut app = Self {
            screen_idx: 0,
            should_quit: false,
            selected_device: None,
            #[cfg(target_os = "windows")]
            volume_guard: None,
            drive_select: DriveSelectState::new(),
            health: HealthState::new(),
            imaging: ImagingState::new(),
            partition: PartitionState::new(),
            file_browser: FileBrowserState::new(),
            carving: CarvingState::new(),
            hex_viewer: HexViewerState::new(),
            quick_recover: QuickRecoverState::new(),
            artifacts: ArtifactsState::new(),
            text_scan: TextScanState::new(),
            session_manager: SessionManagerState::default(),
            report_status: None,
        };
        app.imaging.dest_path = session.imaging_dest;
        app.imaging.mapfile_path = session.imaging_mapfile;
        app.imaging.start_lba_str = session.imaging_start_lba;
        app.imaging.end_lba_str = session.imaging_end_lba;
        app.imaging.reverse = session.imaging_reverse;
        if !session.carving_output_dir.is_empty() {
            app.carving.output_dir = session.carving_output_dir;
        }
        app.carving.scan_start_lba_str = session.carving_scan_start_lba;
        app.carving.scan_end_lba_str = session.carving_scan_end_lba;
        app.hex_viewer.current_lba = session.hex_last_lba;
        app
    }

    /// Run the main event loop until the user quits.
    pub fn run_loop<B: Backend>(&mut self, terminal: &mut Terminal<B>) -> Result<()> {
        loop {
            terminal.draw(|f| self.render(f))?;
            if event::poll(Duration::from_millis(50))? {
                match event::read()? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        self.handle_key(key.code, key.modifiers);
                    }
                    Event::Paste(text) => {
                        self.handle_paste(text);
                    }
                    _ => {}
                }
            }
            self.tick();
            if self.should_quit {
                break;
            }
        }
        // Save carving session if there are hits or a checkpoint.
        if let Some(dev) = &self.selected_device {
            let info = dev.device_info().clone();
            if self.carving.has_hits() || self.carving.checkpoint_path().is_some() {
                let session = self.carving.build_session(&info);
                let _ = session.save();
            }
        }
        Session {
            imaging_dest: self.imaging.dest_path.clone(),
            imaging_mapfile: self.imaging.mapfile_path.clone(),
            imaging_start_lba: self.imaging.start_lba_str.clone(),
            imaging_end_lba: self.imaging.end_lba_str.clone(),
            imaging_reverse: self.imaging.reverse,
            carving_output_dir: self.carving.output_dir.clone(),
            carving_scan_start_lba: self.carving.scan_start_lba_str.clone(),
            carving_scan_end_lba: self.carving.scan_end_lba_str.clone(),
            hex_last_lba: self.hex_viewer.current_lba,
        }
        .save();
        Ok(())
    }

    // ── Key routing ──────────────────────────────────────────────────────────

    fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        // Session manager overlay gets first priority when visible.
        if self.session_manager.visible {
            if let Some(msg) = self.session_manager.handle_key(code, modifiers) {
                match msg {
                    SessionMsg::Resume { session, device } => {
                        let path = device.device_info().path.clone();
                        self.selected_device = Some(Arc::clone(&device));
                        // Take volumes offline to stop Windows background I/O.
                        self.quiesce_volumes(&path);
                        self.health.set_device(path);
                        self.imaging.set_device(Arc::clone(&device));
                        self.partition.set_device(Arc::clone(&device));
                        self.file_browser.set_device(Arc::clone(&device));
                        self.carving.set_device(Arc::clone(&device));
                        self.carving.restore_from_session(&session);
                        self.quick_recover.set_device(Arc::clone(&device));
                        self.artifacts.set_device(Arc::clone(&device));
                        self.text_scan.set_device(Arc::clone(&device));
                        self.hex_viewer.set_device(device);
                        self.screen_idx = 5; // go to carving screen
                        self.on_screen_enter();
                    }
                    SessionMsg::Dismissed => {}
                }
            }
            return;
        }

        // When a text overlay owns the keyboard (e.g. image-open overlay on the
        // Drives screen), suppress all global shortcuts so typed characters don't
        // trigger actions.
        let overlay_active = self.screen_idx == 0 && self.drive_select.image_overlay_active();

        // Tab / Shift-Tab switch screens — suppressed when overlay is active.
        if !overlay_active {
            match (code, modifiers) {
                (KeyCode::Tab, _) => {
                    self.screen_idx = (self.screen_idx + 1) % SCREEN_NAMES.len();
                    self.on_screen_enter();
                    return;
                }
                (KeyCode::BackTab, _) => {
                    self.screen_idx =
                        (self.screen_idx + SCREEN_NAMES.len() - 1) % SCREEN_NAMES.len();
                    self.on_screen_enter();
                    return;
                }
                _ => {}
            }
        }

        // Shift+R generates a recovery report — suppressed when overlay is active.
        if !overlay_active
            && code == KeyCode::Char('R')
            && (modifiers.is_empty() || modifiers == KeyModifiers::SHIFT)
        {
            self.generate_report_to_file();
            return;
        }

        // 'q' quits unless a text-input field on the current screen is active.
        if code == KeyCode::Char('q') && modifiers.is_empty() {
            let in_edit = match self.screen_idx {
                0 => self.drive_select.is_filtering(),
                2 => self.imaging.is_editing(),
                5 => self.carving.is_editing(),
                6 => self.hex_viewer.is_editing(),
                7 => self.quick_recover.is_editing(),
                8 => self.artifacts.is_editing(),
                9 => self.text_scan.is_editing(),
                _ => false,
            };
            if !in_edit {
                self.should_quit = true;
                return;
            }
        }

        match self.screen_idx {
            0 => {
                // 'o' opens the saved session manager overlay — suppressed when
                // the image-open overlay is active (user may be typing a path
                // that contains the letter 'o').
                if !overlay_active && code == KeyCode::Char('o') && modifiers.is_empty() {
                    if crate::carving_session::CarvingSession::load_all().is_empty() {
                        self.report_status = Some(
                            "No saved sessions — sessions are created automatically when you carve a drive and quit.".into(),
                        );
                    } else {
                        self.session_manager.open();
                    }
                    return;
                }
                if let Some(dev) = self.drive_select.handle_key(code, modifiers) {
                    let path = dev.device_info().path.clone();
                    self.selected_device = Some(Arc::clone(&dev));
                    // Take volumes offline to stop Windows background I/O.
                    self.quiesce_volumes(&path);
                    // Propagate device to all dependent screens.
                    self.health.set_device(path);
                    self.imaging.set_device(Arc::clone(&dev));
                    self.partition.set_device(Arc::clone(&dev));
                    self.file_browser.set_device(Arc::clone(&dev));
                    self.carving.set_device(Arc::clone(&dev));
                    self.quick_recover.set_device(Arc::clone(&dev));
                    self.artifacts.set_device(Arc::clone(&dev));
                    self.text_scan.set_device(Arc::clone(&dev));
                    self.hex_viewer.set_device(dev);
                    // Auto-advance to Health so the user sees S.M.A.R.T. results immediately.
                    self.screen_idx = 1;
                    self.on_screen_enter();
                }
            }
            1 => self.health.handle_key(code, modifiers),
            2 => self.imaging.handle_key(code, modifiers),
            3 => self.partition.handle_key(code, modifiers),
            4 => self.file_browser.handle_key(code, modifiers),
            5 => {
                // 'h' on a selected hit deep-links into the hex viewer.
                if code == KeyCode::Char('h') && modifiers.is_empty() {
                    if let Some(offset) = self.carving.selected_hit_offset() {
                        self.hex_viewer.jump_to_byte_offset(offset);
                        self.screen_idx = 6;
                        return;
                    }
                }
                self.carving.handle_key(code, modifiers);
            }
            6 => self.hex_viewer.handle_key(code, modifiers),
            7 => self.quick_recover.handle_key(code, modifiers),
            8 => self.artifacts.handle_key(code, modifiers),
            9 => self.text_scan.handle_key(code, modifiers),
            _ => {}
        }
    }

    fn handle_paste(&mut self, text: String) {
        // Route paste to the Drives screen if it has an active text field.
        if self.screen_idx == 0 {
            if let Some(dev) = self.drive_select.handle_paste(&text) {
                let path = dev.device_info().path.clone();
                self.selected_device = Some(Arc::clone(&dev));
                self.quiesce_volumes(&path);
                self.health.set_device(path);
                self.imaging.set_device(Arc::clone(&dev));
                self.partition.set_device(Arc::clone(&dev));
                self.file_browser.set_device(Arc::clone(&dev));
                self.carving.set_device(Arc::clone(&dev));
                self.quick_recover.set_device(Arc::clone(&dev));
                self.artifacts.set_device(Arc::clone(&dev));
                self.text_scan.set_device(Arc::clone(&dev));
                self.hex_viewer.set_device(dev);
                self.screen_idx = 1;
                self.on_screen_enter();
            }
        }
    }

    fn generate_report_to_file(&mut self) {
        let device_info = match &self.selected_device {
            Some(d) => d.device_info().clone(),
            None => {
                self.report_status = Some("No device selected for report.".into());
                return;
            }
        };
        let smart_ref = self.health.last_smart_data.as_ref();
        let partition_ref = self.partition.table.as_ref();
        let carve_count = self.carving.hits_count();

        let report = generate_report(
            &device_info,
            smart_ref,
            &self.imaging.dest_path,
            &self.imaging.mapfile_path,
            partition_ref,
            carve_count,
        );

        match std::fs::write("ferrite-report.txt", &report) {
            Ok(_) => self.report_status = Some("Report saved to ferrite-report.txt".into()),
            Err(e) => self.report_status = Some(format!("Report failed: {e}")),
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
        self.quick_recover.tick();
        self.artifacts.tick();
        self.text_scan.tick();
    }

    /// Acquire a volume quiesce guard for `path` (Windows only).
    ///
    /// If `path` is a `\\.\PhysicalDriveN` path, all volumes on that disk are
    /// taken offline to stop Windows Search, AutoPlay, and Explorer from
    /// competing with Ferrite's I/O.  Image files are silently skipped.
    /// The previous guard (if any) is dropped first, re-onlining the old drive.
    fn quiesce_volumes(&mut self, path: &str) {
        #[cfg(target_os = "windows")]
        {
            // Drop old guard first — re-onlines the previous drive's volumes.
            self.volume_guard = None;
            self.drive_select.vols_status = None;

            if let Some(disk_num) = parse_disk_number(path) {
                let (guard, status) = VolumeGuard::acquire(disk_num);
                self.drive_select.vols_status = Some(status);
                self.volume_guard = Some(guard);
            }
        }
        #[cfg(not(target_os = "windows"))]
        let _ = path;
    }

    /// Called whenever the active screen changes so screens can react on entry.
    fn on_screen_enter(&mut self) {
        if self.screen_idx == 5 {
            // Suggest a carved-files directory based on the imaging destination.
            self.carving.suggest_output_dir(&self.imaging.dest_path);
        }
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
            6 => self.hex_viewer.render(frame, chunks[1]),
            7 => self.quick_recover.render(frame, chunks[1]),
            8 => self.artifacts.render(frame, chunks[1]),
            9 => self.text_scan.render(frame, chunks[1]),
            _ => {}
        }

        // Session manager overlay (rendered on top of everything).
        if self.session_manager.visible {
            self.session_manager.render(frame, area);
        }

        // Help bar — show report_status for one render if set, else normal help.
        let help_text = if let Some(status) = &self.report_status {
            // Show report status and clear it next frame.
            let s = status.clone();
            // We clear it by rendering once then dropping on next tick — just
            // leave it set; the user can see it until next keypress.
            format!(" {s}")
        } else {
            help_line(self.screen_idx, self.selected_device.is_some()).to_string()
        };

        frame.render_widget(
            Paragraph::new(help_text).style(Style::default().fg(Color::DarkGray)),
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
        0 => " ↑/↓: navigate  Enter: select device  r: refresh list  Tab: next  q: quit  R: report",
        1 => " r: refresh S.M.A.R.T.  ↑/↓: scroll attrs  Tab: next  q: quit  R: report",
        2 => " d: dest  m: mapfile  l: start LBA  e: end LBA  b: block size  r: reverse  p: pause  s: start  c: cancel  Esc: stop edit  Tab: next  q: quit",
        3 => " r: read partition table  s: scan device  w: export  Tab: next  q: quit  R: report",
        4 => " ↑/↓: navigate  Enter: open dir  Backspace: go up  d: toggle deleted  o: open fs  Tab: next  q: quit",
        5 => " ↑/↓: navigate  Space: select hit  a: all  o: output dir  s: start  p: pause/resume  c: stop  e: extract one  E: extract selected  h: hex  v: preview  Tab: next  q: quit",
        6 => " ↑/↓: sector  PgUp/PgDn: ±16  Home/End  g: jump to LBA  b: jump to offset  Tab: next  q: quit",
        7 => " ↑/↓: navigate  Space: check  r: rescan  R: recover  /: filter  a: check-high  A: check-all  Esc: clear  Tab: next  q: quit",
        8 => " ↑/↓: navigate  s: scan  c: cancel  e: export CSV  o: output dir  0-6: filter kind  Tab: next  q: quit",
        9 => " ↑/↓: navigate  s: scan  c: cancel  e: export files  o: output dir  0-8: filter kind  Tab: next  q: quit",
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
        assert_eq!(SCREEN_NAMES.len(), 10);
    }
}

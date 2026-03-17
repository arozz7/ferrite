//! Key-input handling and scan lifecycle for [`CarvingState`].

use std::sync::Arc;
use std::time::Instant;

use crossterm::event::{KeyCode, KeyModifiers};
use ferrite_carver::{CarveHit, Carver, CarvingConfig, ScanProgress, Signature};

use super::{preview, CarveFocus, CarveMsg, CarveStatus, CarvingState, CursorRow, ScanRangeField};

impl CarvingState {
    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        // While editing the output directory, route all keys there.
        if self.editing_dir {
            match code {
                KeyCode::Esc | KeyCode::Enter => self.editing_dir = false,
                KeyCode::Backspace => {
                    self.output_dir.pop();
                }
                KeyCode::Char(c) if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT => {
                    self.output_dir.push(c);
                }
                _ => {}
            }
            return;
        }

        // While editing a scan range LBA field, route digit/backspace there.
        if self.scan_range_field != ScanRangeField::None {
            match code {
                KeyCode::Esc | KeyCode::Enter => {
                    self.scan_range_field = ScanRangeField::None;
                }
                KeyCode::Backspace => match self.scan_range_field {
                    ScanRangeField::Start => {
                        self.scan_start_lba_str.pop();
                    }
                    ScanRangeField::End => {
                        self.scan_end_lba_str.pop();
                    }
                    ScanRangeField::None => {}
                },
                KeyCode::Char(c)
                    if c.is_ascii_digit()
                        && (modifiers.is_empty() || modifiers == KeyModifiers::SHIFT) =>
                {
                    match self.scan_range_field {
                        ScanRangeField::Start => self.scan_start_lba_str.push(c),
                        ScanRangeField::End => self.scan_end_lba_str.push(c),
                        ScanRangeField::None => {}
                    }
                }
                _ => {}
            }
            return;
        }

        match code {
            // Switch focus between panels with Left / Right.
            KeyCode::Left => self.focus = CarveFocus::Signatures,
            KeyCode::Right if !self.hits.is_empty() => self.focus = CarveFocus::Hits,
            KeyCode::Up => {
                self.move_selection(-1);
                if self.show_preview && self.focus == CarveFocus::Hits {
                    self.refresh_preview();
                }
            }
            KeyCode::Down => {
                self.move_selection(1);
                if self.show_preview && self.focus == CarveFocus::Hits {
                    self.refresh_preview();
                }
            }
            KeyCode::PageUp if self.focus == CarveFocus::Hits => {
                self.hit_sel = self.hit_sel.saturating_sub(self.hits_page_size.max(1));
                if self.show_preview {
                    self.refresh_preview();
                }
            }
            KeyCode::PageDown if self.focus == CarveFocus::Hits => {
                if !self.hits.is_empty() {
                    let last = self.hits.len() - 1;
                    self.hit_sel = (self.hit_sel + self.hits_page_size.max(1)).min(last);
                    if self.show_preview {
                        self.refresh_preview();
                    }
                }
            }
            KeyCode::Home if self.focus == CarveFocus::Hits => {
                self.hit_sel = 0;
                if self.show_preview {
                    self.refresh_preview();
                }
            }
            KeyCode::End if self.focus == CarveFocus::Hits => {
                if !self.hits.is_empty() {
                    self.hit_sel = self.hits.len() - 1;
                    if self.show_preview {
                        self.refresh_preview();
                    }
                }
            }
            KeyCode::Char(' ') if self.focus == CarveFocus::Signatures => {
                self.toggle_signature();
            }
            KeyCode::Enter if self.focus == CarveFocus::Signatures => {
                self.toggle_group_expand();
            }
            KeyCode::Char(' ') if self.focus == CarveFocus::Hits => {
                self.toggle_hit_selected();
            }
            KeyCode::Char('a') if self.focus == CarveFocus::Hits => {
                self.toggle_select_all();
            }
            KeyCode::Char('o') => self.editing_dir = true,
            KeyCode::Char('[') if self.status != CarveStatus::Running => {
                self.scan_range_field = ScanRangeField::Start;
            }
            KeyCode::Char(']') if self.status != CarveStatus::Running => {
                self.scan_range_field = ScanRangeField::End;
            }
            KeyCode::Char('s') => self.start_scan(),
            KeyCode::Char('p') => self.toggle_pause(),
            KeyCode::Char('c') => {
                if self.extract_progress.is_some() {
                    self.cancel_extraction();
                } else {
                    self.cancel_scan();
                }
            }
            KeyCode::Char('d') => {
                self.extract_summary = None;
            }
            KeyCode::Char('e') => self.extract_selected(),
            KeyCode::Char('E') => self.extract_all_selected(),
            KeyCode::Char('x') => {
                self.auto_extract = !self.auto_extract;
            }
            KeyCode::Char('v') if self.focus == CarveFocus::Hits => {
                self.show_preview = !self.show_preview;
                if self.show_preview {
                    self.refresh_preview();
                } else {
                    self.current_preview = None;
                    self.preview_hit_idx = None;
                    self.preview_rx = None;
                    self.preview_loading = false;
                }
            }
            _ => {}
        }
    }

    pub(super) fn refresh_preview(&mut self) {
        use std::sync::mpsc;
        let hit_idx = self.hit_sel;
        if self.preview_hit_idx == Some(hit_idx) {
            return; // already cached or in-flight for this index
        }
        let hit = match self.hits.get(hit_idx) {
            Some(e) => e.hit.clone(),
            None => {
                self.current_preview = None;
                self.preview_hit_idx = None;
                self.preview_rx = None;
                self.preview_loading = false;
                return;
            }
        };
        let device = match &self.device {
            Some(d) => Arc::clone(d),
            None => {
                self.current_preview = None;
                self.preview_hit_idx = None;
                return;
            }
        };

        // Mark in-flight immediately so rapid navigation doesn't re-request.
        self.preview_hit_idx = Some(hit_idx);
        self.current_preview = None;
        self.preview_loading = true;

        let (tx, rx) = mpsc::sync_channel(1);
        self.preview_rx = Some(rx);

        std::thread::spawn(move || {
            let result = preview::read_preview(device.as_ref(), &hit);
            let _ = tx.send(result);
        });
    }

    pub(super) fn move_selection(&mut self, delta: i32) {
        match self.focus {
            CarveFocus::Signatures => {
                let len = self.cursor_rows.len();
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

    pub(super) fn toggle_signature(&mut self) {
        match self.cursor_rows.get(self.sig_sel).copied() {
            Some(CursorRow::Group(gi)) => {
                // Toggle all entries in the group: if all are on, turn them off;
                // otherwise turn them all on.
                let all_on = self.groups[gi].entries.iter().all(|e| e.enabled);
                let new_state = !all_on;
                for e in &mut self.groups[gi].entries {
                    e.enabled = new_state;
                }
            }
            Some(CursorRow::Sig(gi, si)) => {
                if let Some(e) = self.groups[gi].entries.get_mut(si) {
                    e.enabled = !e.enabled;
                }
            }
            None => {}
        }
    }

    pub(super) fn toggle_group_expand(&mut self) {
        if let Some(CursorRow::Group(gi)) = self.cursor_rows.get(self.sig_sel).copied() {
            self.groups[gi].expanded = !self.groups[gi].expanded;
            self.rebuild_cursor_rows();
        }
    }

    pub(super) fn start_scan(&mut self) {
        use std::sync::atomic::Ordering;
        use std::sync::mpsc;

        if self.status == CarveStatus::Running {
            return;
        }
        let device = match &self.device {
            Some(d) => Arc::clone(d),
            None => return,
        };
        let enabled: Vec<Signature> = self
            .groups
            .iter()
            .flat_map(|g| g.entries.iter())
            .filter(|e| e.enabled)
            .map(|e| e.sig.clone())
            .collect();
        if enabled.is_empty() {
            return;
        }
        let sector_size = device.sector_size() as u64;
        let window_start = self
            .scan_start_lba_str
            .trim()
            .parse::<u64>()
            .unwrap_or(0)
            .saturating_mul(sector_size);
        // Record the configured window start for progress display (so a resumed
        // scan shows overall completion rather than always starting at 0%).
        self.scan_window_start = window_start;

        // If a session resume position is set, pick up from where we left off
        // (clamped so it never falls before the configured window start).
        let start_byte = if self.resume_from_byte > window_start {
            self.resume_from_byte
        } else {
            window_start
        };
        self.resume_from_byte = 0;
        let end_byte = self
            .scan_end_lba_str
            .trim()
            .parse::<u64>()
            .ok()
            .map(|lba| lba.saturating_mul(sector_size));
        let config = CarvingConfig {
            signatures: enabled,
            scan_chunk_size: 4 * 1024 * 1024,
            start_byte,
            end_byte,
        };

        // Set up checkpoint path based on output dir.
        let dir = if self.output_dir.is_empty() {
            "carved".to_string()
        } else {
            self.output_dir.clone()
        };
        let cp_path = format!("{dir}\\ferrite-hits.jsonl");
        self.checkpoint_path = Some(cp_path);
        self.checkpoint_flushed = 0;

        self.cancel.store(false, Ordering::Relaxed);
        self.pause.store(false, Ordering::Relaxed);
        self.hits.clear();
        self.hit_sel = 0;
        self.scan_progress = None;
        self.scan_start = Some(Instant::now());
        self.paused_elapsed = std::time::Duration::ZERO;
        self.paused_since = None;
        self.status = CarveStatus::Running;

        let (tx, rx) = mpsc::channel::<CarveMsg>();
        // Keep tx alive so extraction results can be sent back after Done.
        self.tx = Some(tx.clone());
        self.rx = Some(rx);

        // Progress messages use a bounded sync channel so the scan thread
        // never blocks if the TUI is slow to drain.
        let (prog_tx, prog_rx) = mpsc::sync_channel::<ScanProgress>(32);

        // Forward ScanProgress from the bounded channel into the main CarveMsg channel.
        let fwd_tx = tx.clone();
        std::thread::spawn(move || {
            for p in prog_rx {
                let _ = fwd_tx.send(CarveMsg::Progress(p));
            }
        });

        let cancel_scan = Arc::clone(&self.cancel);
        let pause_scan = Arc::clone(&self.pause);
        let paused_ack_scan = Arc::clone(&self.paused_ack);

        std::thread::spawn(move || {
            let carver = Carver::new(Arc::clone(&device), config);
            let tx_hits = tx.clone();
            let mut on_hits = move |batch: Vec<CarveHit>| {
                let _ = tx_hits.send(CarveMsg::HitBatch(batch));
            };
            match carver.scan_streaming(
                &prog_tx,
                &cancel_scan,
                &pause_scan,
                &paused_ack_scan,
                &mut on_hits,
            ) {
                Ok(()) => {
                    drop(prog_tx); // signal the forwarder to exit
                    let _ = tx.send(CarveMsg::Done);
                }
                Err(e) => {
                    drop(prog_tx);
                    let _ = tx.send(CarveMsg::Error(e.to_string()));
                }
            }
        });
    }

    pub(super) fn toggle_pause(&mut self) {
        use std::sync::atomic::Ordering;

        // During active extraction the scan is already Done — handle separately.
        // Use extract_pause (not pause) so we don't interfere with back-pressure.
        if self.extract_progress.is_some() {
            let current = self.extract_pause.load(Ordering::Relaxed);
            self.extract_pause.store(!current, Ordering::Relaxed);
            return;
        }
        match self.status {
            CarveStatus::Running => {
                // User takes over from back-pressure: clear the auto flag so
                // this pause is now owned manually and won't be auto-lifted.
                self.backpressure_paused = false;
                self.pause.store(true, Ordering::Relaxed);
                // Transition to Pausing; events.rs will move to Paused once
                // the scan thread confirms via paused_ack.
                self.status = CarveStatus::Pausing;
            }
            // Allow cancelling the pause request before the thread acks it.
            CarveStatus::Pausing => {
                self.backpressure_paused = false;
                self.pause.store(false, Ordering::Relaxed);
                self.status = CarveStatus::Running;
            }
            CarveStatus::Paused => {
                // Manual resume clears back-pressure so the scan isn't
                // immediately re-paused if the queue is still above low-water.
                self.backpressure_paused = false;
                self.pause.store(false, Ordering::Relaxed);
                self.status = CarveStatus::Running;
                if let Some(since) = self.paused_since.take() {
                    self.paused_elapsed += since.elapsed();
                }
            }
            _ => {}
        }
    }

    pub(super) fn cancel_scan(&mut self) {
        use std::sync::atomic::Ordering;
        // Clear pause (including back-pressure) first so the scan thread isn't
        // spin-waiting when cancel fires.
        self.backpressure_paused = false;
        self.pause.store(false, Ordering::Relaxed);
        self.cancel.store(true, Ordering::Relaxed);
        // Leave rx/tx open — the scan thread will return partial hits via Done.
        // Status stays Running until the Done message arrives.
    }

    pub(super) fn cancel_extraction(&mut self) {
        use std::sync::atomic::Ordering;
        self.extract_cancel.store(true, Ordering::Relaxed);
        // extract_progress is cleared when ExtractionDone arrives.
    }

    pub(super) fn toggle_hit_selected(&mut self) {
        if let Some(e) = self.hits.get_mut(self.hit_sel) {
            e.selected = !e.selected;
            // Advance cursor so Space+Down becomes a quick multi-select gesture.
            let len = self.hits.len();
            if len > 0 {
                self.hit_sel = (self.hit_sel + 1).min(len - 1);
            }
        }
    }

    pub(super) fn toggle_select_all(&mut self) {
        let all_selected = self.hits.iter().all(|e| e.selected);
        let new_state = !all_selected;
        for e in &mut self.hits {
            e.selected = new_state;
        }
    }
}

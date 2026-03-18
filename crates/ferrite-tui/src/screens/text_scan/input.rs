//! Key-input handling for [`TextScanState`].

use crossterm::event::{KeyCode, KeyModifiers};
use ferrite_textcarver::TextKind;

use super::{ScanStatus, TextScanState};

impl TextScanState {
    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        // ── Consent dialog ────────────────────────────────────────────────────
        if self.show_consent {
            match code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                    self.consent_given = true;
                    self.show_consent = false;
                    self.start_scan();
                }
                _ => {
                    self.show_consent = false;
                }
            }
            return;
        }

        // ── Output dir editing ────────────────────────────────────────────────
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

        // ── Main keys ─────────────────────────────────────────────────────────
        match code {
            // Navigate block list.
            KeyCode::Up => {
                self.block_sel = self.block_sel.saturating_sub(1);
            }
            KeyCode::Down => {
                if !self.filtered.is_empty() {
                    let max = self.filtered.len() - 1;
                    self.block_sel = (self.block_sel + 1).min(max);
                }
            }
            KeyCode::PageUp => {
                self.block_sel = self.block_sel.saturating_sub(self.blocks_page_size.max(1));
            }
            KeyCode::PageDown => {
                if !self.filtered.is_empty() {
                    let max = self.filtered.len() - 1;
                    self.block_sel = (self.block_sel + self.blocks_page_size.max(1)).min(max);
                }
            }
            KeyCode::Home => self.block_sel = 0,
            KeyCode::End => {
                if !self.filtered.is_empty() {
                    self.block_sel = self.filtered.len() - 1;
                }
            }

            // Start scan.
            KeyCode::Char('s') if modifiers.is_empty() => {
                if self.device.is_some() && self.status != ScanStatus::Running {
                    if self.consent_given {
                        self.start_scan();
                    } else {
                        self.show_consent = true;
                    }
                }
            }

            // Cancel scan.
            KeyCode::Char('c') if modifiers.is_empty() => {
                if self.status == ScanStatus::Running {
                    self.cancel_scan();
                }
            }

            // Export blocks.
            KeyCode::Char('e') if modifiers.is_empty() => {
                self.export_files();
            }

            // Edit output dir.
            KeyCode::Char('o') if modifiers.is_empty() => {
                self.editing_dir = true;
            }

            // Filter: 0 = all, 1–8 = specific TextKind.
            KeyCode::Char('0') => {
                self.filter_kind = None;
                self.rebuild_filtered();
            }
            KeyCode::Char(c @ '1'..='8') => {
                let idx = (c as usize) - ('1' as usize);
                if let Some(&kind) = TextKind::all().get(idx) {
                    self.filter_kind = Some(kind);
                    self.rebuild_filtered();
                }
            }

            _ => {}
        }
    }
}

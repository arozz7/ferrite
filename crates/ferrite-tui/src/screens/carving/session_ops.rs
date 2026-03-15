//! Session persistence helpers for [`CarvingState`] — build/restore carving sessions.

use super::{checkpoint, CarvingState, HitEntry};

impl CarvingState {
    /// Load hits from a JSONL checkpoint file.
    pub(crate) fn load_checkpoint(&mut self, path: &str) {
        if let Ok(entries) = checkpoint::load(path) {
            self.hits = entries
                .into_iter()
                .map(|e| HitEntry {
                    hit: e.hit,
                    status: e.status,
                    selected: false,
                })
                .collect();
            self.checkpoint_path = Some(path.to_string());
            self.checkpoint_flushed = self.hits.len();
        }
    }

    /// Build a `CarvingSession` snapshot from the current state.
    pub(crate) fn build_session(
        &self,
        info: &ferrite_core::types::DeviceInfo,
    ) -> crate::carving_session::CarvingSession {
        use crate::carving_session::CarvingSession;
        let start_lba = self.scan_start_lba_str.trim().parse::<u64>().unwrap_or(0);
        let end_lba = self.scan_end_lba_str.trim().parse::<u64>().unwrap_or(0);
        let last_byte = self
            .scan_progress
            .as_ref()
            .map(|p| p.bytes_scanned)
            .unwrap_or(0);
        let hits_file = self.checkpoint_path.clone().unwrap_or_default();
        let saved_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        CarvingSession {
            drive_serial: info.serial.clone().unwrap_or_default(),
            drive_model: info.model.clone().unwrap_or_default(),
            drive_size: info.size_bytes,
            scan_start_lba: start_lba,
            scan_end_lba: end_lba,
            last_scanned_byte: last_byte,
            output_dir: self.output_dir.clone(),
            hits_file,
            hits_count: self.hits.len(),
            saved_at,
        }
    }

    /// Restore state from a saved `CarvingSession`.
    pub(crate) fn restore_from_session(
        &mut self,
        session: &crate::carving_session::CarvingSession,
    ) {
        self.output_dir = session.output_dir.clone();
        self.scan_start_lba_str = if session.scan_start_lba > 0 {
            session.scan_start_lba.to_string()
        } else {
            String::new()
        };
        self.scan_end_lba_str = if session.scan_end_lba > 0 {
            session.scan_end_lba.to_string()
        } else {
            String::new()
        };
        if !session.hits_file.is_empty() {
            self.load_checkpoint(&session.hits_file);
        }
    }
}

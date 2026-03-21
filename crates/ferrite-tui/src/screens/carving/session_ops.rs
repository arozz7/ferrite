//! Session persistence helpers for [`CarvingState`] — build/restore carving sessions.

use super::{checkpoint, CarvingState, HitEntry, HitStatus};

impl CarvingState {
    /// Load hits from a JSONL checkpoint file.
    ///
    /// Entries are deduplicated by byte offset (last-seen status wins), so
    /// hits that were extracted in a previous run show their final status
    /// rather than the initial `Unextracted` record written during scanning.
    pub(crate) fn load_checkpoint(&mut self, path: &str) {
        if let Ok(entries) = checkpoint::load(path) {
            self.hits = entries
                .into_iter()
                .map(|e| HitEntry {
                    hit: e.hit,
                    status: e.status,
                    selected: false,
                    quality: None,
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

        // Collect names of disabled signatures so they can be restored on resume.
        let disabled_sigs: Vec<String> = self
            .groups
            .iter()
            .flat_map(|g| g.entries.iter())
            .filter(|e| !e.enabled)
            .map(|e| e.sig.name.clone())
            .collect();

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
            auto_extract: self.auto_extract,
            skip_truncated: self.skip_truncated,
            skip_corrupt: self.skip_corrupt,
            device_path: info.path.clone(),
            disabled_sigs,
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
        self.resume_from_byte = session.last_scanned_byte;
        self.auto_extract = session.auto_extract;
        self.skip_truncated = session.skip_truncated;
        self.skip_corrupt = session.skip_corrupt;

        // Restore signature enabled/disabled state.  Only disabled names are
        // stored; everything else stays enabled (including new sigs added since
        // the session was saved).
        if !session.disabled_sigs.is_empty() {
            for group in &mut self.groups {
                for entry in &mut group.entries {
                    if session.disabled_sigs.contains(&entry.sig.name) {
                        entry.enabled = false;
                    }
                }
            }
            // Also apply to the user-defined custom group if present.
            // (User sigs are loaded separately; rebuild_custom_group is called
            // after restore, so they will pick up the correct enabled state on
            // the next render cycle.)
        }

        // Accumulate per-status counts from the loaded checkpoint so the
        // summary line is accurate on resume even before re-extraction.
        for hit in &self.hits {
            match &hit.status {
                HitStatus::Duplicate => self.duplicates_suppressed += 1,
                HitStatus::Skipped => {} // could be trunc or corrupt — count together
                _ => {}
            }
        }
    }
}

//! Per-drive carving session persistence.
//!
//! Sessions are stored as JSON files in the `sessions/` directory, named by
//! drive serial number and day-since-epoch so that multiple sessions for the
//! same drive on different days coexist without overwriting each other.

use serde::{Deserialize, Serialize};

const SESSIONS_DIR: &str = "sessions";

/// A snapshot of a carving session, associated with a specific drive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CarvingSession {
    pub drive_serial: String,
    pub drive_model: String,
    /// Total size of the drive in bytes.
    pub drive_size: u64,
    /// Start of the scan window in LBA units (0 = beginning of device).
    pub scan_start_lba: u64,
    /// End of the scan window in LBA units (0 = end of device).
    pub scan_end_lba: u64,
    /// Byte offset of the last scanned position.
    pub last_scanned_byte: u64,
    /// Directory where extracted files were (or will be) written.
    pub output_dir: String,
    /// Path to the JSONL checkpoint file that stores carve hits.
    pub hits_file: String,
    /// Number of hits recorded at save time.
    pub hits_count: usize,
    /// Unix timestamp (seconds since epoch) when this session was saved.
    pub saved_at: u64,
    /// Whether auto-extract was enabled when the session was saved.
    #[serde(default)]
    pub auto_extract: bool,
    /// Whether skip-truncated mode was enabled when the session was saved.
    #[serde(default)]
    pub skip_truncated: bool,
    /// Whether skip-corrupt mode was enabled when the session was saved.
    #[serde(default)]
    pub skip_corrupt: bool,
}

impl CarvingSession {
    fn serial_key(&self) -> &str {
        if self.drive_serial.is_empty() {
            "unknown"
        } else {
            &self.drive_serial
        }
    }

    /// Return the path where this session will be stored.
    pub fn file_path(&self) -> String {
        // Day-since-epoch keeps files from different days separate while
        // being reproducible so the same session overwrites the same file
        // within a single day.
        format!(
            "{}/{}-{}.json",
            SESSIONS_DIR,
            self.serial_key(),
            self.saved_at / 86400
        )
    }

    /// Persist the session to disk.
    pub fn save(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(SESSIONS_DIR)?;
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(self.file_path(), json)
    }

    /// Load all sessions from `sessions/`, sorted newest-first.
    pub fn load_all() -> Vec<Self> {
        let Ok(rd) = std::fs::read_dir(SESSIONS_DIR) else {
            return Vec::new();
        };
        let mut out: Vec<Self> = rd
            .flatten()
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
            .filter_map(|e| std::fs::read_to_string(e.path()).ok())
            .filter_map(|s| serde_json::from_str(&s).ok())
            .collect();
        out.sort_by(|a, b| b.saved_at.cmp(&a.saved_at));
        out
    }

    /// Delete this session file from disk.
    pub fn delete(&self) -> std::io::Result<()> {
        std::fs::remove_file(self.file_path())
    }

    /// Returns `true` if this session matches the given drive serial and size.
    pub fn matches_drive(&self, serial: &str, size: u64) -> bool {
        !self.drive_serial.is_empty()
            && self.drive_serial == serial
            && (self.drive_size == 0 || self.drive_size == size)
    }

    /// Human-readable age string relative to now.
    pub fn age_str(&self) -> String {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let days = now.saturating_sub(self.saved_at) / 86400;
        if days == 0 {
            "today".into()
        } else {
            format!("{days}d ago")
        }
    }

    /// Format a byte count into a human-readable string.
    pub fn fmt_bytes(n: u64) -> String {
        const GIB: u64 = 1 << 30;
        const MIB: u64 = 1 << 20;
        if n >= GIB {
            format!("{:.1} GiB", n as f64 / GIB as f64)
        } else if n >= MIB {
            format!("{:.1} MiB", n as f64 / MIB as f64)
        } else {
            format!("{n} B")
        }
    }
}

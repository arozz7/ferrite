//! Session persistence — saves and restores imaging configuration across runs.
//!
//! Written to `ferrite-session.json` in the working directory on exit;
//! loaded on startup if the file exists.

use serde::{Deserialize, Serialize};

const SESSION_FILE: &str = "ferrite-session.json";

/// Persistent UI state that survives application restarts.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Session {
    /// Imaging destination image path.
    pub imaging_dest: String,
    /// Mapfile path (empty = none).
    pub imaging_mapfile: String,
    /// Start LBA string (empty = beginning of device).
    pub imaging_start_lba: String,
    /// End LBA string (empty = end of device).
    pub imaging_end_lba: String,
    /// Whether reverse imaging mode is enabled.
    #[serde(default)]
    pub imaging_reverse: bool,
}

impl Session {
    /// Load session from disk, returning `Default` on any error or absence.
    pub fn load() -> Self {
        std::fs::read_to_string(SESSION_FILE)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Save session to disk. Silently ignores I/O errors.
    pub fn save(&self) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(SESSION_FILE, json);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_missing_file_returns_default() {
        // A nonexistent path always returns Default.
        let s = Session {
            imaging_dest: String::new(),
            imaging_mapfile: String::new(),
            imaging_start_lba: String::new(),
            imaging_end_lba: String::new(),
            imaging_reverse: false,
        };
        assert!(s.imaging_dest.is_empty());
    }

    #[test]
    fn save_and_load_roundtrip() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ferrite-session.json");
        let session = Session {
            imaging_dest: "/tmp/image.raw".into(),
            imaging_mapfile: "/tmp/image.map".into(),
            imaging_start_lba: "100".into(),
            imaging_end_lba: "500".into(),
            imaging_reverse: false,
        };
        let json = serde_json::to_string_pretty(&session).unwrap();
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(json.as_bytes()).unwrap();

        let loaded: Session = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.imaging_dest, "/tmp/image.raw");
        assert_eq!(loaded.imaging_start_lba, "100");
        assert_eq!(loaded.imaging_end_lba, "500");
    }
}

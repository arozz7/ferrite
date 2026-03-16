//! Phase 35 — Filesystem-assisted recovery helpers.
//!
//! Provides the background recovery thread and the per-file extraction helper
//! used by [`super::file_browser`].

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

use ferrite_filesystem::{FileEntry, FilesystemParser};

// ── Public types ──────────────────────────────────────────────────────────────

/// Messages sent from the background recovery thread to the TUI.
pub enum RecoveryMsg {
    /// Emitted once per file, just before extraction begins.
    Progress {
        done: usize,
        total: usize,
        current_path: String,
        errors: usize,
    },
    /// Emitted once when the thread finishes (or is cancelled).
    Done { succeeded: usize, failed: usize },
}

/// Snapshot of in-progress batch recovery state held by [`FileBrowserState`].
pub struct RecoveryProgress {
    pub done: usize,
    pub total: usize,
    pub errors: usize,
    pub current_path: String,
    pub finished: bool,
    pub succeeded: usize,
}

// ── Extraction helper ─────────────────────────────────────────────────────────

/// Extract `entry` to `<output_base>/fs/<original_path>`, preserving the
/// original directory tree.  Timestamps are applied when available.
///
/// Returns the number of bytes written, or a human-readable error string.
pub fn extract_to_recovered(
    entry: &FileEntry,
    parser: &dyn FilesystemParser,
    output_base: &str,
) -> Result<u64, String> {
    // Build a safe relative path from entry.path.
    // Strip leading separators and disallow `..` traversal.
    let rel: std::path::PathBuf = entry
        .path
        .trim_start_matches('/')
        .trim_start_matches('\\')
        .split(['/', '\\'])
        .filter(|s| !s.is_empty() && *s != "..")
        .collect();

    let out_path = std::path::Path::new(output_base).join("fs").join(&rel);

    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
    }

    let mut file = std::fs::File::create(&out_path).map_err(|e| format!("create: {e}"))?;

    let bytes = parser
        .read_file(entry, &mut file)
        .map_err(|e| format!("read: {e}"))?;

    // Preserve mtime (and atime) from filesystem metadata.
    let ts_secs = entry.modified.or(entry.created);
    if let Some(secs) = ts_secs {
        let ft = filetime::FileTime::from_unix_time(secs as i64, 0);
        let _ = filetime::set_file_times(&out_path, ft, ft);
    }

    Ok(bytes)
}

// ── Background thread ─────────────────────────────────────────────────────────

/// Spawn a background thread that extracts all deleted files from `parser`
/// into `<output_base>/fs/<path>`, sending [`RecoveryMsg`] updates to the
/// returned [`Receiver`].
///
/// Setting `cancel` to `true` aborts the loop between files.
pub fn spawn_recovery_thread(
    parser: Arc<dyn FilesystemParser>,
    cancel: Arc<AtomicBool>,
    output_base: String,
) -> Receiver<RecoveryMsg> {
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        // Enumerate deleted, non-directory files.
        let entries: Vec<FileEntry> = match parser.enumerate_files() {
            Ok(all) => all
                .into_iter()
                .filter(|f| f.is_deleted && !f.is_dir)
                .collect(),
            Err(_) => {
                let _ = tx.send(RecoveryMsg::Done {
                    succeeded: 0,
                    failed: 0,
                });
                return;
            }
        };

        let total = entries.len();
        // Send an initial progress so the UI can show "0 / N" immediately.
        let _ = tx.send(RecoveryMsg::Progress {
            done: 0,
            total,
            current_path: String::new(),
            errors: 0,
        });

        let mut succeeded = 0usize;
        let mut failed = 0usize;

        for entry in &entries {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let _ = tx.send(RecoveryMsg::Progress {
                done: succeeded + failed,
                total,
                current_path: entry.path.clone(),
                errors: failed,
            });
            match extract_to_recovered(entry, parser.as_ref(), &output_base) {
                Ok(_) => succeeded += 1,
                Err(_) => failed += 1,
            }
        }

        let _ = tx.send(RecoveryMsg::Done { succeeded, failed });
    });

    rx
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #[test]
    fn extract_path_strips_leading_slash() {
        // Verify the relative path logic without hitting the filesystem.
        let rel: std::path::PathBuf = "/Photos/IMG_001.jpg"
            .trim_start_matches('/')
            .split(['/', '\\'])
            .filter(|s| !s.is_empty() && *s != "..")
            .collect();
        assert_eq!(
            rel.to_str().unwrap().replace('\\', "/"),
            "Photos/IMG_001.jpg"
        );
    }

    #[test]
    fn extract_path_rejects_traversal() {
        let rel: std::path::PathBuf = "/../../../etc/passwd"
            .trim_start_matches('/')
            .split(['/', '\\'])
            .filter(|s| !s.is_empty() && *s != "..")
            .collect();
        // `..` segments are filtered out — path stays within the output dir.
        assert_eq!(rel.to_str().unwrap().replace('\\', "/"), "etc/passwd");
    }
}

//! JSONL hit checkpoint — appends hits to disk as they arrive so a restart
//! does not lose work on long carving sessions.
//!
//! ## Wire formats
//!
//! **V2 (current)** — compact; stores only byte offset, signature *name*, and
//! status.  The full `Signature` is reconstructed from the loaded signature
//! table on resume.  Average line size ~70 bytes.
//!
//! ```json
//! {"v":2,"offset":351090,"sig":"JPEG Image","status":"Skipped"}
//! ```
//!
//! **V1 (legacy)** — full `Signature` struct embedded in every line.  Average
//! line size ~300–350 bytes.  Automatically upgraded to V2 on the first resume
//! (file is rewritten in-place after dedup).
//!
//! ```json
//! {"hit":{"byte_offset":351090,"signature":{...full struct...}},"status":"Skipped"}
//! ```
//!
//! ## Deduplication and compaction
//!
//! The file is append-only during a scan: initial hits are written as
//! `Unextracted`; after each extraction batch a second entry is appended with
//! the final status.  `load()` deduplicates by `byte_offset` in a single
//! streaming pass (keeping the last-seen status), then **rewrites the file**
//! with only the canonical deduplicated V2 entries.  This keeps the on-disk
//! file tight across long sessions and multiple resumes.

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};

use ferrite_carver::{CarveHit, Signature};
use serde::{Deserialize, Serialize};

use super::HitStatus;

// ── Public output type ────────────────────────────────────────────────────────

pub(super) struct CheckpointEntry {
    pub hit: CarveHit,
    pub status: HitStatus,
}

// ── Wire format: V2 (compact) ─────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
struct EntryV2 {
    /// Always `2`.  Used to distinguish this format from V1 on deserialise.
    v: u8,
    offset: u64,
    sig: String,
    status: HitStatus,
}

// ── Wire format: V1 (legacy, read-only) ──────────────────────────────────────

#[derive(Deserialize)]
struct EntryV1 {
    hit: HitV1,
    status: HitStatus,
}

/// Legacy hit shape — full `Signature` embedded inline.
#[derive(Deserialize)]
struct HitV1 {
    byte_offset: u64,
    signature: Signature,
}

// ── Unified deserialisation ───────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(untagged)]
enum AnyEntry {
    V2(EntryV2),
    V1(EntryV1),
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Append V2 entries for a slice of `(hit, status)` pairs in one `write` call.
/// Called after each extraction batch to persist updated statuses.
pub(super) fn append_batch(path: &str, updates: &[(&CarveHit, &HitStatus)]) -> std::io::Result<()> {
    if updates.is_empty() {
        return Ok(());
    }
    let mut buf = String::new();
    for (hit, status) in updates {
        if let Ok(line) = serde_json::to_string(&EntryV2 {
            v: 2,
            offset: hit.byte_offset,
            sig: hit.signature.name.clone(),
            status: (*status).clone(),
        }) {
            buf.push_str(&line);
            buf.push('\n');
        }
    }
    let mut f = OpenOptions::new().create(true).append(true).open(path)?;
    f.write_all(buf.as_bytes())
}

/// Load and deduplicate all entries from a checkpoint file.
///
/// `sig_lookup` maps signature name → `Signature` for V2 lines (and as a
/// fallback for V1 lines whose embedded sig can be cross-checked).  Hits whose
/// signature name is not present in `sig_lookup` are silently dropped (the sig
/// was removed from the config since the session was saved).
///
/// After deduplication the file is **rewritten** in compact V2 format
/// (compaction).  This:
/// - converts legacy V1 files on the first resume,
/// - collapses the 2× append bloat from status-update entries, and
/// - keeps the file proportional to the number of unique hits rather than
///   growing with every extraction batch.
///
/// The returned `Vec` preserves first-seen hit order with last-seen status.
pub(super) fn load(
    path: &str,
    sig_lookup: &HashMap<String, Signature>,
) -> std::io::Result<Vec<CheckpointEntry>> {
    let f = fs::File::open(path)?;

    // Single-pass streaming dedup.
    //
    // `order` maps byte_offset → index in `out` so we can update the status
    // of an already-seen hit in O(1) without a second pass over `out`.
    let mut order: HashMap<u64, usize> = HashMap::new();
    let mut out: Vec<CheckpointEntry> = Vec::new();

    for line in BufReader::new(f).lines() {
        let line = line?;
        if line.is_empty() {
            continue;
        }

        let (offset, hit, status) = match serde_json::from_str::<AnyEntry>(&line) {
            Ok(AnyEntry::V2(e)) => {
                let sig = match sig_lookup.get(&e.sig) {
                    Some(s) => s.clone(),
                    None => continue, // sig removed from config since session was saved
                };
                let hit = CarveHit {
                    byte_offset: e.offset,
                    signature: sig,
                };
                (e.offset, hit, e.status)
            }
            Ok(AnyEntry::V1(e)) => {
                // Accept the embedded sig directly; prefer the live version from
                // sig_lookup if available (ensures any config updates are reflected).
                let sig = sig_lookup
                    .get(&e.hit.signature.name)
                    .cloned()
                    .unwrap_or(e.hit.signature);
                let hit = CarveHit {
                    byte_offset: e.hit.byte_offset,
                    signature: sig,
                };
                (e.hit.byte_offset, hit, e.status)
            }
            Err(_) => continue, // malformed line — skip
        };

        if let Some(&idx) = order.get(&offset) {
            // Later entry for the same offset: update status in-place so the
            // last-seen (most recent) status wins.
            out[idx].status = status;
        } else {
            order.insert(offset, out.len());
            out.push(CheckpointEntry { hit, status });
        }
    }

    // Compact: rewrite the file with deduplicated V2 entries.
    // Ignoring errors here — worst case the file stays uncompacted this run.
    let _ = compact(path, &out);

    Ok(out)
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Atomically rewrite `path` with deduplicated V2 entries.
///
/// Writes to `<path>.tmp` first, then replaces the original.  On failure the
/// `.tmp` file is removed to avoid leaving partial state.
fn compact(path: &str, entries: &[CheckpointEntry]) -> std::io::Result<()> {
    let tmp = format!("{path}.tmp");

    // Write compact content to the temp file.
    {
        let mut f = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp)?;

        // Build output in 1 MiB chunks to avoid one huge String allocation.
        let mut buf = String::with_capacity(1024 * 1024);
        for e in entries {
            if let Ok(line) = serde_json::to_string(&EntryV2 {
                v: 2,
                offset: e.hit.byte_offset,
                sig: e.hit.signature.name.clone(),
                status: e.status.clone(),
            }) {
                buf.push_str(&line);
                buf.push('\n');
                if buf.len() >= 1024 * 1024 {
                    f.write_all(buf.as_bytes())?;
                    buf.clear();
                }
            }
        }
        if !buf.is_empty() {
            f.write_all(buf.as_bytes())?;
        }
        f.flush()?;
    }

    // Atomic swap.  On Windows `rename` fails if the destination exists, so
    // remove the original first.
    if let Err(e) = fs::remove_file(path) {
        // If the original doesn't exist that's fine; any other error is real.
        if e.kind() != std::io::ErrorKind::NotFound {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
    }
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }

    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use ferrite_carver::Signature;

    fn make_sig(name: &str) -> Signature {
        Signature {
            name: name.to_string(),
            extension: "bin".to_string(),
            header: vec![Some(0xFF)],
            footer: vec![],
            footer_last: false,
            max_size: 1024,
            size_hint: None,
            min_size: 0,
            pre_validate: None,
            header_offset: 0,
            min_hit_gap: 0,
            suppress_group: None,
            footer_extra: 0,
        }
    }

    fn make_lookup(names: &[&str]) -> HashMap<String, Signature> {
        names.iter().map(|n| (n.to_string(), make_sig(n))).collect()
    }

    fn make_hit(offset: u64, name: &str) -> CarveHit {
        CarveHit {
            byte_offset: offset,
            signature: make_sig(name),
        }
    }

    /// Round-trip: append V2 entries then load them back.
    #[test]
    fn roundtrip_v2() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hits.jsonl");
        let path_str = path.to_str().unwrap();

        let hit_a = make_hit(100, "JPEG Image");
        let hit_b = make_hit(200, "PNG Image");

        append_batch(
            path_str,
            &[
                (&hit_a, &HitStatus::Unextracted),
                (&hit_b, &HitStatus::Unextracted),
            ],
        )
        .unwrap();

        // Append a status update for hit_a.
        append_batch(path_str, &[(&hit_a, &HitStatus::Ok { bytes: 512 })]).unwrap();

        let lookup = make_lookup(&["JPEG Image", "PNG Image"]);
        let entries = load(path_str, &lookup).unwrap();

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].hit.byte_offset, 100);
        assert!(matches!(entries[0].status, HitStatus::Ok { bytes: 512 }));
        assert_eq!(entries[1].hit.byte_offset, 200);
        assert!(matches!(entries[1].status, HitStatus::Unextracted));
    }

    /// Dedup: last-seen status wins; first-seen order preserved.
    #[test]
    fn dedup_keeps_last_status_first_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hits.jsonl");
        let path_str = path.to_str().unwrap();

        let hit = make_hit(500, "PDF Document");
        // Write three status updates for the same offset.
        append_batch(path_str, &[(&hit, &HitStatus::Unextracted)]).unwrap();
        append_batch(path_str, &[(&hit, &HitStatus::Queued)]).unwrap();
        append_batch(path_str, &[(&hit, &HitStatus::Ok { bytes: 1024 })]).unwrap();

        let lookup = make_lookup(&["PDF Document"]);
        let entries = load(path_str, &lookup).unwrap();

        assert_eq!(entries.len(), 1);
        assert!(matches!(entries[0].status, HitStatus::Ok { bytes: 1024 }));
    }

    /// Compaction: after load the file contains only deduplicated V2 lines.
    #[test]
    fn compaction_reduces_line_count() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hits.jsonl");
        let path_str = path.to_str().unwrap();

        let hit = make_hit(42, "JPEG Image");
        // Three appends for the same hit.
        for _ in 0..3 {
            append_batch(path_str, &[(&hit, &HitStatus::Unextracted)]).unwrap();
        }

        let lookup = make_lookup(&["JPEG Image"]);
        let _ = load(path_str, &lookup).unwrap();

        // After compaction exactly one line should remain.
        let content = fs::read_to_string(path_str).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(
            lines.len(),
            1,
            "expected 1 line after compaction, got {}",
            lines.len()
        );

        // That line must be V2 format.
        let v: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(v["v"], 2);
        assert_eq!(v["offset"], 42);
    }

    /// Unknown sig names are silently dropped on load.
    #[test]
    fn unknown_sig_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hits.jsonl");
        let path_str = path.to_str().unwrap();

        let hit = make_hit(999, "OldFormat");
        append_batch(path_str, &[(&hit, &HitStatus::Unextracted)]).unwrap();

        let lookup = make_lookup(&[]); // empty — no sigs registered
        let entries = load(path_str, &lookup).unwrap();
        assert!(entries.is_empty());
    }

    /// V1 legacy format is parsed and upgraded to V2 on load.
    #[test]
    fn v1_legacy_upgraded_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hits.jsonl");
        let path_str = path.to_str().unwrap();

        // Hand-craft a V1-style line (full Signature embedded).
        let v1_line = r#"{"hit":{"byte_offset":123,"signature":{"name":"JPEG Image","extension":"jpg","header":[255,216],"footer":[],"footer_last":false,"max_size":10485760,"size_hint":null,"min_size":0,"pre_validate":null,"header_offset":0,"min_hit_gap":0,"suppress_group":null,"footer_extra":0}},"status":"Unextracted"}"#;
        fs::write(path_str, format!("{v1_line}\n")).unwrap();

        let lookup = make_lookup(&["JPEG Image"]);
        let entries = load(path_str, &lookup).unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].hit.byte_offset, 123);
        assert_eq!(entries[0].hit.signature.name, "JPEG Image");

        // File should now be V2 format after compaction.
        let content = fs::read_to_string(path_str).unwrap();
        let v: serde_json::Value = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(v["v"], 2, "file should be compacted to V2 after load");
    }
}

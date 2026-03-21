//! JSONL hit checkpoint — appends hits to disk as they arrive so a restart
//! does not lose work on long carving sessions.
//!
//! Each line is a `CheckpointEntry` (hit + status).  Hits are appended when
//! first discovered (status = `Unextracted`).  After each extraction batch
//! completes, updated entries are appended for every hit whose status changed.
//! On load, entries are deduplicated by `byte_offset`, keeping the *last*
//! occurrence so final extraction outcomes win over the initial `Unextracted`
//! record.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};

use ferrite_carver::CarveHit;
use serde::{Deserialize, Serialize};

use super::HitStatus;

#[derive(Serialize, Deserialize)]
pub(super) struct CheckpointEntry {
    pub hit: CarveHit,
    pub status: HitStatus,
}

/// Append one entry to the JSONL checkpoint file (creates file if absent).
pub(super) fn append(path: &str, hit: &CarveHit, status: &HitStatus) -> std::io::Result<()> {
    let line = serde_json::to_string(&CheckpointEntry {
        hit: hit.clone(),
        status: status.clone(),
    })
    .map_err(std::io::Error::other)?;
    let mut f = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(f, "{line}")
}

/// Append updated statuses for a slice of `(hit, status)` pairs in one
/// `write` call.  Used after each extraction batch to persist outcomes.
pub(super) fn append_batch(path: &str, updates: &[(&CarveHit, &HitStatus)]) -> std::io::Result<()> {
    if updates.is_empty() {
        return Ok(());
    }
    let mut buf = String::new();
    for (hit, status) in updates {
        if let Ok(line) = serde_json::to_string(&CheckpointEntry {
            hit: (*hit).clone(),
            status: (*status).clone(),
        }) {
            buf.push_str(&line);
            buf.push('\n');
        }
    }
    let mut f = OpenOptions::new().create(true).append(true).open(path)?;
    f.write_all(buf.as_bytes())
}

/// Load all entries from an existing JSONL checkpoint.
///
/// Entries are deduplicated by `byte_offset`, keeping the **last** occurrence
/// so that post-extraction status updates correctly reflect what was actually
/// extracted.  The returned vec preserves first-seen order.
pub(super) fn load(path: &str) -> std::io::Result<Vec<CheckpointEntry>> {
    let f = std::fs::File::open(path)?;
    let mut raw: Vec<CheckpointEntry> = Vec::new();
    for line in BufReader::new(f).lines() {
        let line = line?;
        if let Ok(e) = serde_json::from_str::<CheckpointEntry>(&line) {
            raw.push(e);
        }
    }

    // Pass 1: record the last-seen status for each byte_offset.
    let mut final_status: HashMap<u64, HitStatus> = HashMap::with_capacity(raw.len());
    for e in &raw {
        final_status.insert(e.hit.byte_offset, e.status.clone());
    }

    // Pass 2: emit first-seen entry for each offset, with the final status.
    let mut emitted: HashMap<u64, ()> = HashMap::with_capacity(final_status.len());
    let mut out: Vec<CheckpointEntry> = Vec::with_capacity(final_status.len());
    for e in raw {
        let offset = e.hit.byte_offset;
        if emitted.insert(offset, ()).is_none() {
            let status = final_status.remove(&offset).unwrap_or(e.status);
            out.push(CheckpointEntry { hit: e.hit, status });
        }
    }
    Ok(out)
}

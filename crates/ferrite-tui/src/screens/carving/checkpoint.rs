//! JSONL hit checkpoint — appends hits to disk as they arrive so a restart
//! does not lose work on long carving sessions.

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

/// Load all entries from an existing JSONL checkpoint.  Silently skips malformed lines.
pub(super) fn load(path: &str) -> std::io::Result<Vec<CheckpointEntry>> {
    let f = std::fs::File::open(path)?;
    let mut entries = Vec::new();
    for line in BufReader::new(f).lines() {
        let line = line?;
        if let Ok(e) = serde_json::from_str::<CheckpointEntry>(&line) {
            entries.push(e);
        }
    }
    Ok(entries)
}

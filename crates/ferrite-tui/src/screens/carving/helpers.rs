//! Formatting, deduplication, and signature-loading helpers for the carving screen.

use std::collections::HashMap;

use ferrite_carver::CarvingConfig;

use super::{SigEntry, SigGroup};

// ── Formatting ────────────────────────────────────────────────────────────────

pub(crate) fn fmt_bytes(n: u64) -> String {
    const GIB: u64 = 1024 * 1024 * 1024;
    const MIB: u64 = 1024 * 1024;
    if n >= GIB {
        format!("{:.1} GiB", n as f64 / GIB as f64)
    } else if n >= MIB {
        format!("{:.1} MiB", n as f64 / MIB as f64)
    } else {
        format!("{n} B")
    }
}

// ── Signature grouping ────────────────────────────────────────────────────────

/// Display order for signature groups.
const GROUP_ORDER: &[&str] = &[
    "Images",
    "RAW Photos",
    "Video",
    "Audio",
    "Documents",
    "Office & Email",
    "Archives",
    "System",
    "Other",
];

/// Returns the group label for a given file extension.
fn sig_group_label(ext: &str) -> &'static str {
    match ext {
        "jpg" | "png" | "gif" | "bmp" | "tif" | "webp" | "psd" => "Images",
        "arw" | "cr2" | "nef" | "rw2" | "raf" | "heic" => "RAW Photos",
        "mp4" | "mov" | "m4v" | "3gp" | "avi" | "mkv" | "webm" | "wmv" | "flv" | "mpg" => "Video",
        "mp3" | "flac" | "wav" | "ogg" | "m4a" => "Audio",
        "pdf" | "xml" | "html" | "rtf" | "vcf" | "ics" | "eml" => "Documents",
        "zip" | "ole" | "pst" => "Office & Email",
        "rar" | "7z" | "gz" => "Archives",
        "db" | "vmdk" | "evtx" | "exe" | "elf" | "dat" | "vhd" | "vhdx" | "qcow2" => "System",
        _ => "Other",
    }
}

// ── Signature loading ─────────────────────────────────────────────────────────

/// Load all built-in signatures and return them organised into labelled groups.
/// Groups are ordered by `GROUP_ORDER`; each group starts collapsed.
pub(super) fn load_builtin_sig_groups() -> Vec<SigGroup> {
    let entries = match CarvingConfig::from_toml_str(crate::SIGNATURES_TOML) {
        Ok(cfg) => cfg
            .signatures
            .into_iter()
            .map(|sig| SigEntry { sig, enabled: true })
            .collect::<Vec<_>>(),
        Err(e) => {
            tracing::error!(?e, "failed to load built-in signatures");
            return Vec::new();
        }
    };

    let mut map: HashMap<&'static str, Vec<SigEntry>> = HashMap::new();
    for entry in entries {
        let label = sig_group_label(&entry.sig.extension);
        map.entry(label).or_default().push(entry);
    }

    GROUP_ORDER
        .iter()
        .filter_map(|&label| {
            map.remove(label)
                .filter(|v| !v.is_empty())
                .map(|entries| SigGroup {
                    label,
                    expanded: false,
                    entries,
                })
        })
        .collect()
}

//! Formatting, deduplication, and signature-loading helpers for the carving screen.

use std::collections::HashMap;

use ferrite_carver::CarvingConfig;

use super::user_sigs::UserSigDef;
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
/// "Custom" is intentionally omitted — it is appended dynamically when present.
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
        "jpg" | "png" | "gif" | "bmp" | "tif" | "webp" | "psd" | "exr" | "xcf" | "jp2" | "pcx"
        | "bpg" => "Images",
        "arw" | "cr2" | "nef" | "rw2" | "raf" | "heic" | "orf" | "pef" | "cr3" | "sr2" | "dcr"
        | "crw" | "mrw" | "x3f" => "RAW Photos",
        "mp4" | "mov" | "m4v" | "3gp" | "avi" | "mkv" | "webm" | "wmv" | "flv" | "mpg" | "rm"
        | "swf" | "ts" | "m2ts" | "wtv" => "Video",
        "mp3" | "flac" | "wav" | "ogg" | "m4a" | "mid" | "aif" | "wv" | "ape" | "au" | "aac" => {
            "Audio"
        }
        "pdf" | "xml" | "html" | "rtf" | "vcf" | "ics" | "eml" | "epub" | "odt" | "cdr" | "ttf"
        | "woff" | "chm" | "blend" | "indd" | "php" | "sh" | "djvu" => "Documents",
        "zip" | "ole" | "pst" | "msg" => "Office & Email",
        "rar" | "7z" | "gz" | "xz" | "bz2" | "iso" | "tar" => "Archives",
        "db" | "vmdk" | "evtx" | "exe" | "elf" | "dat" | "vhd" | "vhdx" | "qcow2" | "macho"
        | "kdbx" | "kdb" | "e01" | "pcap" | "dmp" | "plist" | "luks" | "dcm" => "System",
        "ico" => "Images",
        _ => "Other",
    }
}

// ── Signature loading ─────────────────────────────────────────────────────────

/// Build a "Custom" [`SigGroup`] from `sigs`.  Returns `None` if `sigs` is
/// empty (so the caller can skip adding it to `groups`).
///
/// The group starts **expanded** so users immediately see their signatures.
/// Only entries whose `to_signature()` call succeeds are included; broken
/// definitions are silently skipped.
pub(super) fn build_user_sig_group(sigs: &[UserSigDef]) -> Option<SigGroup> {
    let entries: Vec<SigEntry> = sigs
        .iter()
        .filter_map(|def| {
            def.to_signature()
                .ok()
                .map(|sig| SigEntry { sig, enabled: true })
        })
        .collect();
    if entries.is_empty() {
        return None;
    }
    Some(SigGroup {
        label: "Custom",
        expanded: true,
        entries,
    })
}

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

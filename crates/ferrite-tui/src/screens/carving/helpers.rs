//! Formatting, deduplication, and signature-loading helpers for the carving screen.

use ferrite_carver::CarvingConfig;

use super::SigEntry;

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

// ── Signature loading ─────────────────────────────────────────────────────────

pub(super) fn load_builtin_signatures() -> Vec<SigEntry> {
    match CarvingConfig::from_toml_str(crate::SIGNATURES_TOML) {
        Ok(cfg) => cfg
            .signatures
            .into_iter()
            .map(|sig| SigEntry { sig, enabled: true })
            .collect(),
        Err(e) => {
            tracing::error!(?e, "failed to load built-in signatures");
            Vec::new()
        }
    }
}

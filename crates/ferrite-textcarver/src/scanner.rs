//! Core data model for the heuristic text block scanner.

// ── TextKind ──────────────────────────────────────────────────────────────────

/// Classification of a recovered text block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextKind {
    /// PHP script (`<?php` opener).
    Php,
    /// Unix shebang script (`#!/`).  Extension is set by sub-classification in
    /// the classifier (py/rb/pl/js/sh).
    Script,
    /// JSON data (`{` or `[` with `"key":` structure).
    Json,
    /// YAML document (`---` frontmatter or dense `key: value` pairs).
    Yaml,
    /// HTML/XML markup (`<html`, `<!DOCTYPE`, `<?xml`).
    Markup,
    /// SQL source (`SELECT`/`INSERT`/`CREATE`/`UPDATE` keyword density).
    Sql,
    /// C / C++ source (`#include`, `typedef`, `struct` keyword density).
    CSource,
    /// Markdown (`#` headings, `**bold**`, `[link]()` patterns).
    Markdown,
    /// Printable text with no strong classification signal.
    Generic,
}

impl TextKind {
    /// Default file extension for this kind.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Php => "php",
            Self::Script => "sh",
            Self::Json => "json",
            Self::Yaml => "yaml",
            Self::Markup => "html",
            Self::Sql => "sql",
            Self::CSource => "c",
            Self::Markdown => "md",
            Self::Generic => "txt",
        }
    }

    /// Short display label for the TUI filter bar.
    pub fn label(self) -> &'static str {
        match self {
            Self::Php => "php",
            Self::Script => "script",
            Self::Json => "json",
            Self::Yaml => "yaml",
            Self::Markup => "markup",
            Self::Sql => "sql",
            Self::CSource => "csrc",
            Self::Markdown => "md",
            Self::Generic => "txt",
        }
    }

    /// All variants in display order (used for filter key mapping 1–8).
    pub fn all() -> &'static [TextKind] {
        &[
            Self::Php,
            Self::Script,
            Self::Json,
            Self::Yaml,
            Self::Markup,
            Self::Sql,
            Self::CSource,
            Self::Markdown,
            Self::Generic,
        ]
    }
}

// ── TextBlock ─────────────────────────────────────────────────────────────────

/// A recovered contiguous text region from the raw device stream.
#[derive(Debug, Clone)]
pub struct TextBlock {
    /// Absolute byte offset of the block start on the device.
    pub byte_offset: u64,
    /// Length in bytes.
    pub length: u64,
    /// Content classification.
    pub kind: TextKind,
    /// File extension — may differ from `kind.extension()` for `Script` blocks
    /// whose shebang line was sub-classified (e.g. `.py`, `.rb`).
    pub extension: &'static str,
    /// Classifier confidence, 0–100.
    pub confidence: u8,
    /// Fraction of bytes in the ASCII printable range, 0–100.
    pub quality: u8,
    /// First ≤ 80 chars of the block; newlines replaced with `↵`.
    pub preview: String,
}

// ── Config ────────────────────────────────────────────────────────────────────

/// Configuration for a text scan run.
pub struct TextScanConfig {
    /// Minimum block size in bytes to emit (default: 256).
    pub min_block_bytes: u64,
    /// Maximum block size in bytes; larger blocks are split at this boundary
    /// (default: 1 MiB).
    pub max_block_bytes: u64,
    /// Maximum consecutive non-text bytes tolerated before ending a block
    /// (default: 8).
    pub gap_tolerance_bytes: usize,
    /// Minimum fraction of printable ASCII bytes for a block to pass the
    /// quality gate, 0–100 (default: 80).
    pub min_printable_pct: u8,
    /// Bytes per device read (default: 1 MiB).
    pub chunk_bytes: u64,
    /// Bytes of overlap between consecutive chunks to catch blocks that straddle
    /// a chunk boundary (default: 4 KiB).
    pub overlap_bytes: usize,
}

impl Default for TextScanConfig {
    fn default() -> Self {
        Self {
            min_block_bytes: 256,
            max_block_bytes: 1_048_576,
            gap_tolerance_bytes: 8,
            min_printable_pct: 80,
            chunk_bytes: 1_048_576,
            overlap_bytes: 4096,
        }
    }
}

// ── Progress & messages ───────────────────────────────────────────────────────

/// Snapshot of scan progress sent periodically to the TUI.
#[derive(Debug, Clone)]
pub struct TextScanProgress {
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub blocks_found: usize,
}

/// Messages streamed from the background scan thread to the TUI.
pub enum TextScanMsg {
    /// A batch of newly completed text blocks.
    BlockBatch(Vec<TextBlock>),
    /// Periodic progress update.
    Progress(TextScanProgress),
    /// Scan completed successfully.
    Done { total_blocks: usize },
    /// Scan failed with an error message.
    Error(String),
}

//! Core trait and types for the artifact scanner.

use std::fmt;

// ── ArtifactKind ──────────────────────────────────────────────────────────────

/// Category of forensic artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArtifactKind {
    Email,
    Url,
    CreditCard,
    Iban,
    WindowsPath,
    Ssn,
}

impl ArtifactKind {
    /// Human-readable label used in the TUI and CSV output.
    pub fn label(self) -> &'static str {
        match self {
            ArtifactKind::Email => "Email",
            ArtifactKind::Url => "URL",
            ArtifactKind::CreditCard => "Credit Card",
            ArtifactKind::Iban => "IBAN",
            ArtifactKind::WindowsPath => "Windows Path",
            ArtifactKind::Ssn => "SSN",
        }
    }

    /// Short uppercase code used in the TUI hit-list column.
    pub fn short_label(self) -> &'static str {
        match self {
            ArtifactKind::Email => "EMAIL",
            ArtifactKind::Url => "URL  ",
            ArtifactKind::CreditCard => "CC   ",
            ArtifactKind::Iban => "IBAN ",
            ArtifactKind::WindowsPath => "PATH ",
            ArtifactKind::Ssn => "SSN  ",
        }
    }

    /// Ordered list of all variants (used for filter cycling and CSV header).
    pub fn all() -> &'static [ArtifactKind] {
        &[
            ArtifactKind::Email,
            ArtifactKind::Url,
            ArtifactKind::CreditCard,
            ArtifactKind::Iban,
            ArtifactKind::WindowsPath,
            ArtifactKind::Ssn,
        ]
    }
}

impl fmt::Display for ArtifactKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

// ── Confidence ────────────────────────────────────────────────────────────────

/// Confidence level for a forensic artifact match.
///
/// Assigned per-hit by each scanner based on secondary validation.  A `Low`
/// result indicates likely false-positive and is hidden by the default TUI
/// filter.  `High` means the match passed all available structural checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Confidence {
    /// Likely a false positive (e.g. Luhn-valid digits in binary context, or
    /// a placeholder email domain such as `@example.com`).
    Low,
    /// Reasonable match — passes format validation but context is ambiguous.
    Medium,
    /// Strong match — passes all format and context validation checks.
    High,
}

impl Confidence {
    pub fn label(self) -> &'static str {
        match self {
            Confidence::Low => "Low",
            Confidence::Medium => "Med",
            Confidence::High => "High",
        }
    }
}

impl std::fmt::Display for Confidence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

// ── ArtifactHit ───────────────────────────────────────────────────────────────

/// A single forensic artifact found at a specific byte offset.
#[derive(Debug, Clone)]
pub struct ArtifactHit {
    pub kind: ArtifactKind,
    /// Byte offset within the device/image where the artifact was found.
    pub byte_offset: u64,
    /// The matched value.
    ///
    /// Credit card numbers are **masked** — only the last 4 digits are stored
    /// (e.g. `****-****-****-1234`).  No raw CC numbers are retained.
    pub value: String,
    /// Confidence level assigned by the scanner.
    pub confidence: Confidence,
}

// ── ArtifactScanner trait ─────────────────────────────────────────────────────

/// Scans a raw byte slice for a specific class of forensic artifact.
///
/// Implementations must be `Send + Sync` so the engine can box them and share
/// across thread boundaries.
pub trait ArtifactScanner: Send + Sync {
    fn kind(&self) -> ArtifactKind;

    /// Scan `data` (which starts at `block_offset` within the device) and
    /// return every artifact hit found.
    fn scan_block(&self, data: &[u8], block_offset: u64) -> Vec<ArtifactHit>;
}

// ── Shared helper ─────────────────────────────────────────────────────────────

/// Convert a raw byte slice to UTF-8 (lossy) and run a regex over it.
///
/// Matches containing the UTF-8 replacement character (`\u{FFFD}`) are dropped
/// to avoid reporting garbage matches at binary boundaries.
///
/// `transform` maps the raw match string to `(value, confidence)`.  Return
/// `None` to discard a match (e.g. when secondary validation like Luhn fails).
pub fn scan_text_lossy<F>(
    data: &[u8],
    block_offset: u64,
    kind: ArtifactKind,
    re: &regex::Regex,
    transform: F,
) -> Vec<ArtifactHit>
where
    F: Fn(&str) -> Option<(String, Confidence)>,
{
    let text = String::from_utf8_lossy(data);
    re.find_iter(text.as_ref())
        .filter(|m| !m.as_str().contains('\u{FFFD}'))
        .filter_map(|m| {
            transform(m.as_str()).map(|(value, confidence)| ArtifactHit {
                kind,
                byte_offset: block_offset + m.start() as u64,
                value,
                confidence,
            })
        })
        .collect()
}

# Phase 57 — Forensic Artifact Scanner

## Summary
Added a new `ferrite-artifact` crate implementing a regex-based PII artifact scanner
(email, URL, credit card, IBAN, Windows path, SSN) and wired it into the TUI as Tab 8
("Artifacts"). The scanner streams over the raw block device, produces `ArtifactHit`
records, and supports CSV export.

## New Files

### `crates/ferrite-artifact/`
| File | Purpose |
|------|---------|
| `Cargo.toml` | Crate manifest (deps: ferrite-blockdev, regex, tracing, thiserror) |
| `src/lib.rs` | Public re-exports |
| `src/scanner.rs` | `ArtifactKind`, `ArtifactHit`, `ArtifactScanner` trait, `scan_text_lossy` helper |
| `src/engine.rs` | `ArtifactScanConfig`, `ScanProgress`, `ScanMsg`, `run_scan()`, overlap buffer, per-kind dedup |
| `src/export.rs` | `write_csv(path, hits)` — RFC 4180 CSV with quote-escaping |
| `src/scanners/mod.rs` | Re-exports all 6 scanner modules |
| `src/scanners/email.rs` | `EmailScanner` — RFC 5321-ish regex |
| `src/scanners/url.rs` | `UrlScanner` — http/https, trailing punctuation trimming |
| `src/scanners/credit_card.rs` | `CreditCardScanner` — Luhn check, last-4 masking |
| `src/scanners/iban.rs` | `IbanScanner` — mod-97 checksum validation |
| `src/scanners/win_path.rs` | `WinPathScanner` — `C:\...` Windows path pattern |
| `src/scanners/ssn.rs` | `SsnScanner` — US SSN format, filters invalid area/group/serial |

### `crates/ferrite-tui/src/screens/artifacts/`
| File | Purpose |
|------|---------|
| `mod.rs` | `ArtifactsState`, `ScanStatus`, `tick()`, `start_scan()`, `cancel_scan()`, `export_csv()`, `rebuild_filtered()` |
| `input.rs` | Key handler: consent dialog, output dir editing, navigation, s/c/e/o/0-6 |
| `render.rs` | Full render: output bar, progress gauge, hit list, status bar, consent overlay |

## Modified Files

| File | Change |
|------|--------|
| `Cargo.toml` (workspace) | Added `ferrite-artifact` crate member + workspace dep + `regex = "1"` |
| `crates/ferrite-artifact/Cargo.toml` | Added `tempfile` dev-dependency for export tests |
| `crates/ferrite-tui/Cargo.toml` | Added `ferrite-artifact` and `toml` workspace deps |
| `crates/ferrite-tui/src/screens/mod.rs` | Added `pub mod artifacts` |
| `crates/ferrite-tui/src/app.rs` | SCREEN_NAMES 8→9, ArtifactsState wired into App (tick/render/handle_key/set_device) |

## Design Decisions
- **`self.rx.take()` pattern** in `tick()` — avoids borrow checker conflict when mutating `self` inside the drain loop
- **`OnceLock<Regex>`** in each scanner — zero-cost static regex init, compiled once
- **Overlap buffer** in `engine.rs` — 4 KiB tail of previous chunk prepended to next, catching cross-boundary hits
- **Per-kind `HashSet<String>` dedup** — prevents duplicate hits for the same value found in adjacent sectors
- **Sector-aligned reads** via `AlignedBuffer` — same pattern as `ferrite-carver`'s `read_bytes_clamped`
- **Consent dialog** — must be acknowledged once per session before scan starts
- **CC masking** — only last 4 digits stored (`****-****-****-XXXX`), raw number never in memory

## Test Count
- 517 tests total (up from 458) — 59 new tests across ferrite-artifact and ferrite-tui
- `cargo clippy --workspace -- -D warnings` — clean
- `cargo fmt --check` — clean

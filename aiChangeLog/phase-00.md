# Phase 0: Project Scaffolding

**Date:** 2026-03-09
**Status:** Complete

## Summary

Established the full Cargo workspace structure, mandatory folders, docs, CI pipeline,
and the `ferrite-core` foundation crate.

## Tasks Completed

1. **Rust toolchain verified** — `stable-x86_64-pc-windows-msvc` active. `x86_64-unknown-linux-gnu` target added for CI cross-checks.
2. **Git initialized** — `git init`, remote set to `https://github.com/arozz7/ferrite.git`.
3. **`.gitignore`** — Rust template (target/, Cargo.lock, *.pdb).
4. **Workspace `Cargo.toml`** — resolver 2, `[workspace.dependencies]` for all shared deps. Binary entry `src/main.rs → ferrite`.
5. **`crates/ferrite-core/`** — `lib.rs`, `error.rs` (CoreError), `types.rs` (Sector, SectorRange, ByteSize, DeviceInfo), `config.rs` (Config).
6. **`src/main.rs`** — Minimal binary with tracing init.
7. **Mandatory folders** — `aiChangeLog/`, `scripts/release/`, `scripts/deploy/`, `tests/fixtures/`, `tests/integration/`.
8. **`config/signatures.toml`** — File carving signature database (10 types: JPEG, PNG, PDF, ZIP, GIF, BMP, MP3, MP4, RAR, 7z).
9. **`config/smart_thresholds.toml`** — S.M.A.R.T. verdict thresholds (temperature, reallocated sectors, pending sectors, uncorrectable, spin-up time).
10. **`README.md`** — Project overview, crate table, build/test instructions.
11. **`CLAUDE.md`** (project-level) — Crate naming, architecture rules, phase order, test requirements.
12. **`docs/architecture.md`** — Crate dependency diagram, three-layer architecture, key traits, mapfile format, platform matrix.
13. **`docs/adr/ADR-001-pure-rust.md`** — Decision record: no FFI with C libraries.
14. **`docs/adr/ADR-002-tui-first.md`** — Decision record: ratatui TUI before GUI.
15. **`.github/workflows/ci.yml`** — GitHub Actions: fmt check, clippy (-D warnings), test, cross-check (Windows target).

## Crate Naming Decision

Prefix changed from `dr-` to `ferrite-` per user preference.
Binary name: `ferrite`.
License: MIT OR Apache-2.0 (Rust ecosystem dual license).

## File Mapping

```
NEW  Cargo.toml
NEW  src/main.rs
NEW  crates/ferrite-core/Cargo.toml
NEW  crates/ferrite-core/src/lib.rs
NEW  crates/ferrite-core/src/error.rs
NEW  crates/ferrite-core/src/types.rs
NEW  crates/ferrite-core/src/config.rs
NEW  .gitignore
NEW  README.md
NEW  CLAUDE.md
NEW  docs/architecture.md
NEW  docs/adr/ADR-001-pure-rust.md
NEW  docs/adr/ADR-002-tui-first.md
NEW  config/signatures.toml
NEW  config/smart_thresholds.toml
NEW  .github/workflows/ci.yml
NEW  aiChangeLog/phase-00.md
NEW  aiChangeLog/.gitkeep
NEW  scripts/release/.gitkeep
NEW  scripts/deploy/.gitkeep
NEW  tests/fixtures/.gitkeep
NEW  tests/integration/.gitkeep
```

## Verification

- `cargo test --workspace` — pass (ferrite-core unit tests: SectorRange, ByteSize)
- `cargo clippy --workspace -- -D warnings` — pass
- `cargo fmt --check` — pass

# ADR-001: Pure Rust — No FFI with C Libraries

**Status:** Accepted
**Date:** 2026-03-09

## Context

The research phase identified several mature C libraries for the required functionality:
- `libtsk` (The Sleuth Kit) for filesystem analysis
- `libparted` for partition table manipulation
- `libsmartmon` for S.M.A.R.T. access
- Scalpel for file carving

FFI bindings exist (e.g., `sleuthkit-rs`), so the option is technically viable.

## Decision

Implement all functionality in pure Rust. No FFI with C libraries.
For S.M.A.R.T., use `smartctl` as a CLI subprocess (JSON output) rather than `libsmartmon` FFI.

## Rationale

1. **Memory safety** — Rust's core value proposition is memory safety. FFI with C nullifies this guarantee at the boundary and requires `unsafe` blocks throughout the I/O layer.
2. **Cross-platform build complexity** — linking against C libraries (especially on Windows) requires native toolchains, pkg-config shims, vcpkg or Conan, and non-trivial CI setup. Pure Rust builds with `cargo build` on all targets.
3. **Dependency auditing** — Cargo's lockfile and `cargo audit` provide complete supply-chain visibility. External C libraries are opaque blobs from a Rust security perspective.
4. **Maintainability** — Pure Rust codebases are easier to understand, modify, and test than mixed FFI codebases.
5. **Correctness** — Writing minimum viable parsers in Rust allows precise control over error handling and recovery from malformed structures — critical for a recovery tool operating on corrupt data.

## Consequences

- Higher initial implementation effort for filesystem parsers.
- S.M.A.R.T. requires `smartctl` to be installed (graceful degradation when absent).
- No access to decades of C library battle-testing — mitigated by extensive test fixtures.
- Full cross-platform builds with zero native dependencies beyond Rust itself.

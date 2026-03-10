# Ferrite

Autonomous storage diagnostics and data recovery — built in pure Rust.

## Overview

Ferrite recovers data from failing drives through five operational phases:

1. **S.M.A.R.T. Diagnostics** — Health assessment before touching the drive
2. **Resilient Disk Imaging** — ddrescue-style multi-pass imaging with mapfile resume
3. **Partition Recovery** — MBR/GPT parsing, corrupt table reconstruction
4. **Filesystem Analysis** — NTFS, FAT32, ext4 — live and deleted file enumeration
5. **File Carving** — Signature-based recovery from raw sectors

## Design Principles

- **Pure Rust** — no FFI with C libraries; memory-safe throughout
- **TUI first** — `ratatui` terminal UI, usable in recovery environments without a desktop
- **Platform abstracted** — runs on Windows 11 and Linux via `BlockDevice` trait
- **ddrescue-compatible mapfiles** — interoperable with GNU ddrescue
- **Non-destructive** — read-only access to source drives at all times

## Workspace Crates

| Crate | Responsibility |
|---|---|
| `ferrite-core` | Core types, errors, configuration |
| `ferrite-blockdev` | Platform-abstracted block device I/O |
| `ferrite-imaging` | Multi-pass imaging engine |
| `ferrite-smart` | S.M.A.R.T. diagnostics via smartctl |
| `ferrite-partition` | MBR/GPT parsing and recovery |
| `ferrite-filesystem` | NTFS / FAT32 / ext4 metadata parsing |
| `ferrite-carver` | Signature-based file carving |
| `ferrite-tui` | ratatui terminal interface |

## Prerequisites

- Rust stable (1.75+): [rustup.rs](https://rustup.rs)
- `smartctl` from [smartmontools](https://www.smartmontools.org/) (for S.M.A.R.T. features)
- Windows: run as Administrator for raw device access
- Linux: run as root or with `CAP_SYS_RAWIO` for raw device access

## Build

```bash
cargo build --release
```

## Run

```bash
cargo run --release
```

## Test

```bash
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --check
```

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE) at your option.

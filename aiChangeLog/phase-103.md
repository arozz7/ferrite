# Phase 103 — Forensic & System Format Signatures (129 → 139)

**Date:** 2026-03-25
**Branch:** master
**Status:** Complete

## Summary

Added 10 new file-carving signatures targeting forensic and Windows system formats:
VirtualBox VDI disk images, AFF forensic images, Windows LNK shortcuts, Prefetch
files (4 versions), legacy EVT event logs, PEM certificates/keys, and Bitcoin
wallets.  Signature count: **129 → 139**.

## New Signatures

| # | Name | Header | Pre-validate | Max size |
|---|------|--------|--------------|----------|
| 130 | VirtualBox VDI | `7F 10 DA BE` @offset 64 | `Vdi`: image type @8 ∈ {1–4} | 2 GiB |
| 131 | AFF Forensic Image | `AFF\0\0\0\1` (7 B) | — (7-byte unique) | 2 GiB |
| 132 | Windows LNK | 20-byte HeaderSize+CLSID | `Lnk`: FileAttributes non-zero, no reserved bits | 1 MiB |
| 133 | Prefetch WinXP | `11 00 00 00` | `Prefetch`: "SCCA" @4 | 10 MiB |
| 134 | Prefetch Vista/7 | `17 00 00 00` | `Prefetch` (shared) | 10 MiB |
| 135 | Prefetch Win8.1 | `1A 00 00 00` | `Prefetch` (shared) | 10 MiB |
| 136 | Prefetch Win10/11 | `1E 00 00 00` | `Prefetch` (shared) | 10 MiB |
| 137 | Windows EVT | `30 00 00 00 4C 66 4C 65` | `Evt`: MajorVersion==1, MinorVersion==1 | 100 MiB |
| 138 | PEM Certificate/Key | `-----BEGIN` (10 B) | `Pem`: space @10; uppercase @11 | 1 MiB |
| 139 | Bitcoin Wallet (BDB) | `62 31 05 00 09 00` | — (6-byte magic) | 100 MiB |

## New Pre-validators

| Variant | Logic |
|---------|-------|
| `Vdi` | image_type = u32 LE @pos+8; must be in 1..=4 |
| `Lnk` | FileAttributes = u32 LE @pos+24; != 0 AND & 0xFFFF_0000 == 0 |
| `Prefetch` | data[pos+4..pos+8] == b"SCCA" |
| `Evt` | MajorVersion @8 == 1 AND MinorVersion @12 == 1 |
| `Pem` | data[pos+10] == ' ' AND data[pos+11].is_ascii_uppercase() |

## TUI Group Assignments

- `vdi`, `lnk`, `pf`, `evt`, `wallet` → **System**
- `aff` → **Archives**
- `pem` → **Documents**

## Files Changed

| File | Change |
|------|--------|
| `config/signatures.toml` | +10 `[[signature]]` entries |
| `crates/ferrite-carver/src/pre_validate.rs` | +5 enum variants, +5 validators, +23 unit tests |
| `crates/ferrite-tui/src/screens/carving/helpers.rs` | sig_group_label: added vdi/aff/lnk/pf/evt/pem/wallet |
| `crates/ferrite-carver/src/lib.rs` | assertion 129 → 139 |
| `crates/ferrite-tui/src/screens/carving/mod.rs` | assertion 129 → 139 |
| `aiChangeLog/phase-103.md` | This file |

## Test Results

- **629 tests** in ferrite-carver (up from 606; +23 new validator tests)
- All workspace tests pass; clippy clean with `-D warnings`

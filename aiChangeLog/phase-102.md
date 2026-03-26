# Phase 102 — Developer & Science Format Signatures (115 → 129)

**Date:** 2026-03-25
**Branch:** master
**Status:** Complete

## Summary

Added 14 new file-carving signatures covering developer workstation and scientific
data formats: JAR, Python bytecode (7 versions), LZH/LHA, HDF5, FITS, Parquet,
and DPX (2 endian variants).  Signature count: **115 → 129**.

## New Signatures

| # | Name | Header | Pre-validate | Max size |
|---|------|--------|--------------|----------|
| 116 | Java Archive (JAR) | `PK\x03\x04` | `Jar`: first entry starts with `META-INF` | 500 MiB |
| 117 | Python 3.6 bytecode | `33 0D 0D 0A` | — (4-byte unique) | 100 MiB |
| 118 | Python 3.7 bytecode | `42 0D 0D 0A` | — (4-byte unique) | 100 MiB |
| 119 | Python 3.8 bytecode | `55 0D 0D 0A` | — (4-byte unique) | 100 MiB |
| 120 | Python 3.9 bytecode | `61 0D 0D 0A` | — (4-byte unique) | 100 MiB |
| 121 | Python 3.10 bytecode | `6F 0D 0D 0A` | — (4-byte unique) | 100 MiB |
| 122 | Python 3.11 bytecode | `A7 0D 0D 0A` | — (4-byte unique) | 100 MiB |
| 123 | Python 3.12 bytecode | `CB 0D 0D 0A` | — (4-byte unique) | 100 MiB |
| 124 | LZH/LHA Archive | `-lh?-` @offset 2 | `Lzh`: method ∈ {'0'–'7','d','s'} | 200 MiB |
| 125 | HDF5 Scientific Data | `\x89HDF\r\n\x1a\n` | `Hdf5`: superblock version ≤ 3 | 2 GiB |
| 126 | FITS Astronomy Image | `SIMPLE  =` | `Fits`: byte@9=space; byte@29='T' | 2 GiB |
| 127 | Apache Parquet | `PAR1` | — (4-byte unique) | 2 GiB |
| 128 | DPX Film (Big-Endian) | `SDPX` | — (4-byte unique) | 2 GiB |
| 129 | DPX Film (Little-Endian) | `XPDS` | — (4-byte unique) | 2 GiB |

## New Pre-validators

| Variant | Logic |
|---------|-------|
| `Jar` | fname_len ∈ [1, 256]; data[pos+30..pos+30+fname_len] starts with `META-INF` |
| `Lzh` | method char data[pos+3] ∈ {'0'..'7', 'd', 's'} |
| `Hdf5` | superblock version data[pos+8] ≤ 3 |
| `Fits` | value indicator data[pos+9] == ' '; logical value data[pos+29] == 'T' |

## TUI Group Assignments

- `jar`, `lzh` → **Archives**
- `pyc`, `h5`, `fits`, `parquet` → **System**
- `dpx` → **Images**

## Files Changed

| File | Change |
|------|--------|
| `config/signatures.toml` | +14 `[[signature]]` entries |
| `crates/ferrite-carver/src/pre_validate.rs` | +4 enum variants, +4 validators, +19 unit tests |
| `crates/ferrite-tui/src/screens/carving/helpers.rs` | sig_group_label: added jar/lzh/pyc/h5/fits/parquet/dpx |
| `crates/ferrite-carver/src/lib.rs` | assertion 115 → 129 |
| `crates/ferrite-tui/src/screens/carving/mod.rs` | assertion 115 → 129 |
| `aiChangeLog/phase-102.md` | This file |

## Test Results

- **606 tests** in ferrite-carver (up from 587; +19 new validator tests)
- All workspace tests pass; clippy clean with `-D warnings`

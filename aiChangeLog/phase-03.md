# Phase 3: S.M.A.R.T. Diagnostics

**Date:** 2026-03-10
**Status:** Complete

## Summary

Implemented `ferrite-smart` — a pure-Rust S.M.A.R.T. diagnostics layer that
wraps `smartctl --json -a`, parses its JSON output for both ATA and NVMe
drives, evaluates drive health against configurable TOML thresholds, and
exposes a verdict that can block or warn an imaging run.

## Files Created

```
NEW  crates/ferrite-smart/Cargo.toml
NEW  crates/ferrite-smart/src/lib.rs       — module declarations and public re-exports
NEW  crates/ferrite-smart/src/error.rs     — SmartError, Result<T>
NEW  crates/ferrite-smart/src/types.rs     — SmartAttribute, SmartData, HealthVerdict
NEW  crates/ferrite-smart/src/parser.rs    — serde deserialization of smartctl JSON → SmartData
NEW  crates/ferrite-smart/src/thresholds.rs — SmartThresholds (TOML) + default_config()
NEW  crates/ferrite-smart/src/verdict.rs   — assess_health() logic + 12 unit tests
NEW  crates/ferrite-smart/src/runner.rs    — query() / query_and_assess() subprocess execution
MOD  Cargo.toml                            — added ferrite-smart to members and workspace deps
```

## Key Design Decisions

- **CLI wrapper, not FFI**: `smartctl --json -a` output is parsed via serde_json.
  No libsmartmon binding. The JSON format is stable across platforms.
- **Serde deserialization via private raw structs**: `RawSmartctl` and friends
  capture only the fields Ferrite needs; unknown fields are silently ignored.
  A public `parse(device_path, json) -> Result<SmartData>` converts to domain types.
- **Thresholds from TOML**: `SmartThresholds::from_file(path)` loads
  `config/smart_thresholds.toml`; `default_config()` mirrors the same values
  for use in tests and when no config path is provided.
- **HealthVerdict escalation**: `assess_health()` accumulates warnings and
  criticals separately; criticals absorb warnings so the caller sees a single
  verdict with all reasons. `HealthVerdict::blocks_imaging()` is the gate used
  by the imaging engine to refuse to start on a failing drive.
- **NVMe and ATA coverage**:
  - ATA: overall SMART status, temperature, IDs 5/197/198/3, `when_failed` flags
  - NVMe: `critical_warning` bitmask, `media_errors`, `available_spare` vs threshold
- **Exit code handling in runner**: bits 0–1 of the smartctl exit code indicate
  device open / parse failures (no JSON produced) → `SmartctlError`. Bits 2–3
  (ATA command failure, SMART failing) are logged as warnings but still parsed,
  since smartctl emits valid JSON regardless.

## Verification

- `cargo test --workspace`: 59 tests pass (18 ferrite-smart + 25 ferrite-imaging + 14 ferrite-blockdev + 2 ferrite-core)
- `cargo clippy --workspace --all-targets -- -D warnings`: clean
- `cargo fmt --check`: clean

## Test Coverage (ferrite-smart)

| Module | Tests |
|---|---|
| `parser` | parse_ata_drive, parse_nvme_drive, parse_error_on_invalid_json, missing_fields_use_defaults |
| `thresholds` | default_config_values, parse_toml_inline, from_file_not_found_returns_threshold_config_error |
| `verdict` | healthy_drive, smart_failed_is_critical, high_temperature_warning, critical_temperature, reallocated_sectors_warning, reallocated_sectors_critical, pending_sectors_warning, when_failed_attribute_is_warning, nvme_media_errors_critical, nvme_spare_below_threshold_critical, healthy_label_display |

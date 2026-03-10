# Phase 1: Platform Abstraction & Block Device I/O

**Date:** 2026-03-10
**Status:** Complete

## Summary

Implemented `ferrite-blockdev` — the foundational I/O crate that every other
crate builds on. Defines the `BlockDevice` trait and ships three implementations:
`FileBlockDevice` (cross-platform, for testing/images), `MockBlockDevice` (error
injection for unit tests), `WindowsBlockDevice` (direct I/O via Win32), and
`LinuxBlockDevice` (direct I/O via O_DIRECT + pread64).

## Files Created

```
NEW  crates/ferrite-blockdev/Cargo.toml
NEW  crates/ferrite-blockdev/src/lib.rs       — BlockDevice trait, module re-exports
NEW  crates/ferrite-blockdev/src/aligned.rs   — AlignedBuffer (sector-aligned heap alloc)
NEW  crates/ferrite-blockdev/src/error.rs     — BlockDeviceError, Result<T>
NEW  crates/ferrite-blockdev/src/file.rs      — FileBlockDevice (std::fs::File)
NEW  crates/ferrite-blockdev/src/mock.rs      — MockBlockDevice (in-memory + error injection)
NEW  crates/ferrite-blockdev/src/windows.rs   — WindowsBlockDevice (cfg windows)
NEW  crates/ferrite-blockdev/src/linux.rs     — LinuxBlockDevice (cfg linux)
MOD  Cargo.toml                               — added ferrite-blockdev member + tempfile dep
```

## Key Design Decisions

- **AlignedBuffer** uses `std::alloc::{alloc_zeroed, dealloc}` directly for
  guaranteed power-of-two alignment. No `Vec<u8>` overallocation hack.
- **FileBlockDevice** wraps `Mutex<File>` for interior mutability — seek+read
  is serialised but correct for single-threaded test/image use.
- **MockBlockDevice** uses `RwLock<HashMap<sector, (ErrorPolicy, count)>>` for
  fine-grained error injection. `ErrorPolicy::FailFirstN(n)` enables retry-success
  tests (critical for Phase 2 imaging engine).
- **WindowsBlockDevice**: `CreateFileW` with `FILE_FLAG_NO_BUFFERING`. Positioned
  reads via `ReadFile` + synchronous `OVERLAPPED` (offset in struct, no
  `FILE_FLAG_OVERLAPPED` on handle → call blocks). `DeviceIoControl` for size
  (`IOCTL_DISK_GET_LENGTH_INFO`) and sector size (`IOCTL_DISK_GET_DRIVE_GEOMETRY`).
  Model/serial from `IOCTL_STORAGE_QUERY_PROPERTY`. Own `#[repr(C)]` structs to
  avoid `LARGE_INTEGER` union gymnastics.
- **LinuxBlockDevice**: `O_RDONLY | O_DIRECT | O_LARGEFILE`. `pread64` for
  thread-safe positioned reads. `BLKGETSIZE64` / `BLKSSZGET` ioctls. Model/serial
  from `/sys/block/<dev>/device/{model,serial}`.
- **`integration` feature flag**: gates real-hardware tests (not yet written).

## Verification

- `cargo test --workspace`: 16 tests pass (14 ferrite-blockdev + 2 ferrite-core)
- `cargo clippy --workspace --all-targets -- -D warnings`: clean
- `cargo fmt --check`: clean

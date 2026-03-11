# Phase 05 — ferrite-filesystem

## Summary

Implemented the `ferrite-filesystem` crate: minimum-viable read-only parsers
for FAT32, NTFS, and ext4 behind a common `FilesystemParser` trait.

## New crate

`crates/ferrite-filesystem/` (6 source files, ~950 lines)

| File | Purpose |
|------|---------|
| `src/lib.rs` | `FilesystemType`, `FileEntry`, `FilesystemParser` trait, `detect_filesystem()`, `open_filesystem()` |
| `src/error.rs` | `FilesystemError` (thiserror) + `Result<T>` alias |
| `src/io.rs` | `read_bytes()` — sector-aligned helper wrapping `BlockDevice::read_at` |
| `src/fat32.rs` | `Fat32Parser` — BPB parsing, FAT cluster-chain, LFN support, deleted-entry detection |
| `src/ntfs.rs` | `NtfsParser` — boot sector + MFT scan, FILE record parsing, resident & non-resident DATA (run-list decoder), update-sequence fixup |
| `src/ext4.rs` | `Ext4Parser` — superblock, GDT, inode table, linear directory blocks, direct + single-indirect block pointers |

## Trait API

```rust
pub trait FilesystemParser: Send + Sync {
    fn filesystem_type(&self) -> FilesystemType;
    fn root_directory(&self) -> Result<Vec<FileEntry>>;
    fn list_directory(&self, path: &str) -> Result<Vec<FileEntry>>;
    fn read_file(&self, entry: &FileEntry, writer: &mut dyn Write) -> Result<u64>;
    fn deleted_files(&self) -> Result<Vec<FileEntry>>;
}
```

## Design decisions

- All parsers store `Arc<dyn BlockDevice>` for lazy I/O after construction.
- `detect_filesystem()` probes in order: NTFS (OEM ID at +3), FAT32 (type
  string at +82), ext4 (superblock magic at 1024+56).
- NTFS: MFT record count derived from record 0's non-resident DATA attribute;
  capped at 65 536 records for safety. Fixup applied leniently (mismatch logs
  a warning, does not abort).
- FAT32: LFN sequences accumulated and sorted before the 8.3 entry; dot/dot-dot
  and volume labels filtered automatically.
- ext4: revision 0 always uses 128-byte inodes; revision 1+ reads `s_inode_size`
  from superblock. Only direct + single-indirect blocks supported (files up to
  ~12 MiB with 1 KiB blocks).

## Known limitations (documented)

- NTFS: assumes contiguous MFT if record 0's run list is unavailable.
- ext4: double/triple indirect and extent-tree blocks not yet implemented.
- Timestamps not yet populated in `FileEntry` (always `None`).

## Test counts

| Crate | Tests |
|-------|-------|
| ferrite-filesystem | 18 new |
| Workspace total | 104 |

## Workspace changes

- `Cargo.toml`: added `crates/ferrite-filesystem` to `[workspace.members]` and
  `ferrite-filesystem` to `[workspace.dependencies]`.

# Ferrite — Architecture

## Crate Dependency Diagram

```
ferrite (binary)
├── ferrite-tui          ← ratatui terminal interface
│   ├── ferrite-smart
│   ├── ferrite-imaging
│   ├── ferrite-partition
│   ├── ferrite-filesystem
│   └── ferrite-carver
├── ferrite-imaging      ← ddrescue-style multi-pass engine
│   ├── ferrite-blockdev
│   └── ferrite-core
├── ferrite-smart        ← smartctl CLI wrapper
│   └── ferrite-core
├── ferrite-partition    ← MBR/GPT parsing & recovery
│   ├── ferrite-blockdev
│   └── ferrite-core
├── ferrite-filesystem   ← NTFS/FAT32/ext4 metadata
│   ├── ferrite-blockdev
│   └── ferrite-core
├── ferrite-carver       ← signature-based file carving
│   ├── ferrite-blockdev
│   └── ferrite-core
├── ferrite-blockdev     ← platform-abstracted block I/O
│   └── ferrite-core
└── ferrite-core         ← types, errors, config (no deps)
```

## Three-Layer Architecture

```
┌─────────────────────────────────────────────────┐
│  Reasoning Layer  (pure business logic)          │
│  ferrite-imaging, ferrite-partition,             │
│  ferrite-filesystem, ferrite-carver              │
├─────────────────────────────────────────────────┤
│  Memory Layer  (state & persistence)             │
│  Mapfile (imaging progress), config files        │
├─────────────────────────────────────────────────┤
│  Tools Layer  (side effects)                     │
│  ferrite-blockdev (raw I/O), ferrite-smart       │
│  (smartctl subprocess), ferrite-tui (terminal)  │
└─────────────────────────────────────────────────┘
```

## Key Traits

### `BlockDevice` (ferrite-blockdev)

```rust
pub trait BlockDevice: Send + Sync {
    fn read_at(&self, offset: u64, buf: &mut AlignedBuffer) -> Result<usize>;
    fn size(&self) -> u64;
    fn sector_size(&self) -> u32;
    fn device_info(&self) -> &DeviceInfo;
}
```

Implementations: `WindowsBlockDevice`, `LinuxBlockDevice`, `FileBlockDevice` (testing).

### `FilesystemParser` (ferrite-filesystem)

```rust
pub trait FilesystemParser {
    fn root_directory(&self) -> Result<Vec<FileEntry>>;
    fn list_directory(&self, path: &str) -> Result<Vec<FileEntry>>;
    fn read_file(&self, entry: &FileEntry, writer: &mut dyn Write) -> Result<u64>;
    fn deleted_files(&self) -> Result<Vec<FileEntry>>;
}
```

Implementations: `NtfsParser`, `Fat32Parser`, `Ext4Parser`.

## Mapfile Format

GNU ddrescue-compatible. Block state codes:

| Code | State |
|------|-------|
| `?` | NonTried |
| `*` | NonTrimmed |
| `/` | NonScraped |
| `-` | BadSector |
| `+` | Finished |

Users can start imaging with GNU ddrescue and resume in Ferrite (or vice versa).

## Platform Matrix

| Feature | Windows 11 | Linux |
|---|---|---|
| Block I/O | `CreateFileW` + `FILE_FLAG_NO_BUFFERING` | `O_RDONLY \| O_DIRECT` + `pread` |
| Device enum | WMI / DeviceIoControl | `/proc/partitions` + udev |
| S.M.A.R.T. | `smartctl --json /dev/pdX` | `smartctl --json /dev/sdX` |
| Admin required | Run as Administrator | root or `CAP_SYS_RAWIO` |

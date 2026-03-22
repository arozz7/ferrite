# Ferrite — Architecture

## Crate Dependency Diagram

```
ferrite (binary)
├── ferrite-tui          ← ratatui terminal interface (10 tabs)
│   ├── ferrite-smart
│   ├── ferrite-imaging
│   ├── ferrite-partition
│   ├── ferrite-filesystem
│   ├── ferrite-carver
│   ├── ferrite-artifact
│   ├── ferrite-textcarver
│   └── ferrite-core
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
├── ferrite-carver       ← signature-based file carving (99 sigs)
│   ├── ferrite-blockdev
│   └── ferrite-core
├── ferrite-artifact     ← forensic PII artifact scanner
│   ├── ferrite-blockdev
│   └── ferrite-core
├── ferrite-textcarver   ← heuristic text block scanner
│   ├── ferrite-blockdev
│   └── ferrite-core
├── ferrite-blockdev     ← platform-abstracted block I/O
│   └── ferrite-core
└── ferrite-core         ← types, errors, config, ThermalGuard (no deps)
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

## ThermalGuard (`ferrite-core::thermal`)

All long-running scan engines (imaging, carving, artifact scan, text scan) share a single `ThermalGuard` implementation that protects drives from heat damage during multi-hour operations.

**Two independent signals:**

| Signal | Mechanism | Works with |
|--------|-----------|------------|
| SMART temperature | Polls `ferrite-smart::query` every 60 s | Any SMART-capable drive |
| Speed inference | Monitors a shared `Arc<AtomicU64>` bytes-read counter | All drives including USB bridges without SMART |

**Speed-inference algorithm:**
1. Collect `bytes/sec` samples for 90 s → compute median baseline
2. If rolling rate drops below 50 % of baseline **and stays there for 60 s** → trigger pause (`SpeedThrottle`)
3. Brief bad-sector stalls (≤30 s ERC timeout) never satisfy the 60 s sustain window
4. Resume when rate recovers above threshold, or after `10 × sustain` rest regardless

**Lifecycle:** RAII — guard thread starts on construction, stops within one tick of drop.

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

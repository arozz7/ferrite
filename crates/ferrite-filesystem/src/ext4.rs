//! Minimum-viable ext4 read-only parser.
//!
//! Reads the superblock, block-group descriptor table, inode table, and
//! directory blocks.  Direct blocks, single-indirect blocks, and the extent
//! tree (inode flag `EXT4_INODE_EXTENTS = 0x80000`, introduced in Linux
//! 2.6.23) are all supported.  Deleted-entry detection is based on
//! `de_inode == 0` in directory entries.

use std::io::Write;
use std::sync::Arc;

use tracing::trace;

use ferrite_blockdev::BlockDevice;

use crate::error::{FilesystemError, Result};
use crate::ext4_dir::parse_dir_block;
use crate::io::{read_bytes, read_u16_le, read_u32_le};
use crate::{FileEntry, FilesystemParser, FilesystemType, RecoveryChance};

// â”€â”€ Constants â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

const EXT4_MAGIC: u16 = 0xEF53;
const EXT4_SUPERBLOCK_OFFSET: u64 = 1024;
const EXT4_SUPERBLOCK_SIZE: usize = 1024;

// inode i_mode type bits
const S_IFDIR: u16 = 0x4000;
const S_IFMT: u16 = 0xF000;

// inode i_flags â€” extents tree flag (since Linux 2.6.23)
const EXT4_INODE_EXTENTS: u32 = 0x0008_0000;

// extent tree header magic
const EXT4_EXTENT_MAGIC: u16 = 0xF30A;

// â”€â”€ Public struct â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

/// Read-only ext4 filesystem parser.
pub struct Ext4Parser {
    device: Arc<dyn BlockDevice>,
    block_size: u64,
    inodes_per_group: u32,
    inode_size: u32,
    gdt_offset: u64, // byte offset of first group descriptor
}

impl Ext4Parser {
    /// Parse the ext4 superblock from `device` and return an initialised parser.
    pub fn new(device: Arc<dyn BlockDevice>) -> Result<Self> {
        let sb = read_bytes(
            device.as_ref(),
            EXT4_SUPERBLOCK_OFFSET,
            EXT4_SUPERBLOCK_SIZE,
        )?;

        let magic = read_u16_le(&sb, 56)?;
        if magic != EXT4_MAGIC {
            return Err(FilesystemError::InvalidStructure {
                context: "ext4 superblock",
                reason: format!("magic 0xEF53 not found (got {magic:#06X})"),
            });
        }

        let s_log_block_size = read_u32_le(&sb, 24)?;
        let block_size: u64 = 1024 << s_log_block_size;

        let s_first_data_block = read_u32_le(&sb, 20)? as u64;
        let inodes_per_group = read_u32_le(&sb, 40)?;

        // Revision 0 always uses 128-byte inodes; revision 1+ stores inode_size in sb[88..90].
        let rev_level = read_u32_le(&sb, 92)?;
        let inode_size = if rev_level >= 1 {
            read_u16_le(&sb, 88)? as u32
        } else {
            128
        };

        if block_size == 0 || inodes_per_group == 0 || inode_size == 0 {
            return Err(FilesystemError::InvalidStructure {
                context: "ext4 superblock",
                reason: "block_size, inodes_per_group, or inode_size is zero".to_string(),
            });
        }

        // GDT follows the superblock block.
        // For 1 KiB blocks: superblock is in block 1, GDT starts at block 2.
        // For larger blocks: superblock occupies part of block 0, GDT starts at block 1.
        let gdt_block = s_first_data_block + 1;
        let gdt_offset = gdt_block * block_size;

        trace!(
            block_size,
            inodes_per_group,
            inode_size,
            gdt_offset,
            "ext4 parser initialised"
        );

        Ok(Self {
            device,
            block_size,
            inodes_per_group,
            inode_size,
            gdt_offset,
        })
    }

    // â”€â”€ Low-level helpers â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    fn read_block(&self, block_num: u64) -> Result<Vec<u8>> {
        read_bytes(
            self.device.as_ref(),
            block_num * self.block_size,
            self.block_size as usize,
        )
    }

    /// Return the byte offset of inode `inode_num` (1-indexed) in the device.
    fn inode_offset(&self, inode_num: u32) -> Result<u64> {
        let idx = (inode_num - 1) as u64;
        let group = idx / self.inodes_per_group as u64;
        let local = idx % self.inodes_per_group as u64;

        // Group descriptor is 32 bytes; bg_inode_table is at bytes 8..12.
        let gdt_entry_offset = self.gdt_offset + group * 32;
        let gdt_entry = read_bytes(self.device.as_ref(), gdt_entry_offset, 32)?;
        let inode_table_block = read_u32_le(&gdt_entry, 8)? as u64;

        Ok(inode_table_block * self.block_size + local * self.inode_size as u64)
    }

    fn read_inode(&self, inode_num: u32) -> Result<Vec<u8>> {
        let offset = self.inode_offset(inode_num)?;
        read_bytes(self.device.as_ref(), offset, self.inode_size as usize)
    }

    /// Collect all physical block numbers referenced by an extent-tree node.
    ///
    /// `data` must start at the 12-byte `ext4_extent_header`.  For the
    /// inode root, pass `&inode[40..]` (the 60-byte `i_block` area).  For
    /// non-root nodes, pass the full block returned by `read_block`.
    fn walk_extent_node(&self, data: &[u8]) -> Result<Vec<u64>> {
        if data.len() < 12 {
            return Ok(Vec::new());
        }

        let magic = u16::from_le_bytes([data[0], data[1]]);
        if magic != EXT4_EXTENT_MAGIC {
            return Err(FilesystemError::InvalidStructure {
                context: "ext4 extent header",
                reason: format!("expected magic {EXT4_EXTENT_MAGIC:#06X}, got {magic:#06X}"),
            });
        }

        let eh_entries = u16::from_le_bytes([data[2], data[3]]) as usize;
        let eh_depth = u16::from_le_bytes([data[6], data[7]]);

        let mut blocks = Vec::new();

        if eh_depth == 0 {
            // Leaf node: `ext4_extent` entries, each 12 bytes.
            for i in 0..eh_entries {
                let off = 12 + i * 12;
                if off + 12 > data.len() {
                    break;
                }
                // ee_len high bit = uninitialized extent; mask it out.
                let ee_len = u16::from_le_bytes([data[off + 4], data[off + 5]]);
                let len = (ee_len & 0x7FFF) as u64;
                let ee_start_hi = u16::from_le_bytes([data[off + 6], data[off + 7]]) as u64;
                // Safety: off + 12 <= data.len() checked above.
                let ee_start_lo = u32::from_le_bytes([
                    data[off + 8],
                    data[off + 9],
                    data[off + 10],
                    data[off + 11],
                ]) as u64;
                let phys_block = (ee_start_hi << 32) | ee_start_lo;
                for j in 0..len {
                    blocks.push(phys_block + j);
                }
            }
        } else {
            // Index node: `ext4_extent_idx` entries, each 12 bytes.
            for i in 0..eh_entries {
                let off = 12 + i * 12;
                if off + 12 > data.len() {
                    break;
                }
                // Safety: off + 12 <= data.len() checked above.
                let ei_leaf_lo = u32::from_le_bytes([
                    data[off + 4],
                    data[off + 5],
                    data[off + 6],
                    data[off + 7],
                ]) as u64;
                let ei_leaf_hi = u16::from_le_bytes([data[off + 8], data[off + 9]]) as u64;
                let child_block = (ei_leaf_hi << 32) | ei_leaf_lo;
                let child_data = self.read_block(child_block)?;
                let mut child_blocks = self.walk_extent_node(&child_data)?;
                blocks.append(&mut child_blocks);
            }
        }

        Ok(blocks)
    }

    /// Follow an indirect block and collect all data block numbers (non-zero).
    ///
    /// `depth` controls how many levels of indirection to recurse:
    /// - depth=1: single-indirect (block contains data block numbers)
    /// - depth=2: double-indirect (block contains indirect block numbers)
    /// - depth=3: triple-indirect
    fn collect_indirect_blocks(&self, block_num: u32, depth: u8) -> Result<Vec<u64>> {
        if block_num == 0 {
            return Ok(Vec::new());
        }
        let block_data = self.read_block(block_num as u64)?;
        let ptrs_per_block = block_data.len() / 4;
        let mut blocks = Vec::new();
        for i in 0..ptrs_per_block {
            let n = i * 4;
            // Safety: ptrs_per_block = block_data.len() / 4, so n + 4 <= block_data.len().
            let ptr = u32::from_le_bytes([
                block_data[n],
                block_data[n + 1],
                block_data[n + 2],
                block_data[n + 3],
            ]);
            if ptr == 0 {
                // Sparse block or end of allocated range â€” stop.
                break;
            }
            if depth == 1 {
                blocks.push(ptr as u64);
            } else {
                let mut child = self.collect_indirect_blocks(ptr, depth - 1)?;
                blocks.append(&mut child);
            }
        }
        Ok(blocks)
    }

    /// Return the byte offset of the first data block for `inode_num`, or `None`.
    ///
    /// Works for regular files only; returns `None` for directories, empty
    /// files, or inodes that cannot be read.
    fn first_data_block_byte_offset(&self, inode_num: u32) -> Option<u64> {
        let inode = self.read_inode(inode_num).ok()?;
        if inode.len() < 100 {
            return None;
        }
        // Safety: inode.len() >= 100 checked above.
        let i_flags = u32::from_le_bytes([inode[32], inode[33], inode[34], inode[35]]);
        let uses_extents = i_flags & EXT4_INODE_EXTENTS != 0;

        let first_block = if uses_extents {
            self.walk_extent_node(&inode[40..])
                .ok()?
                .into_iter()
                .next()?
        } else {
            let blk = u32::from_le_bytes([inode[40], inode[41], inode[42], inode[43]]);
            if blk == 0 {
                return None;
            }
            blk as u64
        };

        Some(first_block * self.block_size)
    }

    /// Parse all directory entries from the blocks of directory inode `inode_num`.
    ///
    /// When `include_deleted` is `true`, entries with `de_inode == 0` are included.
    fn list_inode(
        &self,
        inode_num: u32,
        path_prefix: &str,
        include_deleted: bool,
    ) -> Result<Vec<FileEntry>> {
        let inode = self.read_inode(inode_num)?;
        if inode.len() < 100 {
            return Err(FilesystemError::InvalidStructure {
                context: "ext4 inode",
                reason: "inode too small".to_string(),
            });
        }

        // Safety: inode.len() >= 100 checked above.
        let i_mode = u16::from_le_bytes([inode[0], inode[1]]);
        if i_mode & S_IFMT != S_IFDIR {
            return Err(FilesystemError::InvalidStructure {
                context: "ext4 inode",
                reason: format!("inode {inode_num} is not a directory"),
            });
        }

        let i_flags = u32::from_le_bytes([inode[32], inode[33], inode[34], inode[35]]);
        let uses_extents = i_flags & EXT4_INODE_EXTENTS != 0;

        let mut entries = Vec::new();

        if uses_extents {
            // Extent tree: root is stored in the 60-byte i_block area (inode[40..100]).
            let extent_blocks = self.walk_extent_node(&inode[40..])?;
            for blk in extent_blocks {
                let block_data = self.read_block(blk)?;
                let mut block_entries = parse_dir_block(&block_data, path_prefix, include_deleted);
                entries.append(&mut block_entries);
            }
        } else {
            // Legacy block map: i_block[0..11] are direct pointers.
            // Safety: inode.len() >= 100, so indices up to inode[87] are valid.
            for block_idx in 0..12u32 {
                let n = 40 + block_idx as usize * 4;
                let blk = u32::from_le_bytes([inode[n], inode[n + 1], inode[n + 2], inode[n + 3]]);
                if blk == 0 {
                    break;
                }
                let block_data = self.read_block(blk as u64)?;
                let mut block_entries = parse_dir_block(&block_data, path_prefix, include_deleted);
                entries.append(&mut block_entries);
            }

            // Single-indirect block at i_block[12].
            // Safety: inode[88..92] within inode.len() >= 100.
            let indirect_blk = u32::from_le_bytes([inode[88], inode[89], inode[90], inode[91]]);
            if indirect_blk != 0 {
                let indirect_data = self.read_block(indirect_blk as u64)?;
                let mut i = 0;
                while i + 4 <= indirect_data.len() {
                    let blk = u32::from_le_bytes([
                        indirect_data[i],
                        indirect_data[i + 1],
                        indirect_data[i + 2],
                        indirect_data[i + 3],
                    ]);
                    if blk == 0 {
                        break;
                    }
                    let block_data = self.read_block(blk as u64)?;
                    let mut block_entries =
                        parse_dir_block(&block_data, path_prefix, include_deleted);
                    entries.append(&mut block_entries);
                    i += 4;
                }
            }

            // Double-indirect at i_block[13]. Safety: inode[92..96] valid.
            let dbl_blk = u32::from_le_bytes([inode[92], inode[93], inode[94], inode[95]]);
            if dbl_blk != 0 {
                for blk in self.collect_indirect_blocks(dbl_blk, 2)? {
                    let block_data = self.read_block(blk)?;
                    let mut block_entries =
                        parse_dir_block(&block_data, path_prefix, include_deleted);
                    entries.append(&mut block_entries);
                }
            }

            // Triple-indirect at i_block[14]. Safety: inode[96..100] valid.
            let tri_blk = u32::from_le_bytes([inode[96], inode[97], inode[98], inode[99]]);
            if tri_blk != 0 {
                for blk in self.collect_indirect_blocks(tri_blk, 3)? {
                    let block_data = self.read_block(blk)?;
                    let mut block_entries =
                        parse_dir_block(&block_data, path_prefix, include_deleted);
                    entries.append(&mut block_entries);
                }
            }
        }

        // Enrich non-directory, non-deleted entries with their first data block offset.
        for entry in entries.iter_mut() {
            if !entry.is_dir && !entry.is_deleted {
                if let Some(ino) = entry.inode_number {
                    entry.data_byte_offset = self.first_data_block_byte_offset(ino);
                }
            }
        }

        // Populate timestamps from each child's inode.
        //
        // inode layout (ext4, offsets within the inode bytes):
        //   +8  : i_atime  — access time        (u32 Unix seconds)
        //   +12 : i_ctime  — inode-change time   (u32 Unix seconds)
        //   +16 : i_mtime  — modification time   (u32 Unix seconds)
        //   +144: i_crtime — creation time (ext4 extended, only when inode_size >= 148)
        for entry in entries.iter_mut() {
            // Skip entries with a zero or absent inode number.  In ext4 a
            // deleted directory entry has de_inode == 0, and inode_offset()
            // would underflow computing (0 - 1) as u64.
            let ino = match entry.inode_number {
                Some(n) if n > 0 => n,
                _ => continue,
            };
            let inode = match self.read_inode(ino) {
                Ok(i) => i,
                Err(_) => continue,
            };
            if inode.len() >= 20 {
                let mtime = u32::from_le_bytes([inode[16], inode[17], inode[18], inode[19]]);
                entry.modified = (mtime > 0).then_some(mtime as u64);
            }
            // Prefer i_crtime (true creation time) when the extended inode is present.
            let crtime = if inode.len() >= 148 {
                let v = u32::from_le_bytes([inode[144], inode[145], inode[146], inode[147]]);
                (v > 0).then_some(v as u64)
            } else {
                None
            };
            // Fall back to i_ctime (inode-change time) when i_crtime is absent.
            entry.created = crtime.or_else(|| {
                if inode.len() >= 16 {
                    let v = u32::from_le_bytes([inode[12], inode[13], inode[14], inode[15]]);
                    (v > 0).then_some(v as u64)
                } else {
                    None
                }
            });
        }

        Ok(entries)
    }

    /// Resolve a path (e.g. `/etc/passwd`) to the inode number of its parent
    /// directory and the final component name, then return that component's inode.
    fn resolve_path(&self, path: &str) -> Result<u32> {
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let mut current_inode: u32 = 2; // root

        for part in &parts {
            let children = self.list_inode(current_inode, "", false)?;
            let found = children
                .iter()
                .find(|e| e.name.eq_ignore_ascii_case(part))
                .and_then(|e| e.inode_number);
            match found {
                Some(ino) => current_inode = ino,
                None => return Err(FilesystemError::NotFound(path.to_string())),
            }
        }
        Ok(current_inode)
    }
}

// â”€â”€ FilesystemParser impl â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

impl FilesystemParser for Ext4Parser {
    fn filesystem_type(&self) -> FilesystemType {
        FilesystemType::Ext4
    }

    fn root_directory(&self) -> Result<Vec<FileEntry>> {
        self.list_inode(2, "/", false)
    }

    fn list_directory(&self, path: &str) -> Result<Vec<FileEntry>> {
        let inode = self.resolve_path(path)?;
        self.list_inode(inode, path, false)
    }

    fn read_file(&self, entry: &FileEntry, writer: &mut dyn Write) -> Result<u64> {
        let inode_num = entry
            .inode_number
            .ok_or(FilesystemError::InvalidStructure {
                context: "read_file",
                reason: "FileEntry has no inode number".to_string(),
            })?;

        let inode = self.read_inode(inode_num)?;
        if inode.len() < 100 {
            return Err(FilesystemError::InvalidStructure {
                context: "ext4 inode",
                reason: "inode too small".to_string(),
            });
        }

        // Safety: inode.len() >= 100 checked above.
        let file_size = u32::from_le_bytes([inode[4], inode[5], inode[6], inode[7]]) as u64;
        let i_flags = u32::from_le_bytes([inode[32], inode[33], inode[34], inode[35]]);
        let uses_extents = i_flags & EXT4_INODE_EXTENTS != 0;
        let mut written: u64 = 0;

        if uses_extents {
            // Extent tree: root in i_block area (inode[40..100]).
            let extent_blocks = self.walk_extent_node(&inode[40..])?;
            for blk in extent_blocks {
                if written >= file_size {
                    break;
                }
                let data = self.read_block(blk)?;
                let remaining = (file_size - written) as usize;
                let to_write = data.len().min(remaining);
                writer.write_all(&data[..to_write]).map_err(|e| {
                    FilesystemError::InvalidStructure {
                        context: "read_file extent write",
                        reason: e.to_string(),
                    }
                })?;
                written += to_write as u64;
            }
        } else {
            // Legacy block map: direct blocks.
            // Safety: inode.len() >= 100, loop reads up to inode[87].
            for block_idx in 0..12u32 {
                if written >= file_size {
                    break;
                }
                let n = 40 + block_idx as usize * 4;
                let blk = u32::from_le_bytes([inode[n], inode[n + 1], inode[n + 2], inode[n + 3]]);
                if blk == 0 {
                    break;
                }
                let data = self.read_block(blk as u64)?;
                let remaining = (file_size - written) as usize;
                let to_write = data.len().min(remaining);
                writer.write_all(&data[..to_write]).map_err(|e| {
                    FilesystemError::InvalidStructure {
                        context: "read_file write",
                        reason: e.to_string(),
                    }
                })?;
                written += to_write as u64;
            }

            // Single-indirect block. Safety: inode[88..92] within inode.len() >= 100.
            if written < file_size {
                let indirect_blk = u32::from_le_bytes([inode[88], inode[89], inode[90], inode[91]]);
                if indirect_blk != 0 {
                    let indirect_data = self.read_block(indirect_blk as u64)?;
                    let mut i = 0;
                    while i + 4 <= indirect_data.len() {
                        if written >= file_size {
                            break;
                        }
                        let blk = u32::from_le_bytes([
                            indirect_data[i],
                            indirect_data[i + 1],
                            indirect_data[i + 2],
                            indirect_data[i + 3],
                        ]);
                        if blk == 0 {
                            break;
                        }
                        let data = self.read_block(blk as u64)?;
                        let remaining = (file_size - written) as usize;
                        let to_write = data.len().min(remaining);
                        writer.write_all(&data[..to_write]).map_err(|e| {
                            FilesystemError::InvalidStructure {
                                context: "read_file indirect write",
                                reason: e.to_string(),
                            }
                        })?;
                        written += to_write as u64;
                        i += 4;
                    }
                }
            }

            // Double-indirect at i_block[13]. Safety: inode[92..96] valid.
            if written < file_size {
                let dbl_blk = u32::from_le_bytes([inode[92], inode[93], inode[94], inode[95]]);
                if dbl_blk != 0 {
                    for blk in self.collect_indirect_blocks(dbl_blk, 2)? {
                        if written >= file_size {
                            break;
                        }
                        let data = self.read_block(blk)?;
                        let remaining = (file_size - written) as usize;
                        let to_write = data.len().min(remaining);
                        writer.write_all(&data[..to_write]).map_err(|e| {
                            FilesystemError::InvalidStructure {
                                context: "read_file double-indirect write",
                                reason: e.to_string(),
                            }
                        })?;
                        written += to_write as u64;
                    }
                }
            }

            // Triple-indirect at i_block[14]. Safety: inode[96..100] valid.
            if written < file_size {
                let tri_blk = u32::from_le_bytes([inode[96], inode[97], inode[98], inode[99]]);
                if tri_blk != 0 {
                    for blk in self.collect_indirect_blocks(tri_blk, 3)? {
                        if written >= file_size {
                            break;
                        }
                        let data = self.read_block(blk)?;
                        let remaining = (file_size - written) as usize;
                        let to_write = data.len().min(remaining);
                        writer.write_all(&data[..to_write]).map_err(|e| {
                            FilesystemError::InvalidStructure {
                                context: "read_file triple-indirect write",
                                reason: e.to_string(),
                            }
                        })?;
                        written += to_write as u64;
                    }
                }
            }
        }

        Ok(written)
    }

    fn deleted_files(&self) -> Result<Vec<FileEntry>> {
        let all = self.list_inode(2, "/", true)?;
        let mut deleted: Vec<FileEntry> = all.into_iter().filter(|e| e.is_deleted).collect();
        for entry in deleted.iter_mut() {
            entry.recovery_chance = if entry.data_byte_offset.is_some() && entry.size > 0 {
                RecoveryChance::High
            } else if entry.size > 0 {
                RecoveryChance::Low
            } else {
                RecoveryChance::Unknown
            };
        }
        Ok(deleted)
    }
}

#[cfg(test)]
#[path = "ext4_tests.rs"]
mod tests;

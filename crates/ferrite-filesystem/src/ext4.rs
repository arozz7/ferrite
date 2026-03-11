//! Minimum-viable ext4 read-only parser.
//!
//! Reads the superblock, block-group descriptor table, inode table, and
//! directory blocks.  Only direct block pointers are supported (files up to
//! 12 * block_size); single-indirect is also handled.  Deleted-entry detection
//! is based on `de_inode == 0` in directory entries.

use std::io::Write;
use std::sync::Arc;

use tracing::trace;

use ferrite_blockdev::BlockDevice;

use crate::error::{FilesystemError, Result};
use crate::io::read_bytes;
use crate::{FileEntry, FilesystemParser, FilesystemType};

// ── Constants ─────────────────────────────────────────────────────────────────

const EXT4_MAGIC: u16 = 0xEF53;
const EXT4_SUPERBLOCK_OFFSET: u64 = 1024;
const EXT4_SUPERBLOCK_SIZE: usize = 1024;

// inode i_mode type bits
const S_IFDIR: u16 = 0x4000;
const S_IFMT: u16 = 0xF000;

// ── Public struct ─────────────────────────────────────────────────────────────

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

        let magic = u16::from_le_bytes(sb[56..58].try_into().unwrap());
        if magic != EXT4_MAGIC {
            return Err(FilesystemError::InvalidStructure {
                context: "ext4 superblock",
                reason: format!("magic 0xEF53 not found (got {magic:#06X})"),
            });
        }

        let s_log_block_size = u32::from_le_bytes(sb[24..28].try_into().unwrap());
        let block_size: u64 = 1024 << s_log_block_size;

        let s_first_data_block = u32::from_le_bytes(sb[20..24].try_into().unwrap()) as u64;
        let inodes_per_group = u32::from_le_bytes(sb[40..44].try_into().unwrap());

        // Revision 0 always uses 128-byte inodes; revision 1+ stores inode_size in sb[88..90].
        let rev_level = u32::from_le_bytes(sb[92..96].try_into().unwrap());
        let inode_size = if rev_level >= 1 {
            u16::from_le_bytes(sb[88..90].try_into().unwrap()) as u32
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

    // ── Low-level helpers ─────────────────────────────────────────────────────

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
        let inode_table_block = u32::from_le_bytes(gdt_entry[8..12].try_into().unwrap()) as u64;

        Ok(inode_table_block * self.block_size + local * self.inode_size as u64)
    }

    fn read_inode(&self, inode_num: u32) -> Result<Vec<u8>> {
        let offset = self.inode_offset(inode_num)?;
        read_bytes(self.device.as_ref(), offset, self.inode_size as usize)
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

        let i_mode = u16::from_le_bytes(inode[0..2].try_into().unwrap());
        if i_mode & S_IFMT != S_IFDIR {
            return Err(FilesystemError::InvalidStructure {
                context: "ext4 inode",
                reason: format!("inode {inode_num} is not a directory"),
            });
        }

        // i_block[0..11] are direct pointers (at byte offset 40 in the inode).
        let mut entries = Vec::new();
        for block_idx in 0..12u32 {
            let blk = u32::from_le_bytes(
                inode[40 + block_idx as usize * 4..44 + block_idx as usize * 4]
                    .try_into()
                    .unwrap(),
            );
            if blk == 0 {
                break;
            }
            let block_data = self.read_block(blk as u64)?;
            let mut block_entries = parse_dir_block(&block_data, path_prefix, include_deleted);
            entries.append(&mut block_entries);
        }

        // Single-indirect block at i_block[12]
        let indirect_blk = u32::from_le_bytes(inode[40 + 12 * 4..40 + 13 * 4].try_into().unwrap());
        if indirect_blk != 0 {
            let indirect_data = self.read_block(indirect_blk as u64)?;
            for i in (0..indirect_data.len()).step_by(4) {
                let blk = u32::from_le_bytes(indirect_data[i..i + 4].try_into().unwrap());
                if blk == 0 {
                    break;
                }
                let block_data = self.read_block(blk as u64)?;
                let mut block_entries = parse_dir_block(&block_data, path_prefix, include_deleted);
                entries.append(&mut block_entries);
            }
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

// ── FilesystemParser impl ─────────────────────────────────────────────────────

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

        let file_size = u32::from_le_bytes(inode[4..8].try_into().unwrap()) as u64;
        let mut written: u64 = 0;

        // Direct blocks
        for block_idx in 0..12u32 {
            if written >= file_size {
                break;
            }
            let blk = u32::from_le_bytes(
                inode[40 + block_idx as usize * 4..44 + block_idx as usize * 4]
                    .try_into()
                    .unwrap(),
            );
            if blk == 0 {
                break;
            }
            let data = self.read_block(blk as u64)?;
            let remaining = (file_size - written) as usize;
            let to_write = data.len().min(remaining);
            writer
                .write_all(&data[..to_write])
                .map_err(|e| FilesystemError::InvalidStructure {
                    context: "read_file write",
                    reason: e.to_string(),
                })?;
            written += to_write as u64;
        }

        // Single-indirect block
        if written < file_size {
            let indirect_blk =
                u32::from_le_bytes(inode[40 + 12 * 4..40 + 13 * 4].try_into().unwrap());
            if indirect_blk != 0 {
                let indirect_data = self.read_block(indirect_blk as u64)?;
                for i in (0..indirect_data.len()).step_by(4) {
                    if written >= file_size {
                        break;
                    }
                    let blk = u32::from_le_bytes(indirect_data[i..i + 4].try_into().unwrap());
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
                }
            }
        }

        Ok(written)
    }

    fn deleted_files(&self) -> Result<Vec<FileEntry>> {
        let all = self.list_inode(2, "/", true)?;
        Ok(all.into_iter().filter(|e| e.is_deleted).collect())
    }
}

// ── Directory block parser ────────────────────────────────────────────────────

/// Parse linear ext4 directory entries from a single block.
fn parse_dir_block(block: &[u8], path_prefix: &str, include_deleted: bool) -> Vec<FileEntry> {
    let mut entries = Vec::new();
    let mut pos = 0usize;

    while pos + 8 <= block.len() {
        let inode_num = u32::from_le_bytes(block[pos..pos + 4].try_into().unwrap());
        let rec_len = u16::from_le_bytes(block[pos + 4..pos + 6].try_into().unwrap()) as usize;
        let name_len = block[pos + 6] as usize;
        let file_type = block[pos + 7];

        if rec_len == 0 {
            break;
        }

        if name_len > 0 && pos + 8 + name_len <= block.len() {
            let name = String::from_utf8_lossy(&block[pos + 8..pos + 8 + name_len]).to_string();

            // Skip dot entries
            if name != "." && name != ".." {
                let is_deleted = inode_num == 0;
                if !is_deleted || include_deleted {
                    let is_dir = file_type == 2;
                    let path = if path_prefix.is_empty() || path_prefix == "/" {
                        format!("/{name}")
                    } else {
                        format!("{path_prefix}/{name}")
                    };
                    entries.push(FileEntry {
                        name,
                        path,
                        size: 0, // size is in the inode, not the dirent
                        is_dir,
                        is_deleted,
                        created: None,
                        modified: None,
                        first_cluster: None,
                        mft_record: None,
                        inode_number: Some(inode_num),
                    });
                }
            }
        }

        pos += rec_len;
    }

    entries
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ferrite_blockdev::MockBlockDevice;

    use super::*;

    /// Build a minimal ext4 image (8 KiB, 1 KiB blocks).
    ///
    /// Block layout:
    ///  0 (  0– 1023): unused (boot)
    ///  1 (1024–2047): superblock
    ///  2 (2048–3071): group descriptor table
    ///  3 (3072–4095): block bitmap  (unused in tests)
    ///  4 (4096–5119): inode bitmap  (unused in tests)
    ///  5 (5120–6143): inode table   (inodes 1..8, 128 bytes each)
    ///  6 (6144–7167): root directory data
    ///  7 (7168–8191): file data
    fn build_image() -> MockBlockDevice {
        let mut image = vec![0u8; 8192];

        // ── Superblock at offset 1024 ─────────────────────────────────────────
        {
            let sb = &mut image[1024..2048];
            sb[0..4].copy_from_slice(&16u32.to_le_bytes()); // s_inodes_count
            sb[4..8].copy_from_slice(&8u32.to_le_bytes()); // s_blocks_count_lo
            sb[20..24].copy_from_slice(&1u32.to_le_bytes()); // s_first_data_block
            sb[24..28].copy_from_slice(&0u32.to_le_bytes()); // s_log_block_size = 0 (1KiB)
            sb[32..36].copy_from_slice(&8192u32.to_le_bytes()); // s_blocks_per_group
            sb[40..44].copy_from_slice(&16u32.to_le_bytes()); // s_inodes_per_group
            sb[56..58].copy_from_slice(&EXT4_MAGIC.to_le_bytes());
            sb[92..96].copy_from_slice(&0u32.to_le_bytes()); // s_rev_level = 0
        }

        // ── Group descriptor table at offset 2048 ────────────────────────────
        {
            let gdt = &mut image[2048..2080];
            gdt[0..4].copy_from_slice(&3u32.to_le_bytes()); // bg_block_bitmap
            gdt[4..8].copy_from_slice(&4u32.to_le_bytes()); // bg_inode_bitmap
            gdt[8..12].copy_from_slice(&5u32.to_le_bytes()); // bg_inode_table = block 5
        }

        // ── Inode table at block 5 (offset 5120) ─────────────────────────────
        // Inode 1: unused (all zeros)
        // Inode 2: root directory
        {
            let off = 5120 + 128; // inode 2 = index 1
            let ino = &mut image[off..off + 128];
            ino[0..2].copy_from_slice(&(S_IFDIR | 0x1ED).to_le_bytes()); // drwxr-xr-x
            ino[4..8].copy_from_slice(&1024u32.to_le_bytes()); // i_size_lo
            ino[26..28].copy_from_slice(&2u16.to_le_bytes()); // i_links_count
            ino[40..44].copy_from_slice(&6u32.to_le_bytes()); // i_block[0] = block 6
        }
        // Inode 3: regular file "test.txt"
        {
            let off = 5120 + 256; // inode 3 = index 2
            let ino = &mut image[off..off + 128];
            ino[0..2].copy_from_slice(&(0x8000u16 | 0x1A4).to_le_bytes()); // -rw-r--r--
            ino[4..8].copy_from_slice(&13u32.to_le_bytes()); // i_size_lo
            ino[26..28].copy_from_slice(&1u16.to_le_bytes()); // i_links_count
            ino[40..44].copy_from_slice(&7u32.to_le_bytes()); // i_block[0] = block 7
        }
        // Inode 4: deleted file (will have inode=0 in the dirent)

        // ── Root directory data at block 6 (offset 6144) ─────────────────────
        {
            let dir = &mut image[6144..7168];
            // Entry: "." → inode 2
            dir[0..4].copy_from_slice(&2u32.to_le_bytes());
            dir[4..6].copy_from_slice(&12u16.to_le_bytes());
            dir[6] = 1;
            dir[7] = 2; // directory
            dir[8] = b'.';
            // Entry: ".." → inode 2
            dir[12..16].copy_from_slice(&2u32.to_le_bytes());
            dir[16..18].copy_from_slice(&12u16.to_le_bytes());
            dir[18] = 2;
            dir[19] = 2;
            dir[20..22].copy_from_slice(b"..");
            // Entry: "test.txt" → inode 3
            dir[24..28].copy_from_slice(&3u32.to_le_bytes());
            dir[28..30].copy_from_slice(&20u16.to_le_bytes()); // rec_len = 20
            dir[30] = 8; // name_len
            dir[31] = 1; // regular file
            dir[32..40].copy_from_slice(b"test.txt");
            // Entry: deleted "lost.dat" → inode 0
            dir[44..48].copy_from_slice(&0u32.to_le_bytes()); // inode = 0 → deleted
            dir[48..50].copy_from_slice(&(1024u16 - 44).to_le_bytes()); // rec_len = rest
            dir[50] = 8; // name_len
            dir[51] = 1; // regular file
            dir[52..60].copy_from_slice(b"lost.dat");
        }

        // ── File data at block 7 (offset 7168) ───────────────────────────────
        image[7168..7181].copy_from_slice(b"Hello, World!");

        MockBlockDevice::new(image, 512)
    }

    #[test]
    fn detects_ext4() {
        let dev = Arc::new(build_image());
        let parser = Ext4Parser::new(dev).unwrap();
        assert_eq!(parser.filesystem_type(), FilesystemType::Ext4);
    }

    #[test]
    fn root_directory_lists_file() {
        let dev = Arc::new(build_image());
        let parser = Ext4Parser::new(dev).unwrap();
        let entries = parser.root_directory().unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(
            names.contains(&"test.txt"),
            "expected 'test.txt' in root, got: {names:?}"
        );
        assert!(!names.contains(&"."), "dot entries should be skipped");
    }

    #[test]
    fn read_file_returns_content() {
        let dev = Arc::new(build_image());
        let parser = Ext4Parser::new(dev).unwrap();
        let entries = parser.root_directory().unwrap();
        let file = entries
            .iter()
            .find(|e| e.name == "test.txt")
            .expect("test.txt not found");
        let mut buf = Vec::new();
        let written = parser.read_file(file, &mut buf).unwrap();
        assert_eq!(written, 13);
        assert_eq!(&buf, b"Hello, World!");
    }

    #[test]
    fn deleted_files_detected() {
        let dev = Arc::new(build_image());
        let parser = Ext4Parser::new(dev).unwrap();
        let deleted = parser.deleted_files().unwrap();
        assert_eq!(deleted.len(), 1);
        assert_eq!(deleted[0].name, "lost.dat");
        assert!(deleted[0].is_deleted);
    }

    #[test]
    fn rejects_non_ext4() {
        let dev = Arc::new(MockBlockDevice::zeroed(2048, 512));
        assert!(Ext4Parser::new(dev).is_err());
    }
}

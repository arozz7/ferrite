//! Minimum-viable APFS read-only parser (MVP).
//!
//! Parses container superblock → object map B-tree → volume superblock →
//! volume object map → file system B-tree (inodes + directory records +
//! file extents).
//!
//! # Limitations
//! - Single volume only (nx_fs_oid[0])
//! - No encryption support (crypto_id fields are ignored)
//! - No snapshot support
//! - `deleted_files()` always returns empty (APFS reclaims deleted inodes
//!   immediately — undelete requires journal replay, which is out of scope)
//! - Sub-directory listing resolves only the first level of path components

use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;

use tracing::trace;

use ferrite_blockdev::BlockDevice;

use crate::error::{FilesystemError, Result};
use crate::io::{read_bytes, read_u32_le, read_u64_le};
use crate::{FileEntry, FilesystemParser, FilesystemType, RecoveryChance};

// ── Constants ─────────────────────────────────────────────────────────────────

/// Container superblock magic ("NXSB" stored as LE u32).
const NX_MAGIC: u32 = 0x4253_584E;
/// Volume superblock magic ("APSB" stored as LE u32).
const APFS_MAGIC: u32 = 0x4253_5041;

// Container superblock (nx_superblock_t) field byte offsets.
const NX_MAGIC_OFF: usize = 32;
const NX_BLOCK_SIZE_OFF: usize = 36;
const NX_OMAP_OID_OFF: usize = 168;
const NX_FS_OID_OFF: usize = 232; // nx_fs_oid[0] — first volume virtual OID

// Volume superblock (apfs_superblock_t) field byte offsets.
const APFS_MAGIC_OFF: usize = 32;
const APFS_OMAP_OID_OFF: usize = 160; // physical OID of volume's own omap
const APFS_ROOT_TREE_OID_OFF: usize = 168; // virtual OID of FS root B-tree

// B-tree node (btnode_phys_t) field byte offsets (after 32-byte obj header).
const BTN_FLAGS_OFF: usize = 32;
const BTN_LEVEL_OFF: usize = 34;
const BTN_NKEYS_OFF: usize = 36;
const BTN_TOC_OFF_OFF: usize = 40; // btn_table_space.off
const BTN_TOC_LEN_OFF: usize = 42; // btn_table_space.len
const BTN_DATA_OFF: usize = 56; // start of key/value data area

// B-tree node flags.
const BTNODE_ROOT: u16 = 0x0001;
const BTNODE_LEAF: u16 = 0x0002;
const BTNODE_FIXED_KV_SIZE: u16 = 0x0004;

/// btree_info_t occupies the last 40 bytes of every root B-tree node.
const BTREE_INFO_SIZE: usize = 40;

// File system record type codes (upper 4 bits of j_key_t.obj_id_and_type).
const APFS_TYPE_INODE: u64 = 3;
const APFS_TYPE_FILE_EXTENT: u64 = 8;
const APFS_TYPE_DIR_REC: u64 = 9;

// APFS epoch offset: 2001-01-01T00:00:00Z expressed as Unix seconds.
const APFS_EPOCH_UNIX_SECS: u64 = 978_307_200;

/// Inode number of the APFS root directory.
const APFS_ROOT_DIR_INO: u64 = 2;

/// j_inode_val_t mode bit: directory.
const S_IFDIR: u16 = 0o040000;
const S_IFMT: u16 = 0o170000;

// j_inode_val_t field offsets (within the value byte slice).
const INO_PARENT_ID_OFF: usize = 0;
const INO_CREATE_TIME_OFF: usize = 16;
const INO_MOD_TIME_OFF: usize = 24;
const INO_MODE_OFF: usize = 80; // uint16_t mode
const INO_UNCOMPRESSED_SIZE_OFF: usize = 84; // uint64_t

// j_drec_val_t field offsets.
const DREC_FILE_ID_OFF: usize = 0; // uint64_t — target inode number

// j_file_extent_val_t field offsets.
const FEXT_LEN_FLAGS_OFF: usize = 0; // uint64_t — lower 56 bits = byte length
const FEXT_PHYS_BLOCK_OFF: usize = 8; // uint64_t — physical block number

// ── Parsed intermediate records ───────────────────────────────────────────────

#[derive(Debug)]
struct InodeRecord {
    ino: u64,
    #[allow(dead_code)]
    parent_id: u64,
    mode: u16,
    size: u64,
    created: Option<u64>,
    modified: Option<u64>,
}

#[derive(Debug)]
struct DirentRecord {
    parent_ino: u64,
    name: String,
    child_ino: u64,
}

#[derive(Debug)]
struct ExtentRecord {
    ino: u64,
    logical_offset: u64,
    byte_length: u64,
    phys_block: u64,
}

// ── Public struct ─────────────────────────────────────────────────────────────

/// Read-only APFS filesystem parser (MVP — single volume, no encryption).
pub struct ApfsParser {
    device: Arc<dyn BlockDevice>,
    block_size: u64,
    /// Physical block of the volume's file system root B-tree.
    root_tree_paddr: u64,
    /// Volume omap: virtual OID → physical block address.
    volume_omap: HashMap<u64, u64>,
}

impl ApfsParser {
    /// Parse the APFS container on `device` and return an initialised parser.
    ///
    /// Reads the container superblock at block 0, walks the object maps, and
    /// locates the first volume's file system root B-tree.
    pub fn new(device: Arc<dyn BlockDevice>) -> Result<Self> {
        let block0 = read_bytes(device.as_ref(), 0, 512)?;

        // Verify container magic at offset 32.
        let nx_magic = read_u32_le(&block0, NX_MAGIC_OFF)?;
        if nx_magic != NX_MAGIC {
            return Err(FilesystemError::InvalidStructure {
                context: "APFS container superblock",
                reason: format!("nx_magic {nx_magic:#010x} != expected {NX_MAGIC:#010x}"),
            });
        }

        let block_size = read_u32_le(&block0, NX_BLOCK_SIZE_OFF)? as u64;
        if block_size == 0 || block_size & (block_size - 1) != 0 {
            return Err(FilesystemError::InvalidStructure {
                context: "APFS container superblock",
                reason: format!("block_size {block_size} is not a power of two"),
            });
        }

        // Re-read a full block now that we know the block size.
        let sb = read_bytes(device.as_ref(), 0, block_size as usize)?;

        let nx_omap_oid = read_u64_le(&sb, NX_OMAP_OID_OFF)?;
        let nx_fs_oid = read_u64_le(&sb, NX_FS_OID_OFF)?;

        trace!(block_size, nx_omap_oid, nx_fs_oid, "APFS container parsed");

        // Container omap: maps virtual OIDs → physical blocks.
        let omap_block = read_bytes(
            device.as_ref(),
            nx_omap_oid * block_size,
            block_size as usize,
        )?;
        let container_omap = walk_omap_tree(&device, &omap_block, block_size)?;

        // Find the volume superblock.
        let vol_paddr = container_omap.get(&nx_fs_oid).copied().ok_or_else(|| {
            FilesystemError::InvalidStructure {
                context: "APFS container omap",
                reason: format!("volume OID {nx_fs_oid} not found in container omap"),
            }
        })?;

        let vol_block = read_bytes(device.as_ref(), vol_paddr * block_size, block_size as usize)?;

        let apfs_magic = read_u32_le(&vol_block, APFS_MAGIC_OFF)?;
        if apfs_magic != APFS_MAGIC {
            return Err(FilesystemError::InvalidStructure {
                context: "APFS volume superblock",
                reason: format!("apfs_magic {apfs_magic:#010x} != expected {APFS_MAGIC:#010x}"),
            });
        }

        let apfs_omap_oid = read_u64_le(&vol_block, APFS_OMAP_OID_OFF)?;
        let apfs_root_tree_oid = read_u64_le(&vol_block, APFS_ROOT_TREE_OID_OFF)?;

        trace!(apfs_omap_oid, apfs_root_tree_oid, "APFS volume parsed");

        // Volume omap: maps virtual OIDs → physical blocks (for FS tree nodes).
        let vol_omap_block = read_bytes(
            device.as_ref(),
            apfs_omap_oid * block_size,
            block_size as usize,
        )?;
        let volume_omap = walk_omap_tree(&device, &vol_omap_block, block_size)?;

        // Resolve the FS root B-tree to its physical block.
        let root_tree_paddr = volume_omap
            .get(&apfs_root_tree_oid)
            .copied()
            .ok_or_else(|| FilesystemError::InvalidStructure {
                context: "APFS volume omap",
                reason: format!("root tree OID {apfs_root_tree_oid} not found in volume omap"),
            })?;

        Ok(Self {
            device,
            block_size,
            root_tree_paddr,
            volume_omap,
        })
    }

    // ── Block I/O ─────────────────────────────────────────────────────────────

    fn read_block(&self, paddr: u64) -> Result<Vec<u8>> {
        read_bytes(
            self.device.as_ref(),
            paddr * self.block_size,
            self.block_size as usize,
        )
    }

    // ── FS record collection ──────────────────────────────────────────────────

    /// Walk the FS root B-tree and collect all inode, dirent and extent records.
    fn collect_fs_records(
        &self,
    ) -> Result<(Vec<InodeRecord>, Vec<DirentRecord>, Vec<ExtentRecord>)> {
        let root = self.read_block(self.root_tree_paddr)?;
        let mut inodes = Vec::new();
        let mut dirents = Vec::new();
        let mut extents = Vec::new();
        self.walk_fs_node(&root, &mut inodes, &mut dirents, &mut extents)?;
        Ok((inodes, dirents, extents))
    }

    fn walk_fs_node(
        &self,
        block: &[u8],
        inodes: &mut Vec<InodeRecord>,
        dirents: &mut Vec<DirentRecord>,
        extents: &mut Vec<ExtentRecord>,
    ) -> Result<()> {
        let btn_flags = read_u16_le_raw(block, BTN_FLAGS_OFF);
        let btn_level = read_u16_le_raw(block, BTN_LEVEL_OFF);
        let btn_nkeys = read_u32_le_raw(block, BTN_NKEYS_OFF) as usize;

        if btn_flags & BTNODE_LEAF != 0 {
            let toc_off = read_u16_le_raw(block, BTN_TOC_OFF_OFF) as usize;
            let toc_len = read_u16_le_raw(block, BTN_TOC_LEN_OFF) as usize;

            let is_root = btn_flags & BTNODE_ROOT != 0;
            let val_area_end = if is_root {
                self.block_size as usize - BTREE_INFO_SIZE
            } else {
                self.block_size as usize
            };

            // key_area_base: end of table-of-contents
            let key_area_base = BTN_DATA_OFF + toc_off + toc_len;

            // Walk toc entries (variable-size: kvloc_t = 8 bytes each)
            let toc_base = BTN_DATA_OFF + toc_off;
            for i in 0..btn_nkeys {
                let te = toc_base + i * 8;
                if te + 8 > block.len() {
                    break;
                }
                let k_off = read_u16_le_raw(block, te) as usize;
                let k_len = read_u16_le_raw(block, te + 2) as usize;
                let v_off = read_u16_le_raw(block, te + 4) as usize;
                let v_len = read_u16_le_raw(block, te + 6) as usize;

                let k_start = key_area_base + k_off;
                let v_start = val_area_end.saturating_sub(v_off);

                if k_start + k_len > block.len() || v_start + v_len > block.len() {
                    continue;
                }

                let key = &block[k_start..k_start + k_len];
                let val = &block[v_start..v_start + v_len];

                if key.len() < 8 {
                    continue;
                }

                let obj_id_and_type = u64::from_le_bytes(key[0..8].try_into().unwrap());
                let record_type = (obj_id_and_type >> 60) & 0xF;
                let object_id = obj_id_and_type & 0x0FFF_FFFF_FFFF_FFFF;

                match record_type {
                    APFS_TYPE_INODE => {
                        if let Some(rec) = parse_inode(object_id, val) {
                            inodes.push(rec);
                        }
                    }
                    APFS_TYPE_DIR_REC => {
                        if let Some(rec) = parse_dirent(object_id, key, val) {
                            dirents.push(rec);
                        }
                    }
                    APFS_TYPE_FILE_EXTENT => {
                        if let Some(rec) = parse_extent(object_id, key, val) {
                            extents.push(rec);
                        }
                    }
                    _ => {}
                }
            }
        } else if btn_level > 0 {
            // Index node: descend into children.
            let toc_off = read_u16_le_raw(block, BTN_TOC_OFF_OFF) as usize;
            let toc_len = read_u16_le_raw(block, BTN_TOC_LEN_OFF) as usize;
            let is_root = btn_flags & BTNODE_ROOT != 0;
            let val_area_end = if is_root {
                self.block_size as usize - BTREE_INFO_SIZE
            } else {
                self.block_size as usize
            };
            let key_area_base = BTN_DATA_OFF + toc_off + toc_len;
            let toc_base = BTN_DATA_OFF + toc_off;

            for i in 0..btn_nkeys {
                let te = toc_base + i * 8;
                if te + 8 > block.len() {
                    break;
                }
                let v_off = read_u16_le_raw(block, te + 4) as usize;
                let _ = key_area_base; // keys unused for index descent
                let v_start = val_area_end.saturating_sub(v_off);
                if v_start + 8 > block.len() {
                    continue;
                }
                // Child pointer: virtual OID stored as u64.
                let child_oid = u64::from_le_bytes(block[v_start..v_start + 8].try_into().unwrap());
                let child_paddr = match self.volume_omap.get(&child_oid) {
                    Some(&p) => p,
                    None => continue,
                };
                let child_block = match self.read_block(child_paddr) {
                    Ok(b) => b,
                    Err(_) => continue,
                };
                let _ = self.walk_fs_node(&child_block, inodes, dirents, extents);
            }
        }
        Ok(())
    }

    // ── Directory navigation ──────────────────────────────────────────────────

    /// Return the inode number of the directory at `path` (relative to root).
    fn resolve_dir_ino(&self, path: &str) -> Result<u64> {
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        if parts.is_empty() {
            return Ok(APFS_ROOT_DIR_INO);
        }
        let (inodes, dirents, _) = self.collect_fs_records()?;
        let mut cur_ino = APFS_ROOT_DIR_INO;
        for part in &parts {
            let found = dirents
                .iter()
                .find(|d| d.parent_ino == cur_ino && d.name.eq_ignore_ascii_case(part));
            match found {
                Some(d) => {
                    // Verify the child is a directory.
                    let is_dir = inodes
                        .iter()
                        .any(|ino| ino.ino == d.child_ino && (ino.mode & S_IFMT) == S_IFDIR);
                    if !is_dir {
                        return Err(FilesystemError::NotFound(path.to_string()));
                    }
                    cur_ino = d.child_ino;
                }
                None => return Err(FilesystemError::NotFound(path.to_string())),
            }
        }
        Ok(cur_ino)
    }

    /// Build `FileEntry` values for all entries in directory `parent_ino`.
    fn list_dir_ino(&self, parent_ino: u64) -> Result<Vec<FileEntry>> {
        let (inodes, dirents, _) = self.collect_fs_records()?;
        let mut result = Vec::new();

        for d in dirents.iter().filter(|d| d.parent_ino == parent_ino) {
            let ino_rec = inodes.iter().find(|i| i.ino == d.child_ino);
            let (size, is_dir, created, modified, mode) = match ino_rec {
                Some(r) => (
                    r.size,
                    (r.mode & S_IFMT) == S_IFDIR,
                    r.created,
                    r.modified,
                    r.mode,
                ),
                None => (0, false, None, None, 0),
            };

            // data_byte_offset: physical byte address of first extent, if any.
            // We don't pre-load extents here; leave as None (caller uses read_file).
            result.push(FileEntry {
                name: d.name.clone(),
                path: format!("/{}", d.name),
                size,
                is_dir,
                is_deleted: false,
                created,
                modified,
                first_cluster: None,
                mft_record: None,
                inode_number: Some(d.child_ino as u32),
                data_byte_offset: None,
                recovery_chance: RecoveryChance::Unknown,
            });
            let _ = mode;
        }

        Ok(result)
    }
}

// ── FilesystemParser impl ─────────────────────────────────────────────────────

impl FilesystemParser for ApfsParser {
    fn filesystem_type(&self) -> FilesystemType {
        FilesystemType::Apfs
    }

    fn root_directory(&self) -> Result<Vec<FileEntry>> {
        self.list_dir_ino(APFS_ROOT_DIR_INO)
    }

    fn list_directory(&self, path: &str) -> Result<Vec<FileEntry>> {
        let dir_ino = self.resolve_dir_ino(path)?;
        self.list_dir_ino(dir_ino)
    }

    fn read_file(&self, entry: &FileEntry, writer: &mut dyn Write) -> Result<u64> {
        let ino = entry
            .inode_number
            .ok_or_else(|| FilesystemError::InvalidStructure {
                context: "APFS read_file",
                reason: "FileEntry has no inode number".to_string(),
            })? as u64;

        let (_, _, mut extents) = self.collect_fs_records()?;
        extents.retain(|e| e.ino == ino);
        extents.sort_by_key(|e| e.logical_offset);

        let mut written = 0u64;
        let file_size = entry.size;

        for ext in &extents {
            if written >= file_size {
                break;
            }
            let remaining = file_size - written;
            let to_copy = remaining.min(ext.byte_length);
            let data = read_bytes(
                self.device.as_ref(),
                ext.phys_block * self.block_size,
                to_copy as usize,
            )?;
            writer
                .write_all(&data)
                .map_err(|e| FilesystemError::InvalidStructure {
                    context: "APFS read_file write",
                    reason: e.to_string(),
                })?;
            written += data.len() as u64;
        }

        Ok(written)
    }

    /// APFS does not have a traditional "deleted" marker — inodes are reclaimed
    /// immediately.  Undelete requires journal replay and is out of scope for
    /// this MVP.
    fn deleted_files(&self) -> Result<Vec<FileEntry>> {
        Ok(Vec::new())
    }
}

// ── B-tree helpers ────────────────────────────────────────────────────────────

/// Walk an omap B-tree (fixed KV, root+leaf or multi-level) and return a
/// map of virtual OID → physical block address, keeping the entry with the
/// highest transaction ID for each OID.
fn walk_omap_tree(
    device: &Arc<dyn BlockDevice>,
    block: &[u8],
    block_size: u64,
) -> Result<HashMap<u64, u64>> {
    let mut map: HashMap<u64, (u64, u64)> = HashMap::new();
    walk_omap_node(device, block, block_size, &mut map)?;
    Ok(map
        .into_iter()
        .map(|(oid, (_, paddr))| (oid, paddr))
        .collect())
}

fn walk_omap_node(
    device: &Arc<dyn BlockDevice>,
    block: &[u8],
    block_size: u64,
    map: &mut HashMap<u64, (u64, u64)>, // oid → (xid, paddr)
) -> Result<()> {
    let btn_flags = read_u16_le_raw(block, BTN_FLAGS_OFF);
    let btn_level = read_u16_le_raw(block, BTN_LEVEL_OFF);
    let btn_nkeys = read_u32_le_raw(block, BTN_NKEYS_OFF) as usize;

    let toc_off = read_u16_le_raw(block, BTN_TOC_OFF_OFF) as usize;
    let toc_len = read_u16_le_raw(block, BTN_TOC_LEN_OFF) as usize;
    let is_root = btn_flags & BTNODE_ROOT != 0;
    let val_area_end = if is_root {
        block.len() - BTREE_INFO_SIZE
    } else {
        block.len()
    };

    // key_area_base: end of table of contents
    let key_area_base = BTN_DATA_OFF + toc_off + toc_len;
    let toc_base = BTN_DATA_OFF + toc_off;

    if btn_flags & BTNODE_FIXED_KV_SIZE != 0 {
        // Fixed KV: toc entries are kvoff_t = {k_off: u16, v_off: u16}
        // key = omap_key_t (16 bytes), val = omap_val_t (16 bytes)
        for i in 0..btn_nkeys {
            let te = toc_base + i * 4;
            if te + 4 > block.len() {
                break;
            }
            let k_off = read_u16_le_raw(block, te) as usize;
            let v_off = read_u16_le_raw(block, te + 2) as usize;

            let k_start = key_area_base + k_off;
            let v_start = val_area_end.saturating_sub(v_off);

            if k_start + 16 > block.len() || v_start + 16 > block.len() {
                continue;
            }

            if btn_level == 0 {
                // Leaf: key=omap_key_t, val=omap_val_t
                let ok_oid = u64::from_le_bytes(block[k_start..k_start + 8].try_into().unwrap());
                let ok_xid =
                    u64::from_le_bytes(block[k_start + 8..k_start + 16].try_into().unwrap());
                // ov_paddr is at val+8 (after ov_flags:u32 + ov_size:u32)
                let ov_paddr =
                    u64::from_le_bytes(block[v_start + 8..v_start + 16].try_into().unwrap());

                // Keep highest-xid entry for each OID.
                let entry = map.entry(ok_oid).or_insert((0, ov_paddr));
                if ok_xid >= entry.0 {
                    *entry = (ok_xid, ov_paddr);
                }
            } else {
                // Index node: val is child physical block address.
                let child_paddr =
                    u64::from_le_bytes(block[v_start..v_start + 8].try_into().unwrap());
                let child_bytes = read_bytes(
                    device.as_ref(),
                    child_paddr * block_size,
                    block_size as usize,
                );
                if let Ok(child_block) = child_bytes {
                    let _ = walk_omap_node(device, &child_block, block_size, map);
                }
            }
        }
    }

    Ok(())
}

// ── FS record parsers ─────────────────────────────────────────────────────────

fn parse_inode(ino: u64, val: &[u8]) -> Option<InodeRecord> {
    if val.len() < INO_UNCOMPRESSED_SIZE_OFF + 8 {
        return None;
    }
    let parent_id = u64::from_le_bytes(
        val[INO_PARENT_ID_OFF..INO_PARENT_ID_OFF + 8]
            .try_into()
            .ok()?,
    );
    let create_ns = i64::from_le_bytes(
        val[INO_CREATE_TIME_OFF..INO_CREATE_TIME_OFF + 8]
            .try_into()
            .ok()?,
    );
    let mod_ns = i64::from_le_bytes(
        val[INO_MOD_TIME_OFF..INO_MOD_TIME_OFF + 8]
            .try_into()
            .ok()?,
    );
    let mode = u16::from_le_bytes(val[INO_MODE_OFF..INO_MODE_OFF + 2].try_into().ok()?);
    let size = u64::from_le_bytes(
        val[INO_UNCOMPRESSED_SIZE_OFF..INO_UNCOMPRESSED_SIZE_OFF + 8]
            .try_into()
            .ok()?,
    );

    let created = apfs_ts_to_unix(create_ns);
    let modified = apfs_ts_to_unix(mod_ns);

    Some(InodeRecord {
        ino,
        parent_id,
        mode,
        size,
        created,
        modified,
    })
}

fn parse_dirent(parent_ino: u64, key: &[u8], val: &[u8]) -> Option<DirentRecord> {
    // j_drec_key_t: j_key_t (8 bytes) + name_len (u16, 2 bytes) + name bytes
    if key.len() < 11 || val.len() < DREC_FILE_ID_OFF + 8 {
        return None;
    }
    let name_len = u16::from_le_bytes([key[8], key[9]]) as usize;
    if key.len() < 10 + name_len {
        return None;
    }
    let name_bytes = &key[10..10 + name_len];
    // Name is null-terminated UTF-8; strip the null.
    let name = String::from_utf8_lossy(name_bytes)
        .trim_end_matches('\0')
        .to_string();
    let child_ino = u64::from_le_bytes(
        val[DREC_FILE_ID_OFF..DREC_FILE_ID_OFF + 8]
            .try_into()
            .ok()?,
    );
    Some(DirentRecord {
        parent_ino,
        name,
        child_ino,
    })
}

fn parse_extent(ino: u64, key: &[u8], val: &[u8]) -> Option<ExtentRecord> {
    // j_file_extent_key_t: j_key_t (8 bytes) + logical_addr (u64, 8 bytes)
    if key.len() < 16 || val.len() < FEXT_PHYS_BLOCK_OFF + 8 {
        return None;
    }
    let logical_offset = u64::from_le_bytes(key[8..16].try_into().ok()?);
    let len_and_flags = u64::from_le_bytes(
        val[FEXT_LEN_FLAGS_OFF..FEXT_LEN_FLAGS_OFF + 8]
            .try_into()
            .ok()?,
    );
    let phys_block = u64::from_le_bytes(
        val[FEXT_PHYS_BLOCK_OFF..FEXT_PHYS_BLOCK_OFF + 8]
            .try_into()
            .ok()?,
    );
    // Lower 56 bits = byte length; upper 8 bits = flags.
    let byte_length = len_and_flags & 0x00FF_FFFF_FFFF_FFFF;
    if byte_length == 0 {
        return None;
    }
    Some(ExtentRecord {
        ino,
        logical_offset,
        byte_length,
        phys_block,
    })
}

// ── Timestamp helper ──────────────────────────────────────────────────────────

/// Convert APFS nanosecond timestamp (since 2001-01-01) to Unix seconds.
fn apfs_ts_to_unix(ns: i64) -> Option<u64> {
    if ns <= 0 {
        return None;
    }
    let secs_since_mac_epoch = (ns / 1_000_000_000) as u64;
    Some(APFS_EPOCH_UNIX_SECS + secs_since_mac_epoch)
}

// ── Raw u16 read (infallible, returns 0 on short buffer) ─────────────────────

fn read_u16_le_raw(buf: &[u8], off: usize) -> u16 {
    buf.get(off..off + 2)
        .map(|s| u16::from_le_bytes([s[0], s[1]]))
        .unwrap_or(0)
}

fn read_u32_le_raw(buf: &[u8], off: usize) -> u32 {
    buf.get(off..off + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .unwrap_or(0)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ferrite_blockdev::MockBlockDevice;

    use super::*;

    const BLOCK_SIZE: usize = 4096;

    /// Build a 6-block minimal APFS image in memory.
    ///
    /// Block layout:
    ///   Block 0 — Container superblock (nx_superblock_t)
    ///   Block 1 — Container omap leaf+root (1 entry: vOID 2 → paddr 2)
    ///   Block 2 — Volume superblock (apfs_superblock_t)
    ///   Block 3 — Volume omap leaf+root (1 entry: vOID 4 → paddr 4)
    ///   Block 4 — FS root B-tree leaf+root (inode 16 + dirent + extent)
    ///   Block 5 — File data ("Hello, World!")
    fn build_image() -> MockBlockDevice {
        let total = BLOCK_SIZE * 6;
        let mut dev = MockBlockDevice::zeroed(total, BLOCK_SIZE as u32);

        // ── Block 0: Container superblock ─────────────────────────────────────
        let mut b0 = vec![0u8; BLOCK_SIZE];
        // nx_magic @ 32
        b0[32..36].copy_from_slice(&NX_MAGIC.to_le_bytes());
        // nx_block_size @ 36
        b0[36..40].copy_from_slice(&(BLOCK_SIZE as u32).to_le_bytes());
        // nx_omap_oid @ 168 = 1 (block 1)
        b0[168..176].copy_from_slice(&1u64.to_le_bytes());
        // nx_fs_oid[0] @ 232 = 2 (virtual OID for volume, → block 2 via omap)
        b0[232..240].copy_from_slice(&2u64.to_le_bytes());
        dev.write_sector(0, &b0);

        // ── Block 1: Container omap (leaf+root, fixed KV) ─────────────────────
        // Maps virtual OID 2 → physical block 2 (volume superblock).
        dev.write_sector(1, &build_omap_block(2, 1, 2));

        // ── Block 2: Volume superblock ────────────────────────────────────────
        let mut b2 = vec![0u8; BLOCK_SIZE];
        // apfs_magic @ 32
        b2[32..36].copy_from_slice(&APFS_MAGIC.to_le_bytes());
        // apfs_omap_oid @ 160 = 3 (physical block 3)
        b2[160..168].copy_from_slice(&3u64.to_le_bytes());
        // apfs_root_tree_oid @ 168 = 4 (virtual OID, → block 4 via volume omap)
        b2[168..176].copy_from_slice(&4u64.to_le_bytes());
        dev.write_sector(2, &b2);

        // ── Block 3: Volume omap (leaf+root, fixed KV) ────────────────────────
        // Maps virtual OID 4 → physical block 4 (FS root B-tree).
        dev.write_sector(3, &build_omap_block(4, 1, 4));

        // ── Block 4: FS root B-tree (leaf+root, variable KV) ──────────────────
        dev.write_sector(4, &build_fs_btree_block());

        // ── Block 5: File data ────────────────────────────────────────────────
        let mut b5 = vec![0u8; BLOCK_SIZE];
        b5[..13].copy_from_slice(b"Hello, World!");
        dev.write_sector(5, &b5);

        dev
    }

    /// Build a single-entry omap leaf+root block.
    /// Maps (ok_oid=virt_oid, ok_xid=xid) → ov_paddr=paddr.
    fn build_omap_block(virt_oid: u64, xid: u64, paddr: u64) -> Vec<u8> {
        let mut b = vec![0u8; BLOCK_SIZE];

        // btn_flags @ 32: ROOT | LEAF | FIXED_KV_SIZE = 0x0007
        b[32..34].copy_from_slice(&0x0007u16.to_le_bytes());
        // btn_level @ 34: 0
        b[34..36].copy_from_slice(&0u16.to_le_bytes());
        // btn_nkeys @ 36: 1
        b[36..40].copy_from_slice(&1u32.to_le_bytes());
        // btn_table_space: {off=0, len=4}
        b[40..42].copy_from_slice(&0u16.to_le_bytes()); // off
        b[42..44].copy_from_slice(&4u16.to_le_bytes()); // len

        // Data area @ 56:
        // toc[0] = kvoff_t {k_off=0, v_off=16}
        b[56..58].copy_from_slice(&0u16.to_le_bytes()); // k_off
        b[58..60].copy_from_slice(&16u16.to_le_bytes()); // v_off

        // key area base = 56 + 0 + 4 = 60
        // key[0] = omap_key_t {ok_oid, ok_xid} at [60..76]
        b[60..68].copy_from_slice(&virt_oid.to_le_bytes());
        b[68..76].copy_from_slice(&xid.to_le_bytes());

        // val_area_end = 4096 - 40 = 4056 (root node has btree_info)
        // val[0] at 4056 - 16 = 4040
        // omap_val_t: {ov_flags:u32=0, ov_size:u32=4096, ov_paddr:u64=paddr}
        b[4040..4044].copy_from_slice(&0u32.to_le_bytes()); // ov_flags
        b[4044..4048].copy_from_slice(&(BLOCK_SIZE as u32).to_le_bytes()); // ov_size
        b[4048..4056].copy_from_slice(&paddr.to_le_bytes()); // ov_paddr

        // btree_info at [4056..4096]
        b[4056..4060].copy_from_slice(&0u32.to_le_bytes()); // bt_flags
        b[4060..4064].copy_from_slice(&(BLOCK_SIZE as u32).to_le_bytes()); // bt_node_size
        b[4064..4068].copy_from_slice(&16u32.to_le_bytes()); // bt_key_size
        b[4068..4072].copy_from_slice(&16u32.to_le_bytes()); // bt_val_size
        b[4072..4076].copy_from_slice(&16u32.to_le_bytes()); // bt_longest_key
        b[4076..4080].copy_from_slice(&16u32.to_le_bytes()); // bt_longest_val
        b[4080..4088].copy_from_slice(&1u64.to_le_bytes()); // bt_key_count
        b[4088..4096].copy_from_slice(&1u64.to_le_bytes()); // bt_node_count

        b
    }

    /// Build the FS root B-tree block with 3 records:
    ///   - Inode 16 (regular file, size=13, parent=APFS_ROOT_DIR_INO=2)
    ///   - Dirent in parent 2: name="HELLO.TXT" → inode 16
    ///   - Extent for inode 16: logical_offset=0, byte_length=13, phys_block=5
    fn build_fs_btree_block() -> Vec<u8> {
        let mut b = vec![0u8; BLOCK_SIZE];

        // btn_flags @ 32: ROOT | LEAF = 0x0003 (variable KV)
        b[32..34].copy_from_slice(&0x0003u16.to_le_bytes());
        // btn_level @ 34: 0
        b[34..36].copy_from_slice(&0u16.to_le_bytes());
        // btn_nkeys @ 36: 3
        b[36..40].copy_from_slice(&3u32.to_le_bytes());
        // btn_table_space: {off=0, len=24} (3 × 8-byte kvloc_t)
        b[40..42].copy_from_slice(&0u16.to_le_bytes()); // off
        b[42..44].copy_from_slice(&24u16.to_le_bytes()); // len

        // toc_base = BTN_DATA_OFF + toc_off = 56 + 0 = 56
        // key_area_base = 56 + 0 + 24 = 80
        // val_area_end = 4096 - 40 = 4056 (root node)

        // ── Key layout (growing from 80) ──────────────────────────────────────
        // key0: inode key (8 bytes) at 80..88, k.off=0
        // key1: dirent key (20 bytes) at 88..108, k.off=8
        // key2: extent key (16 bytes) at 108..124, k.off=28

        let inode_ino: u64 = 16;
        let parent_ino: u64 = APFS_ROOT_DIR_INO;
        let dirent_name = b"HELLO.TXT\0"; // 10 bytes incl. null
        let dirent_name_len: u16 = 10;

        // key0: j_inode_key_t = j_key_t {obj_id_and_type}
        let inode_key: u64 = (APFS_TYPE_INODE << 60) | inode_ino;
        b[80..88].copy_from_slice(&inode_key.to_le_bytes());

        // key1: j_drec_key_t = j_key_t + name_len (u16) + name bytes
        let dirent_key: u64 = (APFS_TYPE_DIR_REC << 60) | parent_ino;
        b[88..96].copy_from_slice(&dirent_key.to_le_bytes());
        b[96..98].copy_from_slice(&dirent_name_len.to_le_bytes());
        b[98..108].copy_from_slice(dirent_name);

        // key2: j_file_extent_key_t = j_key_t + logical_addr (u64)
        let extent_key: u64 = (APFS_TYPE_FILE_EXTENT << 60) | inode_ino;
        b[108..116].copy_from_slice(&extent_key.to_le_bytes());
        b[116..124].copy_from_slice(&0u64.to_le_bytes()); // logical_addr=0

        // ── Value layout (growing down from 4056) ─────────────────────────────
        // val0 (inode val, 92 bytes): at 3964..4056, v.off=92, v.len=92
        // val1 (dirent val, 18 bytes): at 3946..3964, v.off=110, v.len=18
        // val2 (extent val, 24 bytes): at 3922..3946, v.off=134, v.len=24

        // ── toc entries at [56..80] ────────────────────────────────────────────
        // toc[0]: {k:{off=0,len=8}, v:{off=92,len=92}}
        b[56..58].copy_from_slice(&0u16.to_le_bytes()); // k.off
        b[58..60].copy_from_slice(&8u16.to_le_bytes()); // k.len
        b[60..62].copy_from_slice(&92u16.to_le_bytes()); // v.off
        b[62..64].copy_from_slice(&92u16.to_le_bytes()); // v.len
                                                         // toc[1]: {k:{off=8,len=20}, v:{off=110,len=18}}
        b[64..66].copy_from_slice(&8u16.to_le_bytes()); // k.off
        b[66..68].copy_from_slice(&20u16.to_le_bytes()); // k.len
        b[68..70].copy_from_slice(&110u16.to_le_bytes()); // v.off
        b[70..72].copy_from_slice(&18u16.to_le_bytes()); // v.len
                                                         // toc[2]: {k:{off=28,len=16}, v:{off=134,len=24}}
        b[72..74].copy_from_slice(&28u16.to_le_bytes()); // k.off
        b[74..76].copy_from_slice(&16u16.to_le_bytes()); // k.len
        b[76..78].copy_from_slice(&134u16.to_le_bytes()); // v.off
        b[78..80].copy_from_slice(&24u16.to_le_bytes()); // v.len

        // ── val0: j_inode_val_t at [3964..4056] (92 bytes) ───────────────────
        let v0 = 3964usize;
        b[v0..v0 + 8].copy_from_slice(&parent_ino.to_le_bytes()); // parent_id
        b[v0 + 8..v0 + 16].copy_from_slice(&inode_ino.to_le_bytes()); // private_id
                                                                      // timestamps @ 16,24,32,40 = 0 (None)
                                                                      // internal_flags @ 48 = 0
                                                                      // nlink @ 56 = 1
        b[v0 + 56..v0 + 60].copy_from_slice(&1u32.to_le_bytes());
        // cp_key_class @ 60, write_gen @ 64, bsd_flags @ 68 = 0
        // owner @ 72, group @ 76 = 0
        // mode @ 80: S_IFREG | 0o644 = 0o100644 = 0x81A4
        b[v0 + 80..v0 + 82].copy_from_slice(&0x81A4u16.to_le_bytes());
        // pad1 @ 82 = 0
        // uncompressed_size @ 84 = 13
        b[v0 + 84..v0 + 92].copy_from_slice(&13u64.to_le_bytes());

        // ── val1: j_drec_val_t at [3946..3964] (18 bytes) ────────────────────
        let v1 = 3946usize;
        b[v1..v1 + 8].copy_from_slice(&inode_ino.to_le_bytes()); // file_id
                                                                 // date_added @ 8 = 0
                                                                 // flags @ 16: DT_REG = 4
        b[v1 + 16..v1 + 18].copy_from_slice(&4u16.to_le_bytes());

        // ── val2: j_file_extent_val_t at [3922..3946] (24 bytes) ─────────────
        let v2 = 3922usize;
        b[v2..v2 + 8].copy_from_slice(&13u64.to_le_bytes()); // len_and_flags (len=13, flags=0)
        b[v2 + 8..v2 + 16].copy_from_slice(&5u64.to_le_bytes()); // phys_block_num = 5
                                                                 // crypto_id @ 16 = 0

        // ── btree_info at [4056..4096] ────────────────────────────────────────
        b[4056..4060].copy_from_slice(&0u32.to_le_bytes()); // bt_flags
        b[4060..4064].copy_from_slice(&(BLOCK_SIZE as u32).to_le_bytes()); // bt_node_size
        b[4064..4068].copy_from_slice(&0u32.to_le_bytes()); // bt_key_size (variable = 0)
        b[4068..4072].copy_from_slice(&0u32.to_le_bytes()); // bt_val_size (variable = 0)
        b[4072..4076].copy_from_slice(&20u32.to_le_bytes()); // bt_longest_key
        b[4076..4080].copy_from_slice(&92u32.to_le_bytes()); // bt_longest_val
        b[4080..4088].copy_from_slice(&3u64.to_le_bytes()); // bt_key_count
        b[4088..4096].copy_from_slice(&1u64.to_le_bytes()); // bt_node_count

        b
    }

    #[test]
    fn detects_apfs() {
        let dev = Arc::new(build_image());
        let parser = ApfsParser::new(dev).unwrap();
        assert_eq!(parser.filesystem_type(), FilesystemType::Apfs);
    }

    #[test]
    fn root_directory_lists_file() {
        let dev = Arc::new(build_image());
        let parser = ApfsParser::new(dev).unwrap();
        let entries = parser.root_directory().unwrap();
        assert_eq!(entries.len(), 1, "expected 1 entry in root dir");
        let e = &entries[0];
        assert_eq!(e.name, "HELLO.TXT");
        assert_eq!(e.size, 13);
        assert!(!e.is_dir);
        assert!(!e.is_deleted);
    }

    #[test]
    fn read_file_returns_content() {
        let dev = Arc::new(build_image());
        let parser = ApfsParser::new(dev).unwrap();
        let entries = parser.root_directory().unwrap();
        assert_eq!(entries.len(), 1);
        let mut buf = Vec::new();
        let written = parser.read_file(&entries[0], &mut buf).unwrap();
        assert_eq!(written, 13);
        assert_eq!(&buf, b"Hello, World!");
    }

    #[test]
    fn deleted_files_returns_empty() {
        let dev = Arc::new(build_image());
        let parser = ApfsParser::new(dev).unwrap();
        assert!(parser.deleted_files().unwrap().is_empty());
    }

    #[test]
    fn rejects_non_apfs_device() {
        let dev = Arc::new(MockBlockDevice::zeroed(4096, 4096));
        assert!(ApfsParser::new(dev).is_err());
    }

    #[test]
    fn apfs_ts_zero_is_none() {
        assert_eq!(apfs_ts_to_unix(0), None);
    }

    #[test]
    fn apfs_ts_positive_converts() {
        // 1_000_000_000 ns = 1 second from Mac epoch → Unix = 978_307_201
        assert_eq!(apfs_ts_to_unix(1_000_000_000), Some(978_307_201));
    }

    #[test]
    fn apfs_ts_negative_is_none() {
        assert_eq!(apfs_ts_to_unix(-1), None);
    }

    #[test]
    fn omap_lookup_finds_entry() {
        let block = build_omap_block(7, 3, 42);
        let dev: Arc<dyn BlockDevice> = Arc::new(MockBlockDevice::zeroed(4096, 4096));
        let result = walk_omap_tree(&dev, &block, 4096).unwrap();
        assert_eq!(result.get(&7), Some(&42));
    }
}

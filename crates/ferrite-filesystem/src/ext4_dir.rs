//! ext4 directory-block parser split from `ext4.rs`.

use crate::{FileEntry, RecoveryChance};

/// Parse linear ext4 directory entries from a single block.
pub(crate) fn parse_dir_block(
    block: &[u8],
    path_prefix: &str,
    include_deleted: bool,
) -> Vec<FileEntry> {
    let mut entries = Vec::new();
    let mut pos = 0usize;

    while pos + 8 <= block.len() {
        // Safety: pos + 8 <= block.len() guarantees these reads are in-bounds.
        let inode_num =
            u32::from_le_bytes([block[pos], block[pos + 1], block[pos + 2], block[pos + 3]]);
        let rec_len = u16::from_le_bytes([block[pos + 4], block[pos + 5]]) as usize;
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
                        data_byte_offset: None, // enriched by list_inode() afterwards
                        recovery_chance: RecoveryChance::Unknown,
                    });
                }
            }
        }

        pos += rec_len;
    }

    entries
}

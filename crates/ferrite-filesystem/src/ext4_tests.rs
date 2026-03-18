use std::sync::Arc;

use ferrite_blockdev::MockBlockDevice;

use super::*;

/// Build a minimal ext4 image (8 KiB, 1 KiB blocks).
///
/// Block layout:
///  0 (  0â€“ 1023): unused (boot)
///  1 (1024â€“2047): superblock
///  2 (2048â€“3071): group descriptor table
///  3 (3072â€“4095): block bitmap  (unused in tests)
///  4 (4096â€“5119): inode bitmap  (unused in tests)
///  5 (5120â€“6143): inode table   (inodes 1..8, 128 bytes each)
///  6 (6144â€“7167): root directory data
///  7 (7168â€“8191): file data
fn build_image() -> MockBlockDevice {
    let mut image = vec![0u8; 8192];

    // â”€â”€ Superblock at offset 1024 â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
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

    // â”€â”€ Group descriptor table at offset 2048 â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    {
        let gdt = &mut image[2048..2080];
        gdt[0..4].copy_from_slice(&3u32.to_le_bytes()); // bg_block_bitmap
        gdt[4..8].copy_from_slice(&4u32.to_le_bytes()); // bg_inode_bitmap
        gdt[8..12].copy_from_slice(&5u32.to_le_bytes()); // bg_inode_table = block 5
    }

    // â”€â”€ Inode table at block 5 (offset 5120) â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
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

    // â”€â”€ Root directory data at block 6 (offset 6144) â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    {
        let dir = &mut image[6144..7168];
        // Entry: "." â†’ inode 2
        dir[0..4].copy_from_slice(&2u32.to_le_bytes());
        dir[4..6].copy_from_slice(&12u16.to_le_bytes());
        dir[6] = 1;
        dir[7] = 2; // directory
        dir[8] = b'.';
        // Entry: ".." â†’ inode 2
        dir[12..16].copy_from_slice(&2u32.to_le_bytes());
        dir[16..18].copy_from_slice(&12u16.to_le_bytes());
        dir[18] = 2;
        dir[19] = 2;
        dir[20..22].copy_from_slice(b"..");
        // Entry: "test.txt" â†’ inode 3
        dir[24..28].copy_from_slice(&3u32.to_le_bytes());
        dir[28..30].copy_from_slice(&20u16.to_le_bytes()); // rec_len = 20
        dir[30] = 8; // name_len
        dir[31] = 1; // regular file
        dir[32..40].copy_from_slice(b"test.txt");
        // Entry: deleted "lost.dat" â†’ inode 0
        dir[44..48].copy_from_slice(&0u32.to_le_bytes()); // inode = 0 â†’ deleted
        dir[48..50].copy_from_slice(&(1024u16 - 44).to_le_bytes()); // rec_len = rest
        dir[50] = 8; // name_len
        dir[51] = 1; // regular file
        dir[52..60].copy_from_slice(b"lost.dat");
    }

    // â”€â”€ File data at block 7 (offset 7168) â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
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

/// Build a minimal ext4 image where all inodes use the extent tree.
///
/// Same block layout as `build_image()` but inode 2 (root dir) and
/// inode 3 (file) both have `EXT4_INODE_EXTENTS` set and a one-leaf
/// extent entry in the i_block area.
fn build_extent_image() -> MockBlockDevice {
    let mut image = vec![0u8; 8192];

    // â”€â”€ Superblock at offset 1024 â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    {
        let sb = &mut image[1024..2048];
        sb[0..4].copy_from_slice(&16u32.to_le_bytes());
        sb[4..8].copy_from_slice(&8u32.to_le_bytes());
        sb[20..24].copy_from_slice(&1u32.to_le_bytes()); // s_first_data_block
        sb[24..28].copy_from_slice(&0u32.to_le_bytes()); // 1 KiB blocks
        sb[32..36].copy_from_slice(&8192u32.to_le_bytes());
        sb[40..44].copy_from_slice(&16u32.to_le_bytes()); // inodes_per_group
        sb[56..58].copy_from_slice(&EXT4_MAGIC.to_le_bytes());
        sb[92..96].copy_from_slice(&0u32.to_le_bytes()); // rev 0 â†’ 128-byte inodes
    }

    // â”€â”€ Group descriptor table at offset 2048 â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    {
        let gdt = &mut image[2048..2080];
        gdt[0..4].copy_from_slice(&3u32.to_le_bytes()); // bg_block_bitmap
        gdt[4..8].copy_from_slice(&4u32.to_le_bytes()); // bg_inode_bitmap
        gdt[8..12].copy_from_slice(&5u32.to_le_bytes()); // bg_inode_table
    }

    // Helper: write an extent header + one leaf entry into `buf`.
    // Header: magic, entries=1, max=4, depth=0, gen=0
    // Leaf:   ee_block=0, ee_len=1, ee_start_hi=0, ee_start_lo=`phys`
    let write_extent_tree = |buf: &mut [u8], phys: u32| {
        buf[0..2].copy_from_slice(&EXT4_EXTENT_MAGIC.to_le_bytes());
        buf[2..4].copy_from_slice(&1u16.to_le_bytes()); // entries
        buf[4..6].copy_from_slice(&4u16.to_le_bytes()); // max
        buf[6..8].copy_from_slice(&0u16.to_le_bytes()); // depth = 0 (leaf)
        buf[8..12].copy_from_slice(&0u32.to_le_bytes()); // generation
                                                         // leaf entry at +12
        buf[12..16].copy_from_slice(&0u32.to_le_bytes()); // ee_block
        buf[16..18].copy_from_slice(&1u16.to_le_bytes()); // ee_len
        buf[18..20].copy_from_slice(&0u16.to_le_bytes()); // ee_start_hi
        buf[20..24].copy_from_slice(&phys.to_le_bytes()); // ee_start_lo
    };

    // â”€â”€ Inode 2: root directory with extent tree â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    {
        let off = 5120 + 128; // inode 2 = index 1 in inode table
        let ino = &mut image[off..off + 128];
        ino[0..2].copy_from_slice(&(S_IFDIR | 0x1ED).to_le_bytes());
        ino[4..8].copy_from_slice(&1024u32.to_le_bytes()); // i_size_lo
        ino[26..28].copy_from_slice(&2u16.to_le_bytes()); // i_links_count
        ino[32..36].copy_from_slice(&EXT4_INODE_EXTENTS.to_le_bytes()); // i_flags
        write_extent_tree(&mut ino[40..64], 6); // extent â†’ block 6
    }

    // â”€â”€ Inode 3: regular file with extent tree â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    {
        let off = 5120 + 256; // inode 3 = index 2
        let ino = &mut image[off..off + 128];
        ino[0..2].copy_from_slice(&(0x8000u16 | 0x1A4).to_le_bytes());
        ino[4..8].copy_from_slice(&13u32.to_le_bytes()); // i_size_lo
        ino[26..28].copy_from_slice(&1u16.to_le_bytes());
        ino[32..36].copy_from_slice(&EXT4_INODE_EXTENTS.to_le_bytes()); // i_flags
        write_extent_tree(&mut ino[40..64], 7); // extent â†’ block 7
    }

    // â”€â”€ Root directory data at block 6 (offset 6144) â€” same as build_image â”€
    {
        let dir = &mut image[6144..7168];
        dir[0..4].copy_from_slice(&2u32.to_le_bytes());
        dir[4..6].copy_from_slice(&12u16.to_le_bytes());
        dir[6] = 1;
        dir[7] = 2;
        dir[8] = b'.';
        dir[12..16].copy_from_slice(&2u32.to_le_bytes());
        dir[16..18].copy_from_slice(&12u16.to_le_bytes());
        dir[18] = 2;
        dir[19] = 2;
        dir[20..22].copy_from_slice(b"..");
        dir[24..28].copy_from_slice(&3u32.to_le_bytes());
        dir[28..30].copy_from_slice(&20u16.to_le_bytes());
        dir[30] = 8;
        dir[31] = 1;
        dir[32..40].copy_from_slice(b"test.txt");
        // fill rest of block so rec_len adds up
        dir[44..46].copy_from_slice(&0u32.to_le_bytes()[..2]);
        dir[44..48].copy_from_slice(&0u32.to_le_bytes());
        dir[48..50].copy_from_slice(&(1024u16 - 44).to_le_bytes());
        dir[50] = 0; // name_len=0 â†’ ignored by parser
    }

    // â”€â”€ File data at block 7 â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    image[7168..7181].copy_from_slice(b"Hello, World!");

    MockBlockDevice::new(image, 512)
}

#[test]
fn extent_root_directory_lists_file() {
    let dev = Arc::new(build_extent_image());
    let parser = Ext4Parser::new(dev).unwrap();
    let entries = parser.root_directory().unwrap();
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert!(
        names.contains(&"test.txt"),
        "expected 'test.txt' via extent tree, got: {names:?}"
    );
}

#[test]
fn extent_file_read_returns_content() {
    let dev = Arc::new(build_extent_image());
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
fn walk_extent_node_bad_magic_returns_error() {
    let dev = Arc::new(build_image());
    let parser = Ext4Parser::new(dev).unwrap();
    // 12 zero bytes â†’ magic = 0x0000, should fail
    let result = parser.walk_extent_node(&[0u8; 12]);
    assert!(result.is_err());
}

/// Build a 12 KiB ext4 image where inode 3 uses double-indirect addressing.
///
/// Block layout (1 KiB blocks):
///  0: unused
///  1: superblock
///  2: GDT
///  3: block bitmap
///  4: inode bitmap
///  5: inode table
///  6: root directory data
///  7: data block A (1024 x 'A')
///  8: data block B (1024 x 'B')
///  9: level-2 indirect block â†’ [7, 8, 0, ...]
/// 10: double-indirect root â†’ [9, 0, ...]
/// 11: padding
fn build_double_indirect_image() -> MockBlockDevice {
    let mut image = vec![0u8; 12288];

    // Superblock
    {
        let sb = &mut image[1024..2048];
        sb[0..4].copy_from_slice(&16u32.to_le_bytes());
        sb[4..8].copy_from_slice(&12u32.to_le_bytes());
        sb[20..24].copy_from_slice(&1u32.to_le_bytes()); // s_first_data_block
        sb[24..28].copy_from_slice(&0u32.to_le_bytes()); // 1 KiB blocks
        sb[32..36].copy_from_slice(&8192u32.to_le_bytes());
        sb[40..44].copy_from_slice(&16u32.to_le_bytes());
        sb[56..58].copy_from_slice(&EXT4_MAGIC.to_le_bytes());
        sb[92..96].copy_from_slice(&0u32.to_le_bytes());
    }

    // GDT
    {
        let gdt = &mut image[2048..2080];
        gdt[0..4].copy_from_slice(&3u32.to_le_bytes());
        gdt[4..8].copy_from_slice(&4u32.to_le_bytes());
        gdt[8..12].copy_from_slice(&5u32.to_le_bytes());
    }

    // Inode 2: root directory
    {
        let off = 5120 + 128;
        let ino = &mut image[off..off + 128];
        ino[0..2].copy_from_slice(&(S_IFDIR | 0x1ED).to_le_bytes());
        ino[4..8].copy_from_slice(&1024u32.to_le_bytes());
        ino[26..28].copy_from_slice(&2u16.to_le_bytes());
        ino[40..44].copy_from_slice(&6u32.to_le_bytes()); // i_block[0] = block 6
    }

    // Inode 3: file using double-indirect
    {
        let off = 5120 + 256;
        let ino = &mut image[off..off + 128];
        ino[0..2].copy_from_slice(&(0x8000u16 | 0x1A4).to_le_bytes());
        ino[4..8].copy_from_slice(&2048u32.to_le_bytes()); // i_size_lo = 2048
        ino[26..28].copy_from_slice(&1u16.to_le_bytes());
        // i_flags = 0 (legacy block map)
        // i_block[12] = 0 (no single-indirect)
        ino[40 + 12 * 4..40 + 13 * 4].copy_from_slice(&0u32.to_le_bytes());
        // i_block[13] = 10 (double-indirect root)
        ino[40 + 13 * 4..40 + 14 * 4].copy_from_slice(&10u32.to_le_bytes());
        // i_block[14] = 0
        ino[40 + 14 * 4..40 + 15 * 4].copy_from_slice(&0u32.to_le_bytes());
    }

    // Root directory data at block 6
    {
        let dir = &mut image[6144..7168];
        dir[0..4].copy_from_slice(&2u32.to_le_bytes());
        dir[4..6].copy_from_slice(&12u16.to_le_bytes());
        dir[6] = 1;
        dir[7] = 2;
        dir[8] = b'.';
        dir[12..16].copy_from_slice(&2u32.to_le_bytes());
        dir[16..18].copy_from_slice(&12u16.to_le_bytes());
        dir[18] = 2;
        dir[19] = 2;
        dir[20..22].copy_from_slice(b"..");
        dir[24..28].copy_from_slice(&3u32.to_le_bytes());
        dir[28..30].copy_from_slice(&20u16.to_le_bytes());
        dir[30] = 8;
        dir[31] = 1;
        dir[32..40].copy_from_slice(b"dblfile.");
        dir[44..48].copy_from_slice(&0u32.to_le_bytes());
        dir[48..50].copy_from_slice(&(1024u16 - 44).to_le_bytes());
        dir[50] = 0;
    }

    // Block 7: data block A â€” 1024 x 'A'
    for b in &mut image[7168..8192] {
        *b = b'A';
    }

    // Block 8: data block B â€” 1024 x 'B'
    for b in &mut image[8192..9216] {
        *b = b'B';
    }

    // Block 9: level-2 indirect â†’ [7, 8, 0, ...]
    {
        let blk = &mut image[9216..10240];
        blk[0..4].copy_from_slice(&7u32.to_le_bytes());
        blk[4..8].copy_from_slice(&8u32.to_le_bytes());
        // rest are zeros
    }

    // Block 10: double-indirect root â†’ [9, 0, ...]
    {
        let blk = &mut image[10240..11264];
        blk[0..4].copy_from_slice(&9u32.to_le_bytes());
        // rest are zeros
    }

    MockBlockDevice::new(image, 512)
}

#[test]
fn double_indirect_file_read() {
    let dev = Arc::new(build_double_indirect_image());
    let parser = Ext4Parser::new(dev).unwrap();

    let entry = FileEntry {
        name: "dblfile.".into(),
        path: "/dblfile.".into(),
        size: 2048,
        is_dir: false,
        is_deleted: false,
        created: None,
        modified: None,
        first_cluster: None,
        mft_record: None,
        inode_number: Some(3),
        data_byte_offset: None,
        recovery_chance: RecoveryChance::Unknown,
    };

    let mut buf = Vec::new();
    let written = parser.read_file(&entry, &mut buf).unwrap();
    assert_eq!(written, 2048, "expected 2048 bytes written");
    assert!(
        buf[..1024].iter().all(|&b| b == b'A'),
        "first 1024 bytes should be 'A'"
    );
    assert!(
        buf[1024..2048].iter().all(|&b| b == b'B'),
        "next 1024 bytes should be 'B'"
    );
}

/// Build a 13 KiB ext4 image where inode 3 uses triple-indirect addressing.
///
/// Block layout (1 KiB blocks):
///  0: unused
///  1: superblock
///  2: GDT
///  3: block bitmap
///  4: inode bitmap
///  5: inode table
///  6: root directory data
///  7: data block C (1024 x 'C')
///  8: data block D (1024 x 'D')
///  9: level-1 indirect â†’ [7, 8, 0, ...]
/// 10: level-2 indirect â†’ [9, 0, ...]
/// 11: triple-indirect root â†’ [10, 0, ...]
/// 12: padding
fn build_triple_indirect_image() -> MockBlockDevice {
    let mut image = vec![0u8; 13312];

    // Superblock
    {
        let sb = &mut image[1024..2048];
        sb[0..4].copy_from_slice(&16u32.to_le_bytes());
        sb[4..8].copy_from_slice(&13u32.to_le_bytes());
        sb[20..24].copy_from_slice(&1u32.to_le_bytes());
        sb[24..28].copy_from_slice(&0u32.to_le_bytes());
        sb[32..36].copy_from_slice(&8192u32.to_le_bytes());
        sb[40..44].copy_from_slice(&16u32.to_le_bytes());
        sb[56..58].copy_from_slice(&EXT4_MAGIC.to_le_bytes());
        sb[92..96].copy_from_slice(&0u32.to_le_bytes());
    }

    // GDT
    {
        let gdt = &mut image[2048..2080];
        gdt[0..4].copy_from_slice(&3u32.to_le_bytes());
        gdt[4..8].copy_from_slice(&4u32.to_le_bytes());
        gdt[8..12].copy_from_slice(&5u32.to_le_bytes());
    }

    // Inode 2: root directory
    {
        let off = 5120 + 128;
        let ino = &mut image[off..off + 128];
        ino[0..2].copy_from_slice(&(S_IFDIR | 0x1ED).to_le_bytes());
        ino[4..8].copy_from_slice(&1024u32.to_le_bytes());
        ino[26..28].copy_from_slice(&2u16.to_le_bytes());
        ino[40..44].copy_from_slice(&6u32.to_le_bytes());
    }

    // Inode 3: file using triple-indirect
    {
        let off = 5120 + 256;
        let ino = &mut image[off..off + 128];
        ino[0..2].copy_from_slice(&(0x8000u16 | 0x1A4).to_le_bytes());
        ino[4..8].copy_from_slice(&2048u32.to_le_bytes());
        ino[26..28].copy_from_slice(&1u16.to_le_bytes());
        // i_block[12] = 0, i_block[13] = 0, i_block[14] = 11
        ino[40 + 12 * 4..40 + 13 * 4].copy_from_slice(&0u32.to_le_bytes());
        ino[40 + 13 * 4..40 + 14 * 4].copy_from_slice(&0u32.to_le_bytes());
        ino[40 + 14 * 4..40 + 15 * 4].copy_from_slice(&11u32.to_le_bytes());
    }

    // Root directory data at block 6
    {
        let dir = &mut image[6144..7168];
        dir[0..4].copy_from_slice(&2u32.to_le_bytes());
        dir[4..6].copy_from_slice(&12u16.to_le_bytes());
        dir[6] = 1;
        dir[7] = 2;
        dir[8] = b'.';
        dir[12..16].copy_from_slice(&2u32.to_le_bytes());
        dir[16..18].copy_from_slice(&12u16.to_le_bytes());
        dir[18] = 2;
        dir[19] = 2;
        dir[20..22].copy_from_slice(b"..");
        dir[24..28].copy_from_slice(&3u32.to_le_bytes());
        dir[28..30].copy_from_slice(&20u16.to_le_bytes());
        dir[30] = 8;
        dir[31] = 1;
        dir[32..40].copy_from_slice(b"trifile.");
        dir[44..48].copy_from_slice(&0u32.to_le_bytes());
        dir[48..50].copy_from_slice(&(1024u16 - 44).to_le_bytes());
        dir[50] = 0;
    }

    // Block 7: data block C â€” 1024 x 'C'
    for b in &mut image[7168..8192] {
        *b = b'C';
    }

    // Block 8: data block D â€” 1024 x 'D'
    for b in &mut image[8192..9216] {
        *b = b'D';
    }

    // Block 9: level-1 indirect â†’ [7, 8, 0, ...]
    {
        let blk = &mut image[9216..10240];
        blk[0..4].copy_from_slice(&7u32.to_le_bytes());
        blk[4..8].copy_from_slice(&8u32.to_le_bytes());
    }

    // Block 10: level-2 indirect â†’ [9, 0, ...]
    {
        let blk = &mut image[10240..11264];
        blk[0..4].copy_from_slice(&9u32.to_le_bytes());
    }

    // Block 11: triple-indirect root â†’ [10, 0, ...]
    {
        let blk = &mut image[11264..12288];
        blk[0..4].copy_from_slice(&10u32.to_le_bytes());
    }

    MockBlockDevice::new(image, 512)
}

#[test]
fn triple_indirect_file_read() {
    let dev = Arc::new(build_triple_indirect_image());
    let parser = Ext4Parser::new(dev).unwrap();

    let entry = FileEntry {
        name: "trifile.".into(),
        path: "/trifile.".into(),
        size: 2048,
        is_dir: false,
        is_deleted: false,
        created: None,
        modified: None,
        first_cluster: None,
        mft_record: None,
        inode_number: Some(3),
        data_byte_offset: None,
        recovery_chance: RecoveryChance::Unknown,
    };

    let mut buf = Vec::new();
    let written = parser.read_file(&entry, &mut buf).unwrap();
    assert_eq!(written, 2048, "expected 2048 bytes written");
    assert!(
        buf[..1024].iter().all(|&b| b == b'C'),
        "first 1024 bytes should be 'C'"
    );
    assert!(
        buf[1024..2048].iter().all(|&b| b == b'D'),
        "next 1024 bytes should be 'D'"
    );
}

/// Feeding a 10-byte device to Ext4Parser::new must return Err, not panic.
#[test]
fn truncated_device_returns_err_not_panic() {
    let dev = Arc::new(MockBlockDevice::new(vec![0u8; 10], 512));
    let result = Ext4Parser::new(dev);
    assert!(result.is_err(), "expected Err on 10-byte device, got Ok");
}

/// A device large enough for the superblock but with an invalid magic number
/// must return Err(InvalidStructure), not panic.
#[test]
fn invalid_magic_returns_err() {
    // Superblock is at offset 1024; 2048 bytes is enough to read it.
    let data = vec![0u8; 2048]; // magic at offset 1024+56 is 0x0000, not 0xEF53
    let dev = Arc::new(MockBlockDevice::new(data, 512));
    let result = Ext4Parser::new(dev);
    assert!(result.is_err(), "expected Err for invalid ext4 magic");
}

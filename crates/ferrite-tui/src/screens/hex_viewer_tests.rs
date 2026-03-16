use std::sync::Arc;

use ferrite_blockdev::MockBlockDevice;

use super::*;

fn make_device() -> Arc<dyn BlockDevice> {
    Arc::new(MockBlockDevice::zeroed(4096, 512))
}

#[test]
fn set_device_resets_to_lba_zero() {
    let mut s = HexViewerState::new();
    s.current_lba = 42;
    s.set_device(make_device());
    assert_eq!(s.current_lba, 0);
}

#[test]
fn g_key_enters_edit_mode() {
    let mut s = HexViewerState::new();
    s.set_device(make_device());
    s.handle_key(KeyCode::Char('g'), KeyModifiers::NONE);
    assert!(s.is_editing());
}

#[test]
fn b_key_enters_offset_edit_mode() {
    let mut s = HexViewerState::new();
    s.set_device(make_device());
    s.handle_key(KeyCode::Char('b'), KeyModifiers::NONE);
    assert!(s.is_editing());
}

#[test]
fn jump_to_byte_offset_sets_lba_and_highlight() {
    // Device: 4096 bytes, 512-byte sectors (8 sectors total).
    let mut s = HexViewerState::new();
    s.set_device(make_device());
    s.jump_to_byte_offset(513); // sector 1, byte 1 within sector
    assert_eq!(s.current_lba, 1);
    assert_eq!(s.highlight_byte, Some(1));
}

#[test]
fn page_up_down_moves_sixteen_sectors() {
    let dev: Arc<dyn BlockDevice> = Arc::new(MockBlockDevice::zeroed(512 * 100, 512));
    let mut s = HexViewerState::new();
    s.set_device(dev);
    s.current_lba = 32;
    s.handle_key(KeyCode::PageUp, KeyModifiers::NONE);
    assert_eq!(s.current_lba, 16);
    s.handle_key(KeyCode::PageDown, KeyModifiers::NONE);
    assert_eq!(s.current_lba, 32);
}

#[test]
fn home_end_navigation() {
    let dev: Arc<dyn BlockDevice> = Arc::new(MockBlockDevice::zeroed(512 * 10, 512));
    let mut s = HexViewerState::new();
    s.set_device(dev);
    s.current_lba = 5;
    s.handle_key(KeyCode::Home, KeyModifiers::NONE);
    assert_eq!(s.current_lba, 0);
    s.handle_key(KeyCode::End, KeyModifiers::NONE);
    assert_eq!(s.current_lba, 9); // last sector of 10-sector device
}

#[test]
fn detect_sector_type_mbr() {
    let mut data = vec![0u8; 512];
    data[510] = 0x55;
    data[511] = 0xAA;
    assert_eq!(
        detect_sector_type(&data, 0),
        Some("MBR — Master Boot Record")
    );
    // Same signature at LBA 1 should NOT be labelled MBR.
    assert_eq!(detect_sector_type(&data, 1), None);
}

#[test]
fn detect_sector_type_gpt_protective_mbr() {
    let mut data = vec![0u8; 512];
    data[450] = 0xEE; // protective MBR partition type
    data[510] = 0x55;
    data[511] = 0xAA;
    assert_eq!(
        detect_sector_type(&data, 0),
        Some("Protective MBR (GPT disk)")
    );
}

#[test]
fn detect_sector_type_ntfs() {
    let mut data = vec![0u8; 512];
    data[3..11].copy_from_slice(b"NTFS    ");
    assert_eq!(
        detect_sector_type(&data, 0),
        Some("NTFS Volume Boot Record")
    );
}

#[test]
fn detect_sector_type_jpeg() {
    let data = vec![0xFF, 0xD8, 0xFF, 0xE1, 0x00, 0x00];
    assert_eq!(detect_sector_type(&data, 5), Some("JPEG Image"));
}

#[test]
fn detect_sector_type_png() {
    let data = b"\x89PNG\r\n\x1a\n".to_vec();
    assert_eq!(detect_sector_type(&data, 0), Some("PNG Image"));
}

#[test]
fn detect_sector_type_zip() {
    let data = b"PK\x03\x04extra".to_vec();
    assert_eq!(
        detect_sector_type(&data, 0),
        Some("ZIP / DOCX / XLSX / PPTX")
    );
}

#[test]
fn detect_sector_type_none_for_zeros() {
    let data = vec![0u8; 512];
    assert_eq!(detect_sector_type(&data, 1), None);
}

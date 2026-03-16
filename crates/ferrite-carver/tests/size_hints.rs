//! Integration tests for size-hint extraction (Linear, Ole2, LinearScaled,
//! SQLite, SevenZip, OggStream).  These exercise the public `Carver::extract`
//! API end-to-end through a `MockBlockDevice`.

use std::sync::Arc;

use ferrite_blockdev::{BlockDevice, MockBlockDevice};
use ferrite_carver::{CarveHit, Carver, CarvingConfig, Signature, SizeHint};

fn device_from(data: Vec<u8>) -> Arc<dyn BlockDevice> {
    Arc::new(MockBlockDevice::new(data, 512))
}

// ── Linear (RIFF-style) ────────────────────────────────────────────────────────

#[test]
fn extract_size_hint_limits_output() {
    // Build a fake RIFF-like file: header says 100 bytes of content.
    // Total file size = 100 + 8 = 108 bytes.
    let mut data = vec![0xAAu8; 4096];
    data[0..4].copy_from_slice(b"RIFF");
    data[4..8].copy_from_slice(&100u32.to_le_bytes());
    data[8..12].copy_from_slice(b"AVI ");

    let dev = device_from(data);
    let sig = Signature {
        name: "AVI".into(),
        extension: "avi".into(),
        header: vec![Some(0x52), Some(0x49), Some(0x46), Some(0x46)],
        footer: vec![],
        footer_last: false,
        max_size: 2_147_483_648,
        size_hint: Some(SizeHint::Linear {
            offset: 4,
            len: 4,
            little_endian: true,
            add: 8,
        }),
        min_size: 0,
        pre_validate: None,
    };
    let hit = CarveHit {
        byte_offset: 0,
        signature: sig,
    };
    let mut out = Vec::new();
    let written = Carver::new(dev, CarvingConfig::default())
        .extract(&hit, &mut out)
        .unwrap();
    assert_eq!(
        written, 108,
        "size_hint should limit extraction to 108 bytes"
    );
}

// ── OLE2 ──────────────────────────────────────────────────────────────────────

#[test]
fn extract_ole2_size_hint_limits_output() {
    // uSectorShift = 9  (sector_size = 512)
    // csectFat     = 2  → 2 × (512/4) = 256 addressable sectors
    // expected max = (256 + 1) × 512 = 131,584 bytes
    let mut data = vec![0u8; 512 * 1024];
    data[0..8].copy_from_slice(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]);
    data[30..32].copy_from_slice(&9u16.to_le_bytes());
    data[44..48].copy_from_slice(&2u32.to_le_bytes());

    let dev = device_from(data);
    let sig = Signature {
        name: "OLE2".into(),
        extension: "ole".into(),
        header: vec![
            Some(0xD0),
            Some(0xCF),
            Some(0x11),
            Some(0xE0),
            Some(0xA1),
            Some(0xB1),
            Some(0x1A),
            Some(0xE1),
        ],
        footer: vec![],
        footer_last: false,
        max_size: 524_288_000,
        size_hint: Some(SizeHint::Ole2),
        min_size: 0,
        pre_validate: None,
    };
    let hit = CarveHit {
        byte_offset: 0,
        signature: sig,
    };
    let mut out = Vec::new();
    let written = Carver::new(dev, CarvingConfig::default())
        .extract(&hit, &mut out)
        .unwrap();
    assert_eq!(
        written, 131_584,
        "OLE2 size_hint gave {written}, expected 131584"
    );
}

// ── LinearScaled (EVTX) ───────────────────────────────────────────────────────

#[test]
fn extract_linear_scaled_size_hint_limits_output() {
    // chunk_count = 3 at offset 42 (u16 LE) → 3 × 65536 + 4096 = 200,704 bytes
    let mut data = vec![0u8; 512 * 1024];
    data[0..7].copy_from_slice(&[0x45, 0x4C, 0x46, 0x49, 0x4C, 0x45, 0x00]);
    data[42..44].copy_from_slice(&3u16.to_le_bytes());

    let dev = device_from(data);
    let sig = Signature {
        name: "EVTX".into(),
        extension: "evtx".into(),
        header: vec![
            Some(0x45),
            Some(0x4C),
            Some(0x46),
            Some(0x49),
            Some(0x4C),
            Some(0x45),
            Some(0x00),
        ],
        footer: vec![],
        footer_last: false,
        max_size: 104_857_600,
        size_hint: Some(SizeHint::LinearScaled {
            offset: 42,
            len: 2,
            little_endian: true,
            scale: 65536,
            add: 4096,
        }),
        min_size: 0,
        pre_validate: None,
    };
    let hit = CarveHit {
        byte_offset: 0,
        signature: sig,
    };
    let mut out = Vec::new();
    let written = Carver::new(dev, CarvingConfig::default())
        .extract(&hit, &mut out)
        .unwrap();
    assert_eq!(
        written, 200_704,
        "EVTX size_hint gave {written}, expected 200704"
    );
}

// ── SQLite ────────────────────────────────────────────────────────────────────

#[test]
fn extract_sqlite_size_hint_limits_output() {
    // page_size = 4096 (u16 BE at offset 16), db_pages = 5 (u32 BE at offset 28)
    // expected = 4096 × 5 = 20480 bytes
    let mut data = vec![0u8; 512 * 1024];
    data[0..16].copy_from_slice(b"SQLite format 3\0");
    data[16..18].copy_from_slice(&4096u16.to_be_bytes());
    data[28..32].copy_from_slice(&5u32.to_be_bytes());

    let dev = device_from(data);
    let sig = Signature {
        name: "SQLite".into(),
        extension: "db".into(),
        header: vec![
            Some(0x53),
            Some(0x51),
            Some(0x4C),
            Some(0x69),
            Some(0x74),
            Some(0x65),
            Some(0x20),
            Some(0x66),
            Some(0x6F),
            Some(0x72),
            Some(0x6D),
            Some(0x61),
            Some(0x74),
            Some(0x20),
            Some(0x33),
            Some(0x00),
        ],
        footer: vec![],
        footer_last: false,
        max_size: 10_737_418_240,
        size_hint: Some(SizeHint::Sqlite),
        min_size: 0,
        pre_validate: None,
    };
    let hit = CarveHit {
        byte_offset: 0,
        signature: sig,
    };
    let mut out = Vec::new();
    let written = Carver::new(dev, CarvingConfig::default())
        .extract(&hit, &mut out)
        .unwrap();
    assert_eq!(
        written, 20_480,
        "SQLite size_hint gave {written}, expected 20480"
    );
}

// ── SevenZip ──────────────────────────────────────────────────────────────────

#[test]
fn extract_seven_zip_size_hint_limits_output() {
    // NextHeaderOffset = 1000 (u64 LE at offset 12)
    // NextHeaderSize   = 200  (u64 LE at offset 20)
    // expected = 32 + 1000 + 200 = 1232 bytes
    let mut data = vec![0u8; 512 * 1024];
    data[0..6].copy_from_slice(&[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C]);
    data[12..20].copy_from_slice(&1000u64.to_le_bytes());
    data[20..28].copy_from_slice(&200u64.to_le_bytes());

    let dev = device_from(data);
    let sig = Signature {
        name: "7-Zip".into(),
        extension: "7z".into(),
        header: vec![
            Some(0x37),
            Some(0x7A),
            Some(0xBC),
            Some(0xAF),
            Some(0x27),
            Some(0x1C),
        ],
        footer: vec![],
        footer_last: false,
        max_size: 524_288_000,
        size_hint: Some(SizeHint::SevenZip),
        min_size: 0,
        pre_validate: None,
    };
    let hit = CarveHit {
        byte_offset: 0,
        signature: sig,
    };
    let mut out = Vec::new();
    let written = Carver::new(dev, CarvingConfig::default())
        .extract(&hit, &mut out)
        .unwrap();
    assert_eq!(
        written, 1_232,
        "7-Zip size_hint gave {written}, expected 1232"
    );
}

// ── OggStream ─────────────────────────────────────────────────────────────────

/// Build a minimal Ogg page.
fn build_ogg_page(header_type: u8, seg_sizes: &[u8]) -> Vec<u8> {
    let mut page = Vec::new();
    page.extend_from_slice(b"OggS");
    page.push(0); // stream_structure_version
    page.push(header_type);
    page.extend_from_slice(&[0u8; 8]); // granule_position
    page.extend_from_slice(&[0u8; 4]); // bitstream_serial_number
    page.extend_from_slice(&[0u8; 4]); // page_sequence_number
    page.extend_from_slice(&[0u8; 4]); // CRC (zeroed)
    page.push(seg_sizes.len() as u8);
    page.extend_from_slice(seg_sizes);
    let data_len: usize = seg_sizes.iter().map(|&b| b as usize).sum();
    page.extend(std::iter::repeat(0xBBu8).take(data_len));
    page
}

#[test]
fn extract_ogg_stream_size_hint_stops_at_eos() {
    let mut data = Vec::new();
    data.extend(build_ogg_page(0x02, &[100])); // BOS
    data.extend(build_ogg_page(0x00, &[50, 50])); // data
    data.extend(build_ogg_page(0x04, &[30])); // EOS
    let expected = data.len();
    data.extend(vec![0xFFu8; 1024]); // padding

    let dev = device_from(data);
    let sig = Signature {
        name: "OGG Media".into(),
        extension: "ogg".into(),
        header: vec![Some(0x4F), Some(0x67), Some(0x67), Some(0x53)],
        footer: vec![],
        footer_last: false,
        max_size: 65536,
        size_hint: Some(SizeHint::OggStream),
        min_size: 0,
        pre_validate: None,
    };
    let hit = CarveHit {
        byte_offset: 0,
        signature: sig,
    };
    let mut out = Vec::new();
    let written = Carver::new(dev, CarvingConfig::default())
        .extract(&hit, &mut out)
        .unwrap();
    assert_eq!(
        written, expected as u64,
        "OggStream: expected {expected} bytes, got {written}"
    );
    assert_eq!(out[written as usize - 1], 0xBB);
}

#[test]
fn extract_ogg_stream_no_eos_falls_back_to_max_size() {
    let mut data = build_ogg_page(0x02, &[100]); // BOS only, no EOS
    data.extend(vec![0xCCu8; 2048]);

    let max_size = 200u64;
    let dev = device_from(data);
    let sig = Signature {
        name: "OGG Media".into(),
        extension: "ogg".into(),
        header: vec![Some(0x4F), Some(0x67), Some(0x67), Some(0x53)],
        footer: vec![],
        footer_last: false,
        max_size,
        size_hint: Some(SizeHint::OggStream),
        min_size: 0,
        pre_validate: None,
    };
    let hit = CarveHit {
        byte_offset: 0,
        signature: sig,
    };
    let mut out = Vec::new();
    let written = Carver::new(dev, CarvingConfig::default())
        .extract(&hit, &mut out)
        .unwrap();
    assert_eq!(
        written, max_size,
        "should fall back to max_size when no EOS found"
    );
}

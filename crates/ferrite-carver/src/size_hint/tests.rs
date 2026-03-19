//! Unit tests for all size-hint variants.

use std::sync::Arc;

use ferrite_blockdev::MockBlockDevice;

use super::*;
use crate::signature::Signature;

fn device_from(data: Vec<u8>) -> Arc<dyn ferrite_blockdev::BlockDevice> {
    Arc::new(MockBlockDevice::new(data, 512))
}

fn dummy_sig() -> Signature {
    Signature {
        name: "Test".into(),
        extension: "bin".into(),
        header: vec![Some(0xFF)],
        footer: vec![],
        footer_last: false,
        max_size: 1024,
        size_hint: None,
        min_size: 0,
        pre_validate: None,
        header_offset: 0,
        min_hit_gap: 0,
        suppress_group: None,
        footer_extra: 0,
    }
}

// ── ISOBMFF tests ──────────────────────────────────────────────────────────

#[test]
fn isobmff_two_box_file() {
    let mut data = vec![0u8; 512];
    data[0..4].copy_from_slice(&24u32.to_be_bytes());
    data[4..8].copy_from_slice(b"ftyp");
    data[24..28].copy_from_slice(&16u32.to_be_bytes());
    data[28..32].copy_from_slice(b"moov");

    let dev = device_from(data);
    let _ = dummy_sig();
    let result = read_size_hint(dev.as_ref(), 0, &SizeHint::Isobmff, u64::MAX);
    assert_eq!(
        result,
        Some(40),
        "expected 40 bytes from two-box ISOBMFF file"
    );
}

#[test]
fn isobmff_invalid_type_stops_walk() {
    let mut data = vec![0u8; 512];
    data[0..4].copy_from_slice(&16u32.to_be_bytes());
    data[4..8].copy_from_slice(b"ftyp");
    data[16..20].copy_from_slice(&16u32.to_be_bytes());
    data[20] = 0x01;
    data[21..24].copy_from_slice(b"bad");

    let dev = device_from(data);
    let result = read_size_hint(dev.as_ref(), 0, &SizeHint::Isobmff, u64::MAX);
    assert_eq!(result, Some(16));
}

#[test]
fn isobmff_size_zero_stops_walk() {
    let mut data = vec![0u8; 512];
    data[0..4].copy_from_slice(&16u32.to_be_bytes());
    data[4..8].copy_from_slice(b"ftyp");
    data[16..20].copy_from_slice(&0u32.to_be_bytes());
    data[20..24].copy_from_slice(b"mdat");

    let dev = device_from(data);
    let result = read_size_hint(dev.as_ref(), 0, &SizeHint::Isobmff, u64::MAX);
    assert_eq!(result, Some(16));
}

#[test]
fn isobmff_no_valid_boxes_returns_none() {
    let data = vec![0u8; 512];
    let dev = device_from(data);
    let result = read_size_hint(dev.as_ref(), 0, &SizeHint::Isobmff, u64::MAX);
    assert_eq!(result, None);
}

// ── TIFF IFD walker tests ──────────────────────────────────────────────────

fn make_tiff_le_single_strip(strip_off: u32, strip_len: u32) -> Vec<u8> {
    let mut data = vec![0u8; 512];
    data[0..2].copy_from_slice(b"II");
    data[2..4].copy_from_slice(&42u16.to_le_bytes());
    data[4..8].copy_from_slice(&8u32.to_le_bytes());

    data[8..10].copy_from_slice(&2u16.to_le_bytes());
    data[10..12].copy_from_slice(&0x0111u16.to_le_bytes());
    data[12..14].copy_from_slice(&4u16.to_le_bytes());
    data[14..18].copy_from_slice(&1u32.to_le_bytes());
    data[18..22].copy_from_slice(&strip_off.to_le_bytes());
    data[22..24].copy_from_slice(&0x0117u16.to_le_bytes());
    data[24..26].copy_from_slice(&4u16.to_le_bytes());
    data[26..30].copy_from_slice(&1u32.to_le_bytes());
    data[30..34].copy_from_slice(&strip_len.to_le_bytes());
    data[34..38].copy_from_slice(&0u32.to_le_bytes());
    data
}

#[test]
fn tiff_single_strip_extent() {
    let data = make_tiff_le_single_strip(100, 1000);
    let dev = device_from(data);
    let result = read_size_hint(dev.as_ref(), 0, &SizeHint::Tiff, u64::MAX);
    assert_eq!(result, Some(1100), "single-strip TIFF extent");
}

#[test]
fn tiff_subifd_extent() {
    let mut data = vec![0u8; 4096];
    data[0..2].copy_from_slice(b"II");
    data[2..4].copy_from_slice(&42u16.to_le_bytes());
    data[4..8].copy_from_slice(&8u32.to_le_bytes());

    data[8..10].copy_from_slice(&1u16.to_le_bytes());
    data[10..12].copy_from_slice(&0x014Au16.to_le_bytes());
    data[12..14].copy_from_slice(&4u16.to_le_bytes());
    data[14..18].copy_from_slice(&1u32.to_le_bytes());
    data[18..22].copy_from_slice(&32u32.to_le_bytes());
    data[22..26].copy_from_slice(&0u32.to_le_bytes());

    data[32..34].copy_from_slice(&2u16.to_le_bytes());
    data[34..36].copy_from_slice(&0x0111u16.to_le_bytes());
    data[36..38].copy_from_slice(&4u16.to_le_bytes());
    data[38..42].copy_from_slice(&1u32.to_le_bytes());
    data[42..46].copy_from_slice(&1024u32.to_le_bytes());
    data[46..48].copy_from_slice(&0x0117u16.to_le_bytes());
    data[48..50].copy_from_slice(&4u16.to_le_bytes());
    data[50..54].copy_from_slice(&1u32.to_le_bytes());
    data[54..58].copy_from_slice(&20000u32.to_le_bytes());
    data[58..62].copy_from_slice(&0u32.to_le_bytes());

    let dev = device_from(data);
    let result = read_size_hint(dev.as_ref(), 0, &SizeHint::Tiff, u64::MAX);
    assert_eq!(result, Some(21024), "SubIFD strip extent");
}

#[test]
fn tiff_invalid_byte_order_returns_none() {
    let mut data = vec![0u8; 64];
    data[0..2].copy_from_slice(b"XX");
    let dev = device_from(data);
    let result = read_size_hint(dev.as_ref(), 0, &SizeHint::Tiff, u64::MAX);
    assert_eq!(result, None);
}

// ── RAF size hint tests ────────────────────────────────────────────────────

#[test]
fn raf_cfa_dominates() {
    let mut data = vec![0u8; 512];
    data[84..88].copy_from_slice(&200u32.to_be_bytes());
    data[88..92].copy_from_slice(&500u32.to_be_bytes());
    data[92..96].copy_from_slice(&1000u32.to_be_bytes());
    data[96..100].copy_from_slice(&5000u32.to_be_bytes());
    let dev = device_from(data);
    let result = read_size_hint(dev.as_ref(), 0, &SizeHint::Raf, u64::MAX);
    assert_eq!(result, Some(6000));
}

#[test]
fn raf_jpeg_dominates() {
    let mut data = vec![0u8; 512];
    data[84..88].copy_from_slice(&5000u32.to_be_bytes());
    data[88..92].copy_from_slice(&15000u32.to_be_bytes());
    data[92..96].copy_from_slice(&100u32.to_be_bytes());
    data[96..100].copy_from_slice(&1000u32.to_be_bytes());
    let dev = device_from(data);
    let result = read_size_hint(dev.as_ref(), 0, &SizeHint::Raf, u64::MAX);
    assert_eq!(result, Some(20000));
}

#[test]
fn raf_all_zero_returns_none() {
    let data = vec![0u8; 512];
    let dev = device_from(data);
    let result = read_size_hint(dev.as_ref(), 0, &SizeHint::Raf, u64::MAX);
    assert_eq!(result, None);
}

// ── MpegTs size-hint tests ─────────────────────────────────────────────────

fn make_ts_stream(n_packets: usize, trailing_garbage: usize) -> Vec<u8> {
    let mut data = vec![0u8; n_packets * 188 + trailing_garbage];
    for i in 0..n_packets {
        data[i * 188] = 0x47;
    }
    data
}

fn make_m2ts_stream(n_packets: usize, trailing_garbage: usize) -> Vec<u8> {
    let mut data = vec![0u8; n_packets * 192 + trailing_garbage];
    for i in 0..n_packets {
        data[i * 192 + 4] = 0x47;
    }
    data
}

#[test]
fn mpeg_ts_exact_stream_size() {
    let data = make_ts_stream(5, 200);
    let dev = device_from(data);
    let hint = SizeHint::MpegTs {
        ts_offset: 0,
        stride: 188,
    };
    let result = read_size_hint(dev.as_ref(), 0, &hint, u64::MAX);
    assert_eq!(result, Some(940), "5 TS packets → 940 bytes");
}

#[test]
fn mpeg_m2ts_exact_stream_size() {
    let data = make_m2ts_stream(3, 100);
    let dev = device_from(data);
    let hint = SizeHint::MpegTs {
        ts_offset: 4,
        stride: 192,
    };
    let result = read_size_hint(dev.as_ref(), 0, &hint, u64::MAX);
    assert_eq!(result, Some(576), "3 M2TS packets → 576 bytes");
}

#[test]
fn mpeg_ts_no_valid_packets_returns_none() {
    let data = vec![0u8; 512];
    let dev = device_from(data);
    let hint = SizeHint::MpegTs {
        ts_offset: 0,
        stride: 188,
    };
    let result = read_size_hint(dev.as_ref(), 0, &hint, u64::MAX);
    assert_eq!(result, None);
}

#[test]
fn mpeg_ts_max_size_caps_scan() {
    let data = make_ts_stream(20, 0);
    let dev = device_from(data);
    let hint = SizeHint::MpegTs {
        ts_offset: 0,
        stride: 188,
    };
    let result = read_size_hint(dev.as_ref(), 0, &hint, 940);
    assert_eq!(result, Some(940), "max_size caps the scan at 5 packets");
}

#[test]
fn mpeg_ts_stops_on_invalid_run() {
    let mut data = vec![0u8; 30 * 188];
    for i in 0..10 {
        data[i * 188] = 0x47;
    }
    for i in 20..30 {
        data[i * 188] = 0x47;
    }
    let dev = device_from(data);
    let hint = SizeHint::MpegTs {
        ts_offset: 0,
        stride: 188,
    };
    let result = read_size_hint(dev.as_ref(), 0, &hint, u64::MAX);
    assert_eq!(result, Some(1880), "stream ends after invalid run");
}

// ── TextBound size-hint tests ─────────────────────────────────────────────

#[test]
fn text_bound_pure_text() {
    let data = b"<?xml version=\"1.0\"?><root>hello</root>".to_vec();
    let dev = device_from(data.clone());
    let result = read_size_hint(dev.as_ref(), 0, &SizeHint::TextBound, 1024);
    assert_eq!(result, Some(data.len() as u64));
}

#[test]
fn text_bound_stops_at_null() {
    let mut data = b"<?xml>content".to_vec();
    let text_len = data.len();
    data.push(0x00);
    data.extend_from_slice(&[0xFF; 100]);
    let dev = device_from(data);
    let result = read_size_hint(dev.as_ref(), 0, &SizeHint::TextBound, 1024);
    assert_eq!(result, Some(text_len as u64));
}

#[test]
fn text_bound_stops_at_sustained_binary() {
    let mut data = b"<?xml>content".to_vec();
    let text_len = data.len();
    // 8 consecutive non-text bytes (0x01–0x08 are control chars, not in text set)
    data.extend_from_slice(&[0x01; 8]);
    data.extend_from_slice(&[0xFF; 100]);
    let dev = device_from(data);
    let result = read_size_hint(dev.as_ref(), 0, &SizeHint::TextBound, 1024);
    assert_eq!(result, Some(text_len as u64));
}

#[test]
fn text_bound_tolerates_isolated_non_text() {
    // A few scattered non-text bytes don't trigger the limit
    let mut data = b"hello".to_vec();
    data.push(0x01); // 1 non-text
    data.extend_from_slice(b"world");
    let dev = device_from(data.clone());
    let result = read_size_hint(dev.as_ref(), 0, &SizeHint::TextBound, 1024);
    assert_eq!(result, Some(data.len() as u64));
}

#[test]
fn text_bound_empty_device_returns_none() {
    let dev = device_from(vec![0x00; 512]);
    let result = read_size_hint(dev.as_ref(), 0, &SizeHint::TextBound, 1024);
    assert_eq!(result, None);
}

#[test]
fn text_bound_respects_max_size() {
    let data = b"A".repeat(10000);
    let dev = device_from(data);
    let result = read_size_hint(dev.as_ref(), 0, &SizeHint::TextBound, 500);
    assert_eq!(result, Some(500));
}

// ── TTF size-hint tests ───────────────────────────────────────────────────

fn make_ttf_font(tables: &[(u32, u32)]) -> Vec<u8> {
    let num_tables = tables.len() as u16;
    let dir_end = 12 + num_tables as usize * 16;
    let max_extent = tables
        .iter()
        .map(|(off, len)| (*off + *len) as usize)
        .max()
        .unwrap_or(dir_end);
    let mut data = vec![0u8; max_extent.max(dir_end) + 64];
    // sfVersion = 0x00010000
    data[0..4].copy_from_slice(&[0x00, 0x01, 0x00, 0x00]);
    data[4..6].copy_from_slice(&num_tables.to_be_bytes());
    // Fill table records
    for (i, (offset, length)) in tables.iter().enumerate() {
        let base = 12 + i * 16;
        data[base..base + 4].copy_from_slice(b"cmap"); // tag
        data[base + 8..base + 12].copy_from_slice(&offset.to_be_bytes());
        data[base + 12..base + 16].copy_from_slice(&length.to_be_bytes());
    }
    data
}

#[test]
fn ttf_two_tables() {
    let data = make_ttf_font(&[(100, 500), (700, 300)]);
    let dev = device_from(data);
    let result = read_size_hint(dev.as_ref(), 0, &SizeHint::Ttf, u64::MAX);
    assert_eq!(result, Some(1000), "max extent = 700 + 300");
}

#[test]
fn ttf_single_table() {
    let data = make_ttf_font(&[(44, 256)]);
    let dev = device_from(data);
    let result = read_size_hint(dev.as_ref(), 0, &SizeHint::Ttf, u64::MAX);
    assert_eq!(result, Some(300));
}

#[test]
fn ttf_no_tables_returns_none() {
    let mut data = vec![0u8; 512];
    data[0..4].copy_from_slice(&[0x00, 0x01, 0x00, 0x00]);
    // numTables = 0 → out of range [1, 100]
    let dev = device_from(data);
    let result = read_size_hint(dev.as_ref(), 0, &SizeHint::Ttf, u64::MAX);
    assert_eq!(result, None);
}

// ── PDF linearized hint ────────────────────────────────────────────────────

#[test]
fn pdf_linearized_returns_l_value() {
    let header = b"%PDF-1.3\r%\xe2\xe3\xcf\xd3\r\n55 0 obj\r<< \r/Linearized 1 \r/O 58 \r/H [ 750 203 ] \r/L 15206 \r/E 3976 \r/N 4 \r/T 13988 \r>> \rendobj\r";
    let mut data = vec![0u8; 512];
    let len = header.len().min(512);
    data[..len].copy_from_slice(&header[..len]);
    let dev = device_from(data);
    let result = read_size_hint(dev.as_ref(), 0, &SizeHint::Pdf, u64::MAX);
    assert_eq!(result, Some(15206));
}

#[test]
fn pdf_non_linearized_returns_none() {
    let header = b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog >>\nendobj\n";
    let mut data = vec![0u8; 512];
    let len = header.len().min(512);
    data[..len].copy_from_slice(&header[..len]);
    let dev = device_from(data);
    let result = read_size_hint(dev.as_ref(), 0, &SizeHint::Pdf, u64::MAX);
    assert_eq!(result, None);
}

#[test]
fn pdf_linearized_no_l_key_returns_none() {
    let header = b"%PDF-1.5\n1 0 obj\n<< /Linearized 1 /O 10 >>\nendobj\n";
    let mut data = vec![0u8; 512];
    let len = header.len().min(512);
    data[..len].copy_from_slice(&header[..len]);
    let dev = device_from(data);
    let result = read_size_hint(dev.as_ref(), 0, &SizeHint::Pdf, u64::MAX);
    assert_eq!(result, None);
}

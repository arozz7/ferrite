//! Pre-extraction format validators.
//!
//! Each [`PreValidate`] variant applies a cheap structural check to the raw
//! bytes at a hit position during scanning.  Hits that fail are discarded
//! before being reported as `CarveHit`s, reducing false positives and avoiding
//! wasted extraction work.
//!
//! All validators follow the same contract:
//! - If the chunk does not contain enough bytes to validate, return `true`
//!   (give benefit of the doubt — the scan has already matched the magic).
//! - Return `false` only when the header bytes are definitively wrong.

// ── Enum ──────────────────────────────────────────────────────────────────────

/// Selects the format-specific validator applied at scan time.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PreValidate {
    /// ZIP local file header: version, compression method, filename plausibility.
    Zip,
    /// JPEG/JFIF: `JFIF\0` identifier at offset 6.
    JpegJfif,
    /// JPEG/Exif: `Exif` identifier at offset 6.
    JpegExif,
    /// PNG: first chunk length == 13 and type == `IHDR`.
    Png,
    /// PDF: version string `-1.x` or `-2.x` at offset 4.
    Pdf,
    /// GIF: byte 4 is `7` or `9`; byte 5 is `a` (GIF87a / GIF89a).
    Gif,
    /// BMP: DIB header size (u32 LE @14) is a known valid value.
    Bmp,
    /// MP3/ID3v2: version in {2,3,4}; flags low nibble zero; syncsafe size.
    Mp3,
    /// MP4/ISOBMFF: ftyp box size in [12, 512]; brand bytes are printable ASCII.
    Mp4,
    /// RAR: type byte (offset 6) is 0x00 (v4) or 0x01 (v5).
    Rar,
    /// 7-Zip: major version (offset 6) is 0x00.
    SevenZip,
    /// SQLite: page size (u16 BE @16) is a power-of-2 in [512, 65536].
    Sqlite,
    /// Matroska/MKV: EBML VINT leading byte (offset 4) is non-zero.
    Mkv,
    /// FLAC: first metadata block type (lower 7 bits @4) is 0 (STREAMINFO).
    Flac,
    /// Windows PE: `e_lfanew` (u32 LE @60) is in [64, 16384].
    Exe,
    /// VMDK: version field (u32 LE @4) is in {1, 2, 3}.
    Vmdk,
    /// Ogg: stream-structure version (offset 4) == 0 and BOS flag (bit 1 @5) set.
    Ogg,
    /// EVTX: MajorVersion (u16 LE @38) == 3.
    Evtx,
    /// PST/OST: `wMagicClient` bytes @8-9 == [0x4D, 0x53].
    Pst,
    /// XML: byte 5 (after `<?xml`) is a space.
    Xml,
    /// HTML: `<!DOCTYPE` is followed by a space then `html` / `HTML`.
    Html,
    /// RTF: byte 6 (after `{\rtf1`) is `\`, space, CR, or LF.
    Rtf,
    /// vCard: `BEGIN:VCARD` is immediately followed by CR or LF.
    Vcard,
    /// iCalendar: `BEGIN:VCALENDAR` is immediately followed by CR or LF.
    Ical,
    /// OLE2: ByteOrder field (u16 LE @28) == 0xFFFE.
    Ole2,
}

impl PreValidate {
    /// Short label used in display / TOML kind strings.
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Zip => "zip",
            Self::JpegJfif => "jpeg_jfif",
            Self::JpegExif => "jpeg_exif",
            Self::Png => "png",
            Self::Pdf => "pdf",
            Self::Gif => "gif",
            Self::Bmp => "bmp",
            Self::Mp3 => "mp3",
            Self::Mp4 => "mp4",
            Self::Rar => "rar",
            Self::SevenZip => "seven_zip",
            Self::Sqlite => "sqlite",
            Self::Mkv => "mkv",
            Self::Flac => "flac",
            Self::Exe => "exe",
            Self::Vmdk => "vmdk",
            Self::Ogg => "ogg",
            Self::Evtx => "evtx",
            Self::Pst => "pst",
            Self::Xml => "xml",
            Self::Html => "html",
            Self::Rtf => "rtf",
            Self::Vcard => "vcard",
            Self::Ical => "ical",
            Self::Ole2 => "ole2",
        }
    }

    /// Parse a TOML kind string into a `PreValidate` variant.
    pub fn from_kind(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "zip" => Some(Self::Zip),
            "jpeg_jfif" => Some(Self::JpegJfif),
            "jpeg_exif" => Some(Self::JpegExif),
            "png" => Some(Self::Png),
            "pdf" => Some(Self::Pdf),
            "gif" => Some(Self::Gif),
            "bmp" => Some(Self::Bmp),
            "mp3" => Some(Self::Mp3),
            "mp4" => Some(Self::Mp4),
            "rar" => Some(Self::Rar),
            "seven_zip" => Some(Self::SevenZip),
            "sqlite" => Some(Self::Sqlite),
            "mkv" => Some(Self::Mkv),
            "flac" => Some(Self::Flac),
            "exe" => Some(Self::Exe),
            "vmdk" => Some(Self::Vmdk),
            "ogg" => Some(Self::Ogg),
            "evtx" => Some(Self::Evtx),
            "pst" => Some(Self::Pst),
            "xml" => Some(Self::Xml),
            "html" => Some(Self::Html),
            "rtf" => Some(Self::Rtf),
            "vcard" => Some(Self::Vcard),
            "ical" => Some(Self::Ical),
            "ole2" => Some(Self::Ole2),
            _ => None,
        }
    }
}

// ── Dispatch ──────────────────────────────────────────────────────────────────

/// Returns `true` if the bytes at `data[pos..]` pass the format-specific
/// structural check for `kind`.
///
/// Returns `true` (accept) when there are not enough bytes available to
/// validate — the scan has already confirmed the magic bytes match.
pub(crate) fn is_valid(kind: &PreValidate, data: &[u8], pos: usize) -> bool {
    match kind {
        PreValidate::Zip => validate_zip(data, pos),
        PreValidate::JpegJfif => validate_jpeg_jfif(data, pos),
        PreValidate::JpegExif => validate_jpeg_exif(data, pos),
        PreValidate::Png => validate_png(data, pos),
        PreValidate::Pdf => validate_pdf(data, pos),
        PreValidate::Gif => validate_gif(data, pos),
        PreValidate::Bmp => validate_bmp(data, pos),
        PreValidate::Mp3 => validate_mp3(data, pos),
        PreValidate::Mp4 => validate_mp4(data, pos),
        PreValidate::Rar => validate_rar(data, pos),
        PreValidate::SevenZip => validate_seven_zip(data, pos),
        PreValidate::Sqlite => validate_sqlite(data, pos),
        PreValidate::Mkv => validate_mkv(data, pos),
        PreValidate::Flac => validate_flac(data, pos),
        PreValidate::Exe => validate_exe(data, pos),
        PreValidate::Vmdk => validate_vmdk(data, pos),
        PreValidate::Ogg => validate_ogg(data, pos),
        PreValidate::Evtx => validate_evtx(data, pos),
        PreValidate::Pst => validate_pst(data, pos),
        PreValidate::Xml => validate_xml(data, pos),
        PreValidate::Html => validate_html(data, pos),
        PreValidate::Rtf => validate_rtf(data, pos),
        PreValidate::Vcard => validate_vcard(data, pos),
        PreValidate::Ical => validate_ical(data, pos),
        PreValidate::Ole2 => validate_ole2(data, pos),
    }
}

// ── Validators ────────────────────────────────────────────────────────────────

/// Inline helper: return `true` (benefit of doubt) when fewer than `need`
/// bytes are available starting at `pos`.
#[inline]
fn need(data: &[u8], pos: usize, need: usize) -> bool {
    pos + need > data.len()
}

fn validate_zip(data: &[u8], pos: usize) -> bool {
    // ZIP local file header — offsets relative to pos:
    //   4-5  version needed (u16 LE)   8-9  compression method (u16 LE)
    //   26-27 filename length (u16 LE)   30+  filename bytes
    if need(data, pos, 30) {
        return true;
    }
    let version = u16::from_le_bytes([data[pos + 4], data[pos + 5]]);
    let method = u16::from_le_bytes([data[pos + 8], data[pos + 9]]);
    let fname_len = u16::from_le_bytes([data[pos + 26], data[pos + 27]]) as usize;
    if version > 63 {
        return false;
    }
    if fname_len == 0 || fname_len > 512 {
        return false;
    }
    const VALID: &[u16] = &[0, 8, 9, 12, 14, 19, 93, 95, 96, 97, 98, 99];
    if !VALID.contains(&method) {
        return false;
    }
    // Reject directory entries (filename ends with '/').
    if pos + 30 + fname_len <= data.len() && data[pos + 30 + fname_len - 1] == b'/' {
        return false;
    }

    // Reject internal ZIP entries.
    //
    // A ZIP archive's Local File Headers (PK\x03\x04) appear at the start of
    // each entry.  Only the FIRST entry is the true archive start; all others
    // are internal.  When a scan chunk contains multiple LFH hits from the same
    // archive, look backward for a preceding LFH with no EOCD (PK\x05\x06)
    // between it and the current position.  If found, this hit is an internal
    // entry — discard it.  (Hits that straddle a chunk boundary may still slip
    // through; those are handled by deduplication at extraction time.)
    if pos >= 4 {
        let lookback = &data[..pos];
        if let Some(prev_lfh) = pk_find_last(lookback, b'\x03', b'\x04') {
            // No EOCD between the previous LFH and us → same archive, internal entry.
            if pk_find_first(&lookback[prev_lfh + 4..], b'\x05', b'\x06').is_none() {
                return false;
            }
        }
    }

    true
}

/// Find the rightmost `PK<b1><b2>` in `data`, returning its byte index.
fn pk_find_last(data: &[u8], b1: u8, b2: u8) -> Option<usize> {
    let mut end = data.len();
    loop {
        let p = memchr::memrchr(b'P', &data[..end])?;
        if p + 3 < data.len() && data[p + 1] == b'K' && data[p + 2] == b1 && data[p + 3] == b2 {
            return Some(p);
        }
        if p == 0 {
            return None;
        }
        end = p;
    }
}

/// Find the first `PK<b1><b2>` in `data`, returning its byte index.
fn pk_find_first(data: &[u8], b1: u8, b2: u8) -> Option<usize> {
    let mut start = 0;
    while start < data.len() {
        let rel = memchr::memchr(b'P', &data[start..])?;
        let abs = start + rel;
        if abs + 3 < data.len()
            && data[abs + 1] == b'K'
            && data[abs + 2] == b1
            && data[abs + 3] == b2
        {
            return Some(abs);
        }
        start = abs + 1;
    }
    None
}

fn validate_jpeg_jfif(data: &[u8], pos: usize) -> bool {
    // FF D8 FF E0 [len_hi] [len_lo] J I F I F 0x00
    if need(data, pos, 11) {
        return true;
    }
    &data[pos + 6..pos + 11] == b"JFIF\x00"
}

fn validate_jpeg_exif(data: &[u8], pos: usize) -> bool {
    // FF D8 FF E1 [len_hi] [len_lo] E x i f
    if need(data, pos, 10) {
        return true;
    }
    &data[pos + 6..pos + 10] == b"Exif"
}

fn validate_png(data: &[u8], pos: usize) -> bool {
    // 8-byte signature, then first chunk: [4-byte length][4-byte type]
    // First chunk MUST be IHDR with length == 13.
    if need(data, pos, 20) {
        return true;
    }
    let chunk_len =
        u32::from_be_bytes([data[pos + 8], data[pos + 9], data[pos + 10], data[pos + 11]]);
    chunk_len == 13 && &data[pos + 12..pos + 16] == b"IHDR"
}

fn validate_pdf(data: &[u8], pos: usize) -> bool {
    // %PDF-[major].[minor] — e.g. "%PDF-1.4" or "%PDF-2.0"
    if need(data, pos, 8) {
        return true;
    }
    data[pos + 4] == b'-'
        && (data[pos + 5] == b'1' || data[pos + 5] == b'2')
        && data[pos + 6] == b'.'
        && data[pos + 7].is_ascii_digit()
}

fn validate_gif(data: &[u8], pos: usize) -> bool {
    // GIF8[7|9]a
    if need(data, pos, 6) {
        return true;
    }
    (data[pos + 4] == b'7' || data[pos + 4] == b'9') && data[pos + 5] == b'a'
}

fn validate_bmp(data: &[u8], pos: usize) -> bool {
    // DIB header size (u32 LE) at offset 14 must be a known value.
    // Known sizes: 12 (CORE), 40 (INFO), 52, 56 (INFO v2/v3), 108 (V4), 124 (V5).
    if need(data, pos, 18) {
        return true;
    }
    let dib_size = u32::from_le_bytes([
        data[pos + 14],
        data[pos + 15],
        data[pos + 16],
        data[pos + 17],
    ]);
    matches!(dib_size, 12 | 40 | 52 | 56 | 108 | 124)
}

fn validate_mp3(data: &[u8], pos: usize) -> bool {
    // ID3 [ver] [rev=0x00] [flags] [size0][size1][size2][size3]
    // version (pos+3) must be 2, 3, or 4.
    // Flags (pos+5) low nibble must be zero (undefined bits).
    // Size bytes (pos+6..pos+10): each must have top bit clear (syncsafe).
    if need(data, pos, 10) {
        return true;
    }
    let version = data[pos + 3];
    if !matches!(version, 2..=4) {
        return false;
    }
    if data[pos + 5] & 0x0F != 0 {
        return false;
    }
    // All 4 syncsafe size bytes must have top bit clear.
    (data[pos + 6] | data[pos + 7] | data[pos + 8] | data[pos + 9]) & 0x80 == 0
}

fn validate_mp4(data: &[u8], pos: usize) -> bool {
    // [ftyp box size: u32 BE][ftyp][brand: 4 bytes]
    // Box size must be in [12, 512] (reasonable for an ftyp box).
    // Brand bytes must be printable ASCII (0x20-0x7E).
    if need(data, pos, 12) {
        return true;
    }
    let box_size = u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
    if !(12..=512).contains(&box_size) {
        return false;
    }
    data[pos + 8..pos + 12]
        .iter()
        .all(|b| (0x20..=0x7E).contains(b))
}

fn validate_rar(data: &[u8], pos: usize) -> bool {
    // Rar! [0x1A][0x07] [type]  — type 0x00 = RAR4, 0x01 = RAR5
    if need(data, pos, 7) {
        return true;
    }
    matches!(data[pos + 6], 0x00 | 0x01)
}

fn validate_seven_zip(data: &[u8], pos: usize) -> bool {
    // 7z BC AF 27 1C [major=0x00] [minor]
    if need(data, pos, 7) {
        return true;
    }
    data[pos + 6] == 0x00
}

fn validate_sqlite(data: &[u8], pos: usize) -> bool {
    // Page size (u16 BE) at header offset 16; value 1 encodes 65536.
    // Valid power-of-2 values: 512, 1024, 2048, 4096, 8192, 16384, 32768, 65536(=1).
    if need(data, pos, 18) {
        return true;
    }
    let page_size = u16::from_be_bytes([data[pos + 16], data[pos + 17]]);
    matches!(
        page_size,
        1 | 512 | 1024 | 2048 | 4096 | 8192 | 16384 | 32768
    )
}

fn validate_mkv(data: &[u8], pos: usize) -> bool {
    // EBML element size is VINT-encoded; its leading byte (pos+4) must be
    // non-zero (a zero byte would indicate an impossibly wide VINT).
    if need(data, pos, 5) {
        return true;
    }
    data[pos + 4] != 0x00
}

fn validate_flac(data: &[u8], pos: usize) -> bool {
    // fLaC [METADATA_BLOCK_HEADER] — lower 7 bits of pos+4 must be 0 (STREAMINFO).
    if need(data, pos, 5) {
        return true;
    }
    data[pos + 4] & 0x7F == 0x00
}

fn validate_exe(data: &[u8], pos: usize) -> bool {
    // e_lfanew (u32 LE) at offset 60: byte offset from start of file to PE header.
    // Must be in [64, 16384] for a plausible PE file.
    if need(data, pos, 64) {
        return true;
    }
    let e_lfanew = u32::from_le_bytes([
        data[pos + 60],
        data[pos + 61],
        data[pos + 62],
        data[pos + 63],
    ]);
    (64..=16384).contains(&e_lfanew)
}

fn validate_vmdk(data: &[u8], pos: usize) -> bool {
    // KDMV [version: u32 LE] — valid versions are 1, 2, 3.
    if need(data, pos, 8) {
        return true;
    }
    let version = u32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]]);
    matches!(version, 1..=3)
}

fn validate_ogg(data: &[u8], pos: usize) -> bool {
    // OggS [version=0x00] [header_type]
    // version MUST be 0; header_type bit 1 (0x02) must be set (BOS page).
    if need(data, pos, 6) {
        return true;
    }
    data[pos + 4] == 0x00 && (data[pos + 5] & 0x02) != 0
}

fn validate_evtx(data: &[u8], pos: usize) -> bool {
    // ElfFile\0 ... MajorVersion (u16 LE) at offset 38 must be 3.
    if need(data, pos, 40) {
        return true;
    }
    let major = u16::from_le_bytes([data[pos + 38], data[pos + 39]]);
    major == 3
}

fn validate_pst(data: &[u8], pos: usize) -> bool {
    // !BDN [dwCRCPartial: 4 bytes] [wMagicClient: u16 LE] = 0x534D
    // Stored LE as bytes [0x4D, 0x53].
    if need(data, pos, 10) {
        return true;
    }
    data[pos + 8] == 0x4D && data[pos + 9] == 0x53
}

fn validate_xml(data: &[u8], pos: usize) -> bool {
    // <?xml followed by a space
    if need(data, pos, 6) {
        return true;
    }
    data[pos + 5] == b' '
}

fn validate_html(data: &[u8], pos: usize) -> bool {
    // <!DOCTYPE followed by ' html' or ' HTML' (case-insensitive)
    if need(data, pos, 14) {
        return true;
    }
    data[pos + 9] == b' '
        && data[pos + 10..pos + 14]
            .iter()
            .zip(b"html")
            .all(|(a, b)| a.to_ascii_lowercase() == *b)
}

fn validate_rtf(data: &[u8], pos: usize) -> bool {
    // {\rtf1 followed by '\', space, CR, or LF
    if need(data, pos, 7) {
        return true;
    }
    matches!(data[pos + 6], b'\\' | b' ' | b'\r' | b'\n')
}

fn validate_vcard(data: &[u8], pos: usize) -> bool {
    // BEGIN:VCARD (11 bytes) followed by CR or LF
    if need(data, pos, 12) {
        return true;
    }
    matches!(data[pos + 11], b'\r' | b'\n')
}

fn validate_ical(data: &[u8], pos: usize) -> bool {
    // BEGIN:VCALENDAR (15 bytes) followed by CR or LF
    if need(data, pos, 16) {
        return true;
    }
    matches!(data[pos + 15], b'\r' | b'\n')
}

fn validate_ole2(data: &[u8], pos: usize) -> bool {
    // D0 CF 11 E0 A1 B1 1A E1 (8 bytes) ... ByteOrder (u16 LE) at offset 28 must be 0xFFFE.
    // Stored LE as bytes [0xFE, 0xFF].
    if need(data, pos, 30) {
        return true;
    }
    data[pos + 28] == 0xFE && data[pos + 29] == 0xFF
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── ZIP ───────────────────────────────────────────────────────────────────

    fn make_zip_lfh(fname: &str, version: u16, method: u16) -> Vec<u8> {
        let fb = fname.as_bytes();
        let mut buf = vec![0u8; 30 + fb.len()];
        buf[0..4].copy_from_slice(b"PK\x03\x04");
        buf[4..6].copy_from_slice(&version.to_le_bytes());
        buf[8..10].copy_from_slice(&method.to_le_bytes());
        buf[26..28].copy_from_slice(&(fb.len() as u16).to_le_bytes());
        buf[30..30 + fb.len()].copy_from_slice(fb);
        buf
    }

    #[test]
    fn zip_directory_entry_rejected() {
        assert!(!validate_zip(&make_zip_lfh("patch/", 20, 8), 0));
    }

    #[test]
    fn zip_file_entry_accepted() {
        assert!(validate_zip(&make_zip_lfh("readme.txt", 20, 8), 0));
    }

    #[test]
    fn zip_invalid_version_rejected() {
        assert!(!validate_zip(&make_zip_lfh("file.bin", 200, 8), 0));
    }

    #[test]
    fn zip_unknown_compression_rejected() {
        assert!(!validate_zip(&make_zip_lfh("file.bin", 20, 255), 0));
    }

    #[test]
    fn zip_first_entry_in_chunk_accepted() {
        // First LFH in a buffer with no preceding PK\x03\x04 — must pass.
        let lfh = make_zip_lfh("file.txt", 20, 8);
        assert!(validate_zip(&lfh, 0));
    }

    #[test]
    fn zip_internal_entry_rejected() {
        // Buffer: [LFH for "a.txt"][some data][LFH for "b.txt"]
        // The second LFH at pos=64 should be rejected as an internal entry.
        let first = make_zip_lfh("a.txt", 20, 8);
        let mut buf = vec![0u8; 64]; // fake compressed data gap
        buf[..first.len()].copy_from_slice(&first);
        let second = make_zip_lfh("b.txt", 20, 8);
        buf.extend_from_slice(&second);
        let pos = 64;
        assert!(!validate_zip(&buf, pos), "internal LFH should be rejected");
    }

    #[test]
    fn zip_new_archive_after_eocd_accepted() {
        // Buffer: [LFH][EOCD][LFH] — second LFH starts a new archive after an EOCD.
        let first = make_zip_lfh("a.txt", 20, 8);
        let eocd =
            b"PK\x05\x06\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00";
        let second = make_zip_lfh("b.txt", 20, 8);
        let mut buf = first.clone();
        buf.extend_from_slice(eocd);
        let pos = buf.len();
        buf.extend_from_slice(&second);
        assert!(
            validate_zip(&buf, pos),
            "LFH after EOCD should be accepted as new archive"
        );
    }

    // ── PNG ───────────────────────────────────────────────────────────────────

    #[test]
    fn png_valid_ihdr_accepted() {
        let mut data = vec![0u8; 24];
        data[0..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        data[8..12].copy_from_slice(&13u32.to_be_bytes()); // IHDR length = 13
        data[12..16].copy_from_slice(b"IHDR");
        assert!(validate_png(&data, 0));
    }

    #[test]
    fn png_wrong_first_chunk_rejected() {
        let mut data = vec![0u8; 24];
        data[0..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        data[8..12].copy_from_slice(&9999u32.to_be_bytes()); // wrong length
        data[12..16].copy_from_slice(b"tEXt"); // wrong type
        assert!(!validate_png(&data, 0));
    }

    // ── EXE ───────────────────────────────────────────────────────────────────

    #[test]
    fn exe_valid_e_lfanew_accepted() {
        let mut data = vec![0u8; 64];
        data[0..4].copy_from_slice(b"MZ\x90\x00");
        data[60..64].copy_from_slice(&128u32.to_le_bytes()); // e_lfanew = 128
        assert!(validate_exe(&data, 0));
    }

    #[test]
    fn exe_zero_e_lfanew_rejected() {
        let mut data = vec![0u8; 64];
        data[0..4].copy_from_slice(b"MZ\x90\x00");
        // e_lfanew = 0 (all zeroes) — invalid
        assert!(!validate_exe(&data, 0));
    }

    // ── MP4 ───────────────────────────────────────────────────────────────────

    #[test]
    fn mp4_valid_ftyp_accepted() {
        let mut data = vec![0u8; 12];
        data[0..4].copy_from_slice(&28u32.to_be_bytes()); // box size = 28
        data[4..8].copy_from_slice(b"ftyp");
        data[8..12].copy_from_slice(b"isom"); // printable ASCII brand
        assert!(validate_mp4(&data, 0));
    }

    #[test]
    fn mp4_non_printable_brand_rejected() {
        let mut data = vec![0u8; 12];
        data[0..4].copy_from_slice(&28u32.to_be_bytes());
        data[4..8].copy_from_slice(b"ftyp");
        data[8] = 0x01; // non-printable
        assert!(!validate_mp4(&data, 0));
    }

    #[test]
    fn mp4_implausible_box_size_rejected() {
        let mut data = vec![0u8; 12];
        data[0..4].copy_from_slice(&1024u32.to_be_bytes()); // too large for ftyp
        data[4..8].copy_from_slice(b"ftyp");
        data[8..12].copy_from_slice(b"isom");
        assert!(!validate_mp4(&data, 0));
    }

    // ── OGG ───────────────────────────────────────────────────────────────────

    #[test]
    fn ogg_bos_page_accepted() {
        let mut data = vec![0u8; 6];
        data[0..4].copy_from_slice(b"OggS");
        data[4] = 0x00; // version
        data[5] = 0x02; // BOS flag
        assert!(validate_ogg(&data, 0));
    }

    #[test]
    fn ogg_continuation_page_rejected() {
        let mut data = vec![0u8; 6];
        data[0..4].copy_from_slice(b"OggS");
        data[4] = 0x00;
        data[5] = 0x00; // no BOS flag — this is a continuation page, not a file start
        assert!(!validate_ogg(&data, 0));
    }

    // ── Benefit-of-doubt ──────────────────────────────────────────────────────

    #[test]
    fn all_validators_pass_on_short_buffer() {
        // Every validator must return true when the buffer is too short.
        let data = vec![0u8; 4];
        assert!(validate_zip(&data, 0));
        assert!(validate_jpeg_jfif(&data, 0));
        assert!(validate_jpeg_exif(&data, 0));
        assert!(validate_png(&data, 0));
        assert!(validate_pdf(&data, 0));
        assert!(validate_gif(&data, 0));
        assert!(validate_bmp(&data, 0));
        assert!(validate_mp3(&data, 0));
        assert!(validate_mp4(&data, 0));
        assert!(validate_rar(&data, 0));
        assert!(validate_seven_zip(&data, 0));
        assert!(validate_sqlite(&data, 0));
        assert!(validate_mkv(&data, 0));
        assert!(validate_flac(&data, 0));
        assert!(validate_exe(&data, 0));
        assert!(validate_vmdk(&data, 0));
        assert!(validate_ogg(&data, 0));
        assert!(validate_evtx(&data, 0));
        assert!(validate_pst(&data, 0));
        assert!(validate_xml(&data, 0));
        assert!(validate_html(&data, 0));
        assert!(validate_rtf(&data, 0));
        assert!(validate_vcard(&data, 0));
        assert!(validate_ical(&data, 0));
        assert!(validate_ole2(&data, 0));
    }
}

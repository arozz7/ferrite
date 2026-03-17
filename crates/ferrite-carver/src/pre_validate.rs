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
    /// Sony ARW: TIFF LE + IFD at offset 8, "SONY" string within first 512 bytes.
    Arw,
    /// Canon CR2: TIFF LE + `CR\x02\x00` at offset 8, plausible IFD offset.
    Cr2,
    /// Panasonic RW2: `II\x55\x00` TIFF variant, plausible IFD offset + entry count.
    Rw2,
    /// Fujifilm RAF: `FUJIFILMCCD-RAW ` + 4-digit version string at offset 16.
    Raf,
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
            Self::Arw => "arw",
            Self::Cr2 => "cr2",
            Self::Rw2 => "rw2",
            Self::Raf => "raf",
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
            "arw" => Some(Self::Arw),
            "cr2" => Some(Self::Cr2),
            "rw2" => Some(Self::Rw2),
            "raf" => Some(Self::Raf),
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
        PreValidate::Arw => validate_arw(data, pos),
        PreValidate::Cr2 => validate_cr2(data, pos),
        PreValidate::Rw2 => validate_rw2(data, pos),
        PreValidate::Raf => validate_raf(data, pos),
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
    if &data[pos + 6..pos + 11] != b"JFIF\x00" {
        return false;
    }
    // Reject embedded thumbnails.
    // A JPEG thumbnail embedded inside an EXIF APP1 segment starts with the
    // same FF D8 magic.  If a preceding SOI (FF D8) appears in the lookback
    // buffer with no matching EOI (FF D9) between it and `pos`, this hit is
    // nested inside an outer JPEG and should be discarded.
    !jpeg_is_embedded(data, pos)
}

fn validate_jpeg_exif(data: &[u8], pos: usize) -> bool {
    // FF D8 FF E1 [len_hi] [len_lo] E x i f
    if need(data, pos, 10) {
        return true;
    }
    if &data[pos + 6..pos + 10] != b"Exif" {
        return false;
    }
    !jpeg_is_embedded(data, pos)
}

/// Returns `true` when `pos` appears to be an embedded JPEG (thumbnail) inside
/// an outer JPEG that is already present in the lookback buffer.
///
/// Strategy: search backward from `pos` for a JPEG SOI marker (`FF D8`).  If
/// one is found with no intervening EOI (`FF D9`) between it and `pos`, the
/// current hit is nested — it is a thumbnail, not an independent file.
///
/// False negatives (outer SOI straddling a chunk boundary) may still produce
/// one extra hit per boundary; those are tolerable and rare.
fn jpeg_is_embedded(data: &[u8], pos: usize) -> bool {
    if pos < 2 {
        return false;
    }
    let lookback = &data[..pos];
    // Walk backward looking for FF D8.
    let mut end = lookback.len();
    loop {
        let Some(ff_pos) = memchr::memrchr(0xFF, &lookback[..end]) else {
            break;
        };
        if ff_pos + 1 < lookback.len() && lookback[ff_pos + 1] == 0xD8 {
            // Found a preceding SOI.  Check for EOI between it and pos.
            let between = &lookback[ff_pos + 2..];
            if !jpeg_has_eoi(between) {
                return true; // no EOI → embedded thumbnail
            }
            // EOI found → the outer JPEG ended before us; we are independent.
            return false;
        }
        if ff_pos == 0 {
            break;
        }
        end = ff_pos;
    }
    false
}

/// Returns `true` if `data` contains a JPEG EOI marker (`FF D9`).
#[inline]
fn jpeg_has_eoi(data: &[u8]) -> bool {
    let mut start = 0;
    while start < data.len() {
        let Some(rel) = memchr::memchr(0xFF, &data[start..]) else {
            break;
        };
        let abs = start + rel;
        if abs + 1 < data.len() && data[abs + 1] == 0xD9 {
            return true;
        }
        start = abs + 1;
    }
    false
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
    // BMP file layout (all offsets from pos):
    //   0-1  "BM"  (magic — already matched by scanner)
    //   2-5  FileSize (u32 LE)
    //   10-13 PixelDataOffset (u32 LE)
    //   14-17 DIB header size (u32 LE)
    if need(data, pos, 18) {
        return true;
    }

    // FileSize must be at least 26 bytes (smallest theoretically valid BMP).
    let file_size = u32::from_le_bytes([
        data[pos + 2],
        data[pos + 3],
        data[pos + 4],
        data[pos + 5],
    ]);
    if file_size < 26 {
        return false;
    }

    // PixelDataOffset must be >= 14 (past the file header) and <= FileSize.
    let pixel_offset = u32::from_le_bytes([
        data[pos + 10],
        data[pos + 11],
        data[pos + 12],
        data[pos + 13],
    ]);
    if pixel_offset < 14 || pixel_offset > file_size {
        return false;
    }

    // DIB header size must be a known value.
    // Known sizes: 12 (CORE), 40 (INFO), 52, 56 (INFO v2/v3), 108 (V4), 124 (V5).
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
    // Layout: [box_size: u32 BE][ftyp][major_brand: 4B][minor_ver: 4B][compat…]
    // `pos` is the start of the ftyp box (the scanner wildcards the 4-byte size).
    if need(data, pos, 12) {
        return true;
    }
    let box_size = u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
    if !(12..=512).contains(&box_size) {
        return false;
    }
    // Major brand (bytes 8-11) must be printable ASCII.
    if !data[pos + 8..pos + 12]
        .iter()
        .all(|b| (0x20..=0x7E).contains(b))
    {
        return false;
    }

    // Look-ahead: verify the box immediately after the ftyp box is also a
    // plausible ISOBMFF box.  In a real MP4 the next box is always one of
    // `moov`, `mdat`, `free`, `skip`, `wide`, `moof`, `meta`, `uuid`, etc.
    // Random H.264/H.265 data inside an mdat region is very unlikely to
    // produce two consecutive valid-looking ISOBMFF boxes.
    let next = pos + box_size as usize;
    if next + 8 <= data.len() {
        let next_size = u32::from_be_bytes([
            data[next],
            data[next + 1],
            data[next + 2],
            data[next + 3],
        ]);
        // Minimum valid box is 8 bytes (size + type with no payload).
        if next_size < 8 {
            return false;
        }
        // Next box type must be 4 ASCII letter/digit/space bytes — the full
        // ISOBMFF type alphabet.  Control characters and high bytes are
        // rejected; punctuation is also rejected to avoid gibberish from
        // encoded video data.
        let next_type = &data[next + 4..next + 8];
        if !next_type
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b' ')
        {
            return false;
        }
    }

    true
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
    // EBML element: \x1A\x45\xDF\xA3 [VINT size] [sub-elements…]
    // The VINT leading byte (pos+4) must be non-zero.
    if need(data, pos, 5) {
        return true;
    }
    if data[pos + 4] == 0x00 {
        return false;
    }

    // Look ahead for the EBML DocType element (ID bytes 0x42 0x82).
    // It is always present within the first 80 bytes of every valid
    // MKV or WebM file and its value is "matroska" or "webm".
    // If we can read the full window and find no DocType, this is not MKV.
    const WINDOW: usize = 80;
    let search_end = data.len().min(pos + WINDOW);
    let have_full_window = search_end == pos + WINDOW;
    let window = &data[pos + 5..search_end];

    let mut i = 0;
    while i + 1 < window.len() {
        if window[i] == 0x42 && window[i + 1] == 0x82 {
            // DocType element found. Next byte is a VINT-encoded length.
            if i + 2 >= window.len() {
                return true; // can't read length — benefit of doubt
            }
            let vint = window[i + 2];
            if vint & 0x80 != 0 {
                // Single-byte VINT: lower 7 bits are the string length.
                let doc_len = (vint & 0x7F) as usize;
                let doc_start = i + 3;
                if doc_start + doc_len <= window.len() {
                    let doc = &window[doc_start..doc_start + doc_len];
                    return doc == b"matroska" || doc == b"webm";
                }
            }
            return true; // DocType found but value straddles boundary
        }
        i += 1;
    }

    // Searched the full window and found no DocType → not MKV/WebM.
    !have_full_window
}

fn validate_flac(data: &[u8], pos: usize) -> bool {
    // fLaC [METADATA_BLOCK_HEADER] — lower 7 bits of pos+4 must be 0 (STREAMINFO).
    if need(data, pos, 5) {
        return true;
    }
    data[pos + 4] & 0x7F == 0x00
}

fn validate_exe(data: &[u8], pos: usize) -> bool {
    // MZ DOS header: e_lfanew (u32 LE) at offset 60 is the byte offset to the
    // PE header.  Must be in [64, 16384] for a plausible PE file.
    if need(data, pos, 64) {
        return true;
    }
    let e_lfanew = u32::from_le_bytes([
        data[pos + 60],
        data[pos + 61],
        data[pos + 62],
        data[pos + 63],
    ]) as usize;
    if !(64..=16384).contains(&e_lfanew) {
        return false;
    }
    // Look-ahead: verify the PE signature (`PE\0\0`) at the e_lfanew offset.
    // This is a near-certain discriminator — random data that both (a) passes
    // the e_lfanew range check AND (b) has `PE\0\0` at that exact variable
    // offset is essentially impossible.  When the PE header falls outside the
    // current scan chunk we give benefit of the doubt.
    let pe_pos = pos + e_lfanew;
    if pe_pos + 4 <= data.len() {
        return &data[pe_pos..pe_pos + 4] == b"PE\x00\x00";
    }
    true
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
    // EVTX file header layout (offsets from pos):
    //   0-7   "ElfFile\0"  (magic — already matched)
    //   32-35  HeaderSize (u32 LE) — always 128
    //   36-37  MinorVersion (u16 LE) — always 1
    //   38-39  MajorVersion (u16 LE) — always 3
    if need(data, pos, 42) {
        return true;
    }
    // HeaderSize is a fixed constant in all known EVTX files.
    let header_size = u32::from_le_bytes([
        data[pos + 32],
        data[pos + 33],
        data[pos + 34],
        data[pos + 35],
    ]);
    if header_size != 128 {
        return false;
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

fn validate_arw(data: &[u8], pos: usize) -> bool {
    // Sony ARW: TIFF little-endian with IFD at offset 8 (anchored in magic).
    // Verify a plausible IFD entry count, then search for "SONY" within the
    // first 512 bytes — the Make IFD value is always in this region.
    if need(data, pos, 10) {
        return true;
    }
    let entry_count = u16::from_le_bytes([data[pos + 8], data[pos + 9]]) as usize;
    if !(5..=50).contains(&entry_count) {
        return false;
    }
    let window_end = data.len().min(pos + 512);
    data[pos..window_end].windows(4).any(|w| w == b"SONY")
}

fn validate_cr2(data: &[u8], pos: usize) -> bool {
    // Canon CR2: TIFF LE magic + IFD offset (wildcard) + CR\x02\x00 at +8.
    // The CR marker bytes are already guaranteed by the scan magic; here we
    // just verify the IFD offset at +4 is plausible (8–4096 bytes).
    if need(data, pos, 8) {
        return true;
    }
    let ifd_offset = u32::from_le_bytes([
        data[pos + 4],
        data[pos + 5],
        data[pos + 6],
        data[pos + 7],
    ]) as usize;
    (8..=4096).contains(&ifd_offset)
}

fn validate_rw2(data: &[u8], pos: usize) -> bool {
    // Panasonic RW2: TIFF variant magic II\x55\x00 (already matched).
    // Verify the IFD0 offset at +4 and a plausible IFD entry count.
    if need(data, pos, 10) {
        return true;
    }
    let ifd_offset = u32::from_le_bytes([
        data[pos + 4],
        data[pos + 5],
        data[pos + 6],
        data[pos + 7],
    ]) as usize;
    if !(8..=4096).contains(&ifd_offset) {
        return false;
    }
    let ifd_pos = pos + ifd_offset;
    if ifd_pos + 2 > data.len() {
        return true; // IFD outside current chunk — benefit of doubt
    }
    let entry_count = u16::from_le_bytes([data[ifd_pos], data[ifd_pos + 1]]) as usize;
    (3..=50).contains(&entry_count)
}

fn validate_raf(data: &[u8], pos: usize) -> bool {
    // Fujifilm RAF: "FUJIFILMCCD-RAW " (16 bytes) + 4-digit version string.
    // Magic anchors on the first 15 bytes; check the space at +15 and that
    // bytes +16..+20 are ASCII decimal digits (e.g. "0201").
    if need(data, pos, 20) {
        return true;
    }
    data[pos + 15] == b' ' && data[pos + 16..pos + 20].iter().all(|b| b.is_ascii_digit())
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

    fn make_pe(e_lfanew: u32) -> Vec<u8> {
        let total = e_lfanew as usize + 4;
        let mut data = vec![0u8; total];
        data[0..4].copy_from_slice(b"MZ\x90\x00");
        data[60..64].copy_from_slice(&e_lfanew.to_le_bytes());
        data[e_lfanew as usize..e_lfanew as usize + 4].copy_from_slice(b"PE\x00\x00");
        data
    }

    #[test]
    fn exe_valid_pe_signature_accepted() {
        // e_lfanew = 128, PE\0\0 present at that offset.
        let data = make_pe(128);
        assert!(validate_exe(&data, 0));
    }

    #[test]
    fn exe_valid_e_lfanew_accepted() {
        // Buffer too short for look-ahead — benefit of doubt.
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

    #[test]
    fn exe_missing_pe_signature_rejected() {
        // e_lfanew = 128, but no PE\0\0 at that offset (random bytes instead).
        let mut data = make_pe(128);
        // Overwrite the PE signature with garbage.
        data[128..132].copy_from_slice(b"NOPE");
        assert!(!validate_exe(&data, 0));
    }

    #[test]
    fn exe_mz_in_binary_data_rejected() {
        // Simulate `MZ` appearing inside a binary data region: e_lfanew
        // resolves to a plausible offset but there is no PE signature there.
        let mut data = vec![0xCC_u8; 300]; // filler
        // Inject fake MZ header at offset 50.
        let pos = 50_usize;
        data[pos] = b'M';
        data[pos + 1] = b'Z';
        data[pos + 60..pos + 64].copy_from_slice(&100u32.to_le_bytes()); // e_lfanew=100
        // No PE\0\0 at pos+100 (just 0xCC filler).
        assert!(!validate_exe(&data, pos));
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

    // ── MP4 ───────────────────────────────────────────────────────────────────

    /// Build a 16-byte ftyp box: [size=16][ftyp][brand][minor_version=0].
    /// The next box appended directly after this will be at offset 16.
    fn make_mp4_ftyp(brand: &[u8; 4]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&16u32.to_be_bytes()); // box_size = 16
        v.extend_from_slice(b"ftyp");
        v.extend_from_slice(brand);
        v.extend_from_slice(&[0u8; 4]); // minor version
        v
    }

    #[test]
    fn mp4_valid_ftyp_followed_by_moov_accepted() {
        let mut data = make_mp4_ftyp(b"isom");
        // Next box: moov, size 100
        data.extend_from_slice(&100u32.to_be_bytes());
        data.extend_from_slice(b"moov");
        assert!(validate_mp4(&data, 0));
    }

    #[test]
    fn mp4_valid_ftyp_followed_by_mdat_accepted() {
        let mut data = make_mp4_ftyp(b"mp42");
        data.extend_from_slice(&1000u32.to_be_bytes());
        data.extend_from_slice(b"mdat");
        assert!(validate_mp4(&data, 0));
    }

    #[test]
    fn mp4_ftyp_followed_by_garbage_rejected() {
        let mut data = make_mp4_ftyp(b"isom");
        // Next "box": size=500 but type has non-alphanumeric bytes (H.264 NAL).
        data.extend_from_slice(&500u32.to_be_bytes());
        data.extend_from_slice(&[0x00, 0x01, 0xB3, 0xFF]); // H.262 start codes
        assert!(!validate_mp4(&data, 0));
    }

    #[test]
    fn mp4_ftyp_followed_by_tiny_next_box_rejected() {
        let mut data = make_mp4_ftyp(b"isom");
        // Next box: size < 8 (impossible for a valid box).
        data.extend_from_slice(&4u32.to_be_bytes());
        data.extend_from_slice(b"moov");
        assert!(!validate_mp4(&data, 0));
    }

    #[test]
    fn mp4_ftyp_no_lookahead_data_accepted() {
        // ftyp box is at the very end of the scan chunk — no lookahead available.
        let data = make_mp4_ftyp(b"isom");
        assert!(validate_mp4(&data, 0));
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

    // ── JPEG ──────────────────────────────────────────────────────────────────

    /// Build a minimal JFIF JPEG header at `pos` inside `buf`.
    fn make_jfif_at(buf: &mut Vec<u8>, pos: usize) {
        // Ensure buf is long enough.
        if buf.len() < pos + 11 {
            buf.resize(pos + 11, 0);
        }
        buf[pos] = 0xFF;
        buf[pos + 1] = 0xD8;
        buf[pos + 2] = 0xFF;
        buf[pos + 3] = 0xE0;
        buf[pos + 4] = 0x00;
        buf[pos + 5] = 0x10;
        buf[pos + 6..pos + 11].copy_from_slice(b"JFIF\x00");
    }

    #[test]
    fn jpeg_jfif_standalone_accepted() {
        // First JPEG in the buffer — no preceding SOI, must be accepted.
        let mut data = vec![0u8; 11];
        make_jfif_at(&mut data, 0);
        assert!(validate_jpeg_jfif(&data, 0));
    }

    #[test]
    fn jpeg_jfif_embedded_thumbnail_rejected() {
        // Outer JPEG SOI at offset 0, then an embedded JFIF JPEG at offset 100
        // with no EOI (FF D9) between them — should be rejected as thumbnail.
        let mut buf = vec![0u8; 120];
        // Outer SOI at 0.
        buf[0] = 0xFF;
        buf[1] = 0xD8;
        // Embedded JFIF at 100 — no FF D9 between 0 and 100.
        make_jfif_at(&mut buf, 100);
        assert!(!validate_jpeg_jfif(&buf, 100));
    }

    #[test]
    fn jpeg_jfif_after_eoi_accepted() {
        // Outer JPEG: SOI at 0, EOI at 50.  New standalone JFIF at 60 — must
        // be accepted because the preceding JPEG is closed.
        let mut buf = vec![0u8; 80];
        // Outer SOI.
        buf[0] = 0xFF;
        buf[1] = 0xD8;
        // Outer EOI.
        buf[50] = 0xFF;
        buf[51] = 0xD9;
        // Standalone JFIF.
        make_jfif_at(&mut buf, 60);
        assert!(validate_jpeg_jfif(&buf, 60));
    }

    #[test]
    fn jpeg_exif_embedded_thumbnail_rejected() {
        // Outer JPEG SOI at 0, Exif JPEG at 200 — no EOI between them.
        let mut buf = vec![0u8; 210];
        buf[0] = 0xFF;
        buf[1] = 0xD8;
        // Exif header at 200.
        buf[200] = 0xFF;
        buf[201] = 0xD8;
        buf[202] = 0xFF;
        buf[203] = 0xE1;
        buf[204] = 0x00;
        buf[205] = 0x20;
        buf[206..210].copy_from_slice(b"Exif");
        assert!(!validate_jpeg_exif(&buf, 200));
    }

    // ── BMP ───────────────────────────────────────────────────────────────────

    fn make_bmp(file_size: u32, pixel_offset: u32, dib_size: u32) -> Vec<u8> {
        let mut data = vec![0u8; 18];
        data[0..2].copy_from_slice(b"BM");
        data[2..6].copy_from_slice(&file_size.to_le_bytes());
        data[10..14].copy_from_slice(&pixel_offset.to_le_bytes());
        data[14..18].copy_from_slice(&dib_size.to_le_bytes());
        data
    }

    #[test]
    fn bmp_valid_accepted() {
        // Typical BMP: 40-byte BITMAPINFOHEADER, pixel data at 54.
        let data = make_bmp(1078, 54, 40);
        assert!(validate_bmp(&data, 0));
    }

    #[test]
    fn bmp_tiny_file_size_rejected() {
        let data = make_bmp(10, 54, 40); // file_size < 26
        assert!(!validate_bmp(&data, 0));
    }

    #[test]
    fn bmp_pixel_offset_past_file_size_rejected() {
        let data = make_bmp(1000, 2000, 40); // pixel_offset > file_size
        assert!(!validate_bmp(&data, 0));
    }

    #[test]
    fn bmp_pixel_offset_before_header_rejected() {
        let data = make_bmp(1000, 4, 40); // pixel_offset < 14
        assert!(!validate_bmp(&data, 0));
    }

    #[test]
    fn bmp_unknown_dib_size_rejected() {
        let data = make_bmp(1000, 54, 99); // 99 is not a known DIB size
        assert!(!validate_bmp(&data, 0));
    }

    // ── EVTX ──────────────────────────────────────────────────────────────────

    fn make_evtx_header() -> Vec<u8> {
        let mut data = vec![0u8; 42];
        data[0..8].copy_from_slice(b"ElfFile\x00");
        // HeaderSize at offset 32 = 128
        data[32..36].copy_from_slice(&128u32.to_le_bytes());
        // MajorVersion at offset 38 = 3
        data[38..40].copy_from_slice(&3u16.to_le_bytes());
        data
    }

    #[test]
    fn evtx_valid_header_accepted() {
        assert!(validate_evtx(&make_evtx_header(), 0));
    }

    #[test]
    fn evtx_wrong_header_size_rejected() {
        let mut data = make_evtx_header();
        data[32..36].copy_from_slice(&64u32.to_le_bytes()); // not 128
        assert!(!validate_evtx(&data, 0));
    }

    #[test]
    fn evtx_wrong_major_version_rejected() {
        let mut data = make_evtx_header();
        data[38..40].copy_from_slice(&2u16.to_le_bytes()); // not 3
        assert!(!validate_evtx(&data, 0));
    }

    // ── MKV ───────────────────────────────────────────────────────────────────

    fn make_mkv_header(doctype: &[u8]) -> Vec<u8> {
        // Minimal EBML header: ID + unknown-size VINT + sub-elements.
        let mut v = Vec::new();
        v.extend_from_slice(b"\x1A\x45\xDF\xA3"); // EBML ID
        v.extend_from_slice(b"\x9F");              // VINT: single-byte size = 31
        // EBMLVersion element (ID 0x4286, value 1).
        v.extend_from_slice(b"\x42\x86\x81\x01");
        // EBMLReadVersion element (ID 0x42F7, value 1).
        v.extend_from_slice(b"\x42\xF7\x81\x01");
        // DocType element: ID 0x4282 + single-byte VINT len + value.
        v.push(0x42);
        v.push(0x82);
        v.push(0x80 | doctype.len() as u8); // VINT
        v.extend_from_slice(doctype);
        v
    }

    #[test]
    fn mkv_matroska_doctype_accepted() {
        let data = make_mkv_header(b"matroska");
        assert!(validate_mkv(&data, 0));
    }

    #[test]
    fn mkv_webm_doctype_accepted() {
        let data = make_mkv_header(b"webm");
        assert!(validate_mkv(&data, 0));
    }

    #[test]
    fn mkv_unknown_doctype_rejected() {
        let data = make_mkv_header(b"divx");
        assert!(!validate_mkv(&data, 0));
    }

    #[test]
    fn mkv_no_doctype_in_full_window_rejected() {
        // 80-byte buffer with valid VINT but no DocType element — rejected.
        let mut data = vec![0x01u8; 80]; // non-zero VINT bytes, no 0x42 0x82
        data[0..4].copy_from_slice(b"\x1A\x45\xDF\xA3");
        data[4] = 0x9F; // valid VINT
        assert!(!validate_mkv(&data, 0));
    }

    #[test]
    fn mkv_short_buffer_benefit_of_doubt() {
        let data = vec![0x1Au8, 0x45, 0xDF, 0xA3, 0x9F]; // 5 bytes, full window not reachable
        assert!(validate_mkv(&data, 0));
    }

    // ── RAW photo formats ─────────────────────────────────────────────────────

    fn make_arw_header(with_sony: bool) -> Vec<u8> {
        // Minimal TIFF LE header: magic + IFD at 8 + entry count.
        let mut v = vec![0u8; 512];
        v[0..4].copy_from_slice(b"II\x2A\x00");         // TIFF LE magic
        v[4..8].copy_from_slice(&8u32.to_le_bytes());    // IFD at offset 8
        v[8..10].copy_from_slice(&12u16.to_le_bytes());  // 12 IFD entries
        if with_sony {
            v[100..104].copy_from_slice(b"SONY");
        }
        v
    }

    #[test]
    fn arw_with_sony_string_accepted() {
        assert!(validate_arw(&make_arw_header(true), 0));
    }

    #[test]
    fn arw_without_sony_string_rejected() {
        assert!(!validate_arw(&make_arw_header(false), 0));
    }

    #[test]
    fn arw_implausible_entry_count_rejected() {
        let mut data = make_arw_header(true);
        data[8..10].copy_from_slice(&200u16.to_le_bytes()); // entry_count=200 > 50
        assert!(!validate_arw(&data, 0));
    }

    #[test]
    fn cr2_plausible_ifd_offset_accepted() {
        // Canon CR2: TIFF LE + IFD at 16 + CR\x02\x00 at +8.
        let mut data = vec![0u8; 12];
        data[0..4].copy_from_slice(b"II\x2A\x00");
        data[4..8].copy_from_slice(&16u32.to_le_bytes()); // IFD at 16
        data[8..12].copy_from_slice(b"CR\x02\x00");
        assert!(validate_cr2(&data, 0));
    }

    #[test]
    fn cr2_zero_ifd_offset_rejected() {
        let mut data = vec![0u8; 12];
        data[0..4].copy_from_slice(b"II\x2A\x00");
        // IFD offset = 0 — invalid
        data[8..12].copy_from_slice(b"CR\x02\x00");
        assert!(!validate_cr2(&data, 0));
    }

    #[test]
    fn rw2_valid_accepted() {
        // Panasonic RW2: II\x55\x00 + IFD at 8 + 10 entries.
        let mut data = vec![0u8; 12];
        data[0..4].copy_from_slice(b"II\x55\x00");
        data[4..8].copy_from_slice(&8u32.to_le_bytes()); // IFD at 8
        data[8..10].copy_from_slice(&10u16.to_le_bytes()); // 10 entries
        assert!(validate_rw2(&data, 0));
    }

    #[test]
    fn rw2_bad_ifd_offset_rejected() {
        let mut data = vec![0u8; 12];
        data[0..4].copy_from_slice(b"II\x55\x00");
        data[4..8].copy_from_slice(&8000u32.to_le_bytes()); // way too large
        assert!(!validate_rw2(&data, 0));
    }

    #[test]
    fn raf_valid_accepted() {
        let mut data = vec![0u8; 20];
        data[0..15].copy_from_slice(b"FUJIFILMCCD-RAW");
        data[15] = b' ';
        data[16..20].copy_from_slice(b"0201"); // version digits
        assert!(validate_raf(&data, 0));
    }

    #[test]
    fn raf_missing_space_rejected() {
        let mut data = vec![0u8; 20];
        data[0..15].copy_from_slice(b"FUJIFILMCCD-RAW");
        data[15] = 0x00; // not a space
        data[16..20].copy_from_slice(b"0201");
        assert!(!validate_raf(&data, 0));
    }

    #[test]
    fn raf_non_digit_version_rejected() {
        let mut data = vec![0u8; 20];
        data[0..15].copy_from_slice(b"FUJIFILMCCD-RAW");
        data[15] = b' ';
        data[16..20].copy_from_slice(b"VX01"); // not all digits
        assert!(!validate_raf(&data, 0));
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
        assert!(validate_arw(&data, 0));
        assert!(validate_cr2(&data, 0));
        assert!(validate_rw2(&data, 0));
        assert!(validate_raf(&data, 0));
    }
}

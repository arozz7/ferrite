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
    /// JPEG starting with DQT (Define Quantization Table) directly after SOI —
    /// no APP0/APP1 header.  DQT segment length (u16 BE @4) must be in [67, 518].
    JpegDqt,
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
    /// Generic TIFF (little-endian `II\x2A\x00`): plausible IFD offset and entry
    /// count.  Explicitly rejects Sony ARW ("SONY" marker), Canon CR2 (`CR\x02\x00`
    /// at offset 8), and Nikon NEF ("NIKON" marker) since those have dedicated
    /// signatures that produce correctly-named output files.
    TiffLe,
    /// Generic TIFF (big-endian `MM\x00\x2A`): plausible IFD offset and entry count.
    TiffBe,
    /// Nikon NEF: TIFF LE file that contains the "NIKON" manufacturer string within
    /// the first 512 bytes of the header.
    Nef,
    /// Apple HEIC / HEIF: ISO Base Media File Format with an `heic` or `heix` major
    /// brand in the `ftyp` box.  Box size must be in [12, 512].
    Heic,
    /// QuickTime MOV: ISOBMFF ftyp box with `qt  ` major brand.
    Mov,
    /// iTunes M4V video: ISOBMFF ftyp box with `M4V ` major brand.
    M4v,
    /// 3GPP / 3GPP2 mobile video: ISOBMFF ftyp box whose major brand starts
    /// with `3gp` or `3g2`.
    ThreeGp,
    /// WebM video: EBML file whose DocType element equals `"webm"` (not
    /// `"matroska"`).
    Webm,
    /// Windows Media Video / ASF: ASF Header Object GUID at offset 0; object
    /// size (u64 LE @16) must be ≥ 30.
    Wmv,
    /// Flash Video: `FLV\x01` header; type-flags reserved bits must be zero;
    /// DataOffset (u32 BE @5) must equal 9.
    Flv,
    /// MPEG-2 / MPEG-1 Program Stream: pack header `00 00 01 BA`; top 2 bits
    /// of byte 4 must be `01` (MPEG-2) or top 4 bits must be `0010` (MPEG-1).
    Mpeg,
    /// WebP image: RIFF container with "WEBP" subtype at bytes 8–11; RIFF size
    /// field (u32 LE @4) must be ≥ 4.
    Webp,
    /// AAC / M4A audio: ISOBMFF ftyp box with `M4A ` major brand; box size in
    /// [12, 512].
    M4a,
    /// GZip compressed file: compression method (byte @2) must be 8 (DEFLATE);
    /// reserved flag bit 5 of byte @3 must be 0 per RFC 1952.
    Gz,
    /// Email message (EML): `From ` header followed by a printable ASCII character.
    Eml,
    /// ELF executable or shared library: EI_CLASS in {1,2}; EI_DATA in {1,2};
    /// EI_VERSION == 1.
    Elf,
    /// Windows Registry Hive (REGF): major version (u32 LE @20) == 1; minor
    /// version (u32 LE @24) in [2, 6].
    Regf,
    /// Adobe Photoshop Document (PSD/PSB): version (u16 BE @4) in {1,2};
    /// number of channels (u16 BE @6) in [1, 56].
    Psd,
    /// VHD Virtual Hard Disk: disk type (u32 BE @60) in {2, 3, 4}.
    Vhd,
    /// VHDX Virtual Hard Disk: 8-byte "vhdxfile" magic is globally unique.
    Vhdx,
    /// QCOW2 virtual disk: version (u32 BE @4) in {2, 3}; cluster_bits
    /// (u32 BE @20) in [9, 21].
    Qcow2,
    /// Standard MIDI File: chunk length (u32 BE @4) == 6; format (u16 BE @8) in {0,1,2}.
    Midi,
    /// AIFF / AIFC: byte @11 is 'F' (AIFF) or 'C' (AIFC); FORM chunk size > 4.
    Aiff,
    /// XZ compressed stream: reserved byte @6 == 0x00; check type byte @7
    /// in {0x00, 0x01, 0x04, 0x0A}.
    Xz,
    /// BZip2: level byte @3 in '1'-'9'; block magic bytes @4-9 equal the BWT pi constant.
    Bzip2,
    /// RealMedia File Format: object version (u16 BE @4) in {0,1}; header size
    /// (u32 BE @6) >= 18.
    RealMedia,
    /// Windows ICO: image count (u16 LE @4) in [1, 200].
    Ico,
    /// Olympus ORF: TIFF LE with RO magic; IFD offset (u32 LE @4) in [8, 4096].
    Orf,
    /// Pentax PEF: TIFF LE file with "PENTAX " string in first 512 bytes.
    Pef,
    /// Mach-O 64-bit: filetype (u32 LE @12) in [1, 12]; ncmds (u32 LE @16) in [1, 512].
    MachO,
    /// Canon CR3: ISOBMFF `ftyp` box with `crx ` brand; box size (u32 BE @0) in [12, 512].
    Cr3,
    /// Sony SR2: TIFF LE with IFD at offset 8; "SONY" string in first 512 bytes; IFD entry
    /// for private tag 0x7200 (SR2Private) present in IFD0 entries.
    Sr2,
    /// EPUB e-book: ZIP container whose first local file entry is named `mimetype` (8 bytes,
    /// uncompressed) and whose content contains `epub+zip`.
    Epub,
    /// OpenDocument (ODT/ODS/ODP/…): ZIP container whose first local file entry is named
    /// `mimetype` and whose content contains `opendocument`.
    Odt,
    /// Outlook MSG: OLE2 compound document; ByteOrder @28 == 0xFFFE and first 4 KiB contains
    /// the MAPI stream name `__substg1.0_`.
    Msg,
    /// WavPack audio: `wvpk` block; ck_size (u32 LE @4) > 0; version (u16 LE @8) in
    /// [0x0402, 0x0410].
    WavPack,
    /// CorelDRAW CDR: RIFF container with `CDR` at bytes 8–10 and a valid version suffix
    /// at byte 11 (e.g. `4`–`9`, `A`–`Z`, space).
    Cdr,
    /// Shockwave Flash (FWS/CWS/ZWS): first byte identifies compression; version byte @3
    /// in [1, 50]; file length (u32 LE @4) >= 8.
    Swf,
    /// Kodak DCR: TIFF LE with `Kodak` or `KODAK` string in first 512 bytes; rejects other
    /// TIFF-based RAW formats.
    Dcr,
    /// Canon CRW: legacy CIFF-based RAW; `HEAPCCDR` string at offset 6 in the first 14 bytes.
    Crw,
    /// Minolta MRW: `\x00MRM` header; first data block tag at offset 8 is `PRD` or `TTW`.
    Mrw,
    /// KeePass 2.x (KDBX): sig1 `03 D9 A2 9A` + sig2 `67 FB 4B B5`; major version @10
    /// (u16 LE) in {3, 4}.
    Kdbx,
    /// KeePass 1.x (KDB): sig1 `03 D9 A2 9A` + sig2 `65 FB 4B B5`; major version @10
    /// (u16 LE) in {1, 2}.
    Kdb,
    /// EnCase EWF/E01 evidence file: `EVF\x09\x0D\x0A\xFF\x00` header; segment number
    /// (u16 LE @8) == 1 (first segment only).
    E01,
    /// PCAP network capture (libpcap): magic `D4 C3 B2 A1` (LE) or `A1 B2 C3 D4` (BE);
    /// major version == 2, minor version == 4.
    Pcap,
    /// Windows Minidump: `MDMP\x93\xA7` header; stream count (u32 LE @8) > 0.
    Dmp,
    /// Apple binary property list: `bplist00` magic; data length ≥ 34 bytes.
    Plist,
    /// MPEG-TS (Transport Stream): sync byte `0x47` at stride 188; at least 3 consecutive
    /// sync bytes at offsets 0, 188, and 376.
    Ts,
    /// Blu-ray M2TS: `0x47` at offset 4 (stride 192); sync bytes at offsets 4, 196, 388.
    M2ts,
    /// LUKS encrypted disk image: `LUKS\xBA\xBE` magic; version (u16 BE @6) in {1, 2}.
    Luks,
    /// Sigma X3F RAW photo: `FOVb` magic; version (u32 LE @4) major byte in {2, 3}.
    X3f,
    /// Monkey's Audio (APE): `MAC ` magic; version (u16 LE @6) in [3930, 4100].
    Ape,
    /// Sun AU audio: `.snd` magic; data_offset (u32 BE @4) ≥ 24; encoding (u32 BE @12) in known
    /// set.
    Au,
    /// TrueType Font (TTF): sfVersion `00 01 00 00`; numTables (u16 BE @4) in [4, 50].
    Ttf,
    /// WOFF web font: `wOFF` magic; flavor in {0x00010000, 0x4F54544F}; length ≥ 44;
    /// numTables in [1, 50].
    Woff,
    /// Microsoft CHM help: `ITSF\x03\x00\x00\x00\x60\x00\x00\x00` 12-byte magic — fully
    /// deterministic, no further validation required.
    Chm,
    /// Blender 3D file: `BLENDER` magic; pointer-size byte @7 in {`-`, `_`}; endian byte @8
    /// in {`v`, `V`}.
    Blend,
    /// Adobe InDesign document: 16-byte GUID magic — globally unique, no further validation.
    Indd,
    /// Windows WTV television recording: 16-byte GUID magic — globally unique, same pattern
    /// as WMV/ASF GUID headers.
    Wtv,
    /// ISO 9660 disc image: `CD001` identifier at the start of the Primary Volume Descriptor
    /// (magic at offset 32769 within the file); version byte @5 must be 1.
    Iso,
    /// DICOM medical image: `DICM` magic at offset 128; data after the magic must be >= 8
    /// bytes to confirm a valid DICOM dataset element tag follows.
    Dicom,
    /// POSIX TAR archive (ustar): `ustar\0` magic at offset 257 (within a 512-byte header
    /// block); version string at offset 263 must be `"00"` or `"  "`.
    Tar,
    /// PHP script: `<?php` opener; byte @5 must be whitespace (space, tab, CR, or LF).
    Php,
    /// Unix shebang script (`#!`): byte @2 must be `/` (interpreter path start).
    /// Extension is fixed as `.sh`; content-based classification is not performed
    /// at scan time.
    Shebang,
    /// JPEG starting with a COM (Comment) marker directly after SOI.
    /// COM segment length (u16 BE @4) must be ≥ 2.
    JpegCom,
    /// Java bytecode class file: major version (u16 BE @6) in [45, 80]
    /// (Java 1 through Java 36, future-proofed).
    JavaClass,
    /// Microsoft Cabinet: reserved1 field (u32 LE @4) must be 0;
    /// cabinet file size (u32 LE @8) must be > 0.
    Cab,
    /// OpenType font with PostScript outlines (`OTTO` magic); numTables
    /// (u16 BE @4) in [1, 50].
    Otf,
    /// WOFF2 web font: flavor in {0x00010000, 0x4F54544F}; numTables
    /// (u16 BE @12) in [1, 50].
    Woff2,
    /// Android Dalvik Executable: version string at bytes 4–7 must be three
    /// ASCII digits followed by a null byte.
    Dex,
    /// Raw AAC/ADTS audio: ADTS layer bits (bits 1–2 of byte 1) must be 0x00;
    /// sampling_freq_index ((byte2 >> 2) & 0x0F) must be in [0, 12].
    Aac,
    /// DjVu document: `AT&TFORM` container; form type at bytes 12–15 must be
    /// one of `DJVU`, `DJVM`, `DJVI`, or `THUM`.
    Djvu,
    /// GIMP XCF image: version string at bytes 10–14 must be `file\0` (very
    /// old) or three ASCII decimal digits followed by `\0`.
    Xcf,
    /// ZSoft PCX image: version @1 in {0,2,3,4,5}; encoding @2 in {0,1};
    /// bitsPerPlane @3 in {1,2,4,8}; xMax >= xMin; yMax >= yMin;
    /// reserved @64 == 0; colorPlanes @65 in {1,3,4}.
    Pcx,
    /// Java Archive (JAR): ZIP container whose first local file entry filename
    /// (bytes 30 to 30+fname_len) starts with `META-INF`.
    Jar,
    /// LZH/LHA archive: method character at offset 3 of the `-lh?-` magic
    /// must be one of `0`–`7`, `d`, or `s`.
    Lzh,
    /// HDF5 scientific data file: superblock version byte @8 must be ≤ 3
    /// (versions 0–3 are the only defined superblock formats).
    Hdf5,
    /// FITS astronomy image: value indicator byte @9 must be space; logical
    /// value byte @29 must be `T` (FITS boolean True).
    Fits,
    /// VirtualBox VDI disk image: image type u32 LE at bytes 8–11 (relative to
    /// the magic at file offset 64) must be 1–4 (normal/fixed/undo/diff).
    Vdi,
    /// Windows Shell Link (LNK): FileAttributes u32 LE @24 must be non-zero
    /// and have no reserved high bits set (bits 16–31 must be 0).
    Lnk,
    /// Windows Prefetch file: bytes 4–7 must equal `SCCA` (the Prefetch
    /// signature that follows the version field in all known versions).
    Prefetch,
    /// Windows legacy Event Log (EVT): MajorVersion u32 LE @8 must be 1 and
    /// MinorVersion u32 LE @12 must be 1.
    Evt,
    /// PEM-encoded certificate / key: byte @10 must be a space (separator
    /// after `-----BEGIN`) and byte @11 must be an ASCII uppercase letter
    /// (start of the type label, e.g. `C` for CERTIFICATE).
    Pem,
    /// Parchive PAR2 recovery set: `packet_length` u64 LE @8 must be ≥ 20
    /// (minimum valid packet payload).
    Par2,
    /// WAV audio (RIFF container): chunk size (u32 LE @4) must be ≥ 36
    /// (minimum valid WAV: `WAVE` + `fmt ` chunk + `data` chunk header).
    Wav,
    /// AVI video (RIFF container): chunk size (u32 LE @4) must be ≥ 12
    /// (minimum: `AVI ` subtype + at least one LIST chunk header).
    Avi,
    /// Python bytecode (.pyc): flags field (u32 LE @4) must be 0–3
    /// (only bits 0–1 are defined; bit 0 = use-hash, bit 1 = checked-hash).
    Pyc,
    /// DPX film image (SDPX big-endian or XPDS little-endian): version ID
    /// string at bytes 8–11 must match `V[12].0` (DPX V1.0 or V2.0).
    Dpx,
    /// OpenEXR HDR image: version byte @4 must be 2; reserved flag bits
    /// @5 (upper nibble) and @6–@7 must all be zero.
    Exr,
}

impl PreValidate {
    /// Short label used in display / TOML kind strings.
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Zip => "zip",
            Self::JpegJfif => "jpeg_jfif",
            Self::JpegExif => "jpeg_exif",
            Self::JpegDqt => "jpeg_dqt",
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
            Self::TiffLe => "tiff_le",
            Self::TiffBe => "tiff_be",
            Self::Nef => "nef",
            Self::Heic => "heic",
            Self::Mov => "mov",
            Self::M4v => "m4v",
            Self::ThreeGp => "3gp",
            Self::Webm => "webm",
            Self::Wmv => "wmv",
            Self::Flv => "flv",
            Self::Mpeg => "mpeg",
            Self::Webp => "webp",
            Self::M4a => "m4a",
            Self::Gz => "gz",
            Self::Eml => "eml",
            Self::Elf => "elf",
            Self::Regf => "regf",
            Self::Psd => "psd",
            Self::Vhd => "vhd",
            Self::Vhdx => "vhdx",
            Self::Qcow2 => "qcow2",
            Self::Midi => "midi",
            Self::Aiff => "aiff",
            Self::Xz => "xz",
            Self::Bzip2 => "bzip2",
            Self::RealMedia => "realmedia",
            Self::Ico => "ico",
            Self::Orf => "orf",
            Self::Pef => "pef",
            Self::MachO => "macho",
            Self::Cr3 => "cr3",
            Self::Sr2 => "sr2",
            Self::Epub => "epub",
            Self::Odt => "odt",
            Self::Msg => "msg",
            Self::WavPack => "wavpack",
            Self::Cdr => "cdr",
            Self::Swf => "swf",
            Self::Dcr => "dcr",
            Self::Crw => "crw",
            Self::Mrw => "mrw",
            Self::Kdbx => "kdbx",
            Self::Kdb => "kdb",
            Self::E01 => "e01",
            Self::Pcap => "pcap",
            Self::Dmp => "dmp",
            Self::Plist => "plist",
            Self::Ts => "ts",
            Self::M2ts => "m2ts",
            Self::Luks => "luks",
            Self::X3f => "x3f",
            Self::Ape => "ape",
            Self::Au => "au",
            Self::Ttf => "ttf",
            Self::Woff => "woff",
            Self::Chm => "chm",
            Self::Blend => "blend",
            Self::Indd => "indd",
            Self::Wtv => "wtv",
            Self::Iso => "iso",
            Self::Dicom => "dicom",
            Self::Tar => "tar",
            Self::Php => "php",
            Self::Shebang => "shebang",
            Self::JpegCom => "jpeg_com",
            Self::JavaClass => "java_class",
            Self::Cab => "cab",
            Self::Otf => "otf",
            Self::Woff2 => "woff2",
            Self::Dex => "dex",
            Self::Aac => "aac",
            Self::Djvu => "djvu",
            Self::Xcf => "xcf",
            Self::Pcx => "pcx",
            Self::Jar => "jar",
            Self::Lzh => "lzh",
            Self::Hdf5 => "hdf5",
            Self::Fits => "fits",
            Self::Vdi => "vdi",
            Self::Lnk => "lnk",
            Self::Prefetch => "prefetch",
            Self::Evt => "evt",
            Self::Pem => "pem",
            Self::Par2 => "par2",
            Self::Wav => "wav",
            Self::Avi => "avi",
            Self::Pyc => "pyc",
            Self::Dpx => "dpx",
            Self::Exr => "exr",
        }
    }

    /// Parse a TOML kind string into a `PreValidate` variant.
    pub fn from_kind(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "zip" => Some(Self::Zip),
            "jpeg_jfif" => Some(Self::JpegJfif),
            "jpeg_exif" => Some(Self::JpegExif),
            "jpeg_dqt" => Some(Self::JpegDqt),
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
            "tiff_le" => Some(Self::TiffLe),
            "tiff_be" => Some(Self::TiffBe),
            "nef" => Some(Self::Nef),
            "heic" => Some(Self::Heic),
            "mov" => Some(Self::Mov),
            "m4v" => Some(Self::M4v),
            "3gp" => Some(Self::ThreeGp),
            "webm" => Some(Self::Webm),
            "wmv" => Some(Self::Wmv),
            "flv" => Some(Self::Flv),
            "mpeg" => Some(Self::Mpeg),
            "webp" => Some(Self::Webp),
            "m4a" => Some(Self::M4a),
            "gz" => Some(Self::Gz),
            "eml" => Some(Self::Eml),
            "elf" => Some(Self::Elf),
            "regf" => Some(Self::Regf),
            "psd" => Some(Self::Psd),
            "vhd" => Some(Self::Vhd),
            "vhdx" => Some(Self::Vhdx),
            "qcow2" => Some(Self::Qcow2),
            "midi" => Some(Self::Midi),
            "aiff" => Some(Self::Aiff),
            "xz" => Some(Self::Xz),
            "bzip2" => Some(Self::Bzip2),
            "realmedia" => Some(Self::RealMedia),
            "ico" => Some(Self::Ico),
            "orf" => Some(Self::Orf),
            "pef" => Some(Self::Pef),
            "macho" => Some(Self::MachO),
            "cr3" => Some(Self::Cr3),
            "sr2" => Some(Self::Sr2),
            "epub" => Some(Self::Epub),
            "odt" => Some(Self::Odt),
            "msg" => Some(Self::Msg),
            "wavpack" => Some(Self::WavPack),
            "cdr" => Some(Self::Cdr),
            "swf" => Some(Self::Swf),
            "dcr" => Some(Self::Dcr),
            "crw" => Some(Self::Crw),
            "mrw" => Some(Self::Mrw),
            "kdbx" => Some(Self::Kdbx),
            "kdb" => Some(Self::Kdb),
            "e01" => Some(Self::E01),
            "pcap" => Some(Self::Pcap),
            "dmp" => Some(Self::Dmp),
            "plist" => Some(Self::Plist),
            "ts" => Some(Self::Ts),
            "m2ts" => Some(Self::M2ts),
            "luks" => Some(Self::Luks),
            "x3f" => Some(Self::X3f),
            "ape" => Some(Self::Ape),
            "au" => Some(Self::Au),
            "ttf" => Some(Self::Ttf),
            "woff" => Some(Self::Woff),
            "chm" => Some(Self::Chm),
            "blend" => Some(Self::Blend),
            "indd" => Some(Self::Indd),
            "wtv" => Some(Self::Wtv),
            "iso" => Some(Self::Iso),
            "dicom" => Some(Self::Dicom),
            "tar" => Some(Self::Tar),
            "php" => Some(Self::Php),
            "shebang" => Some(Self::Shebang),
            "jpeg_com" => Some(Self::JpegCom),
            "java_class" => Some(Self::JavaClass),
            "cab" => Some(Self::Cab),
            "otf" => Some(Self::Otf),
            "woff2" => Some(Self::Woff2),
            "dex" => Some(Self::Dex),
            "aac" => Some(Self::Aac),
            "djvu" => Some(Self::Djvu),
            "xcf" => Some(Self::Xcf),
            "pcx" => Some(Self::Pcx),
            "jar" => Some(Self::Jar),
            "lzh" => Some(Self::Lzh),
            "hdf5" => Some(Self::Hdf5),
            "fits" => Some(Self::Fits),
            "vdi" => Some(Self::Vdi),
            "lnk" => Some(Self::Lnk),
            "prefetch" => Some(Self::Prefetch),
            "evt" => Some(Self::Evt),
            "pem" => Some(Self::Pem),
            "par2" => Some(Self::Par2),
            "wav" => Some(Self::Wav),
            "avi" => Some(Self::Avi),
            "pyc" => Some(Self::Pyc),
            "dpx" => Some(Self::Dpx),
            "exr" => Some(Self::Exr),
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
        PreValidate::JpegDqt => validate_jpeg_dqt(data, pos),
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
        PreValidate::TiffLe => validate_tiff_le(data, pos),
        PreValidate::TiffBe => validate_tiff_be(data, pos),
        PreValidate::Nef => validate_nef(data, pos),
        PreValidate::Heic => validate_heic(data, pos),
        PreValidate::Mov => validate_mov(data, pos),
        PreValidate::M4v => validate_m4v(data, pos),
        PreValidate::ThreeGp => validate_3gp(data, pos),
        PreValidate::Webm => validate_webm(data, pos),
        PreValidate::Wmv => validate_wmv(data, pos),
        PreValidate::Flv => validate_flv(data, pos),
        PreValidate::Mpeg => validate_mpeg(data, pos),
        PreValidate::Webp => validate_webp(data, pos),
        PreValidate::M4a => validate_m4a(data, pos),
        PreValidate::Gz => validate_gz(data, pos),
        PreValidate::Eml => validate_eml(data, pos),
        PreValidate::Elf => validate_elf(data, pos),
        PreValidate::Regf => validate_regf(data, pos),
        PreValidate::Psd => validate_psd(data, pos),
        PreValidate::Vhd => validate_vhd(data, pos),
        PreValidate::Vhdx => validate_vhdx(data, pos),
        PreValidate::Qcow2 => validate_qcow2(data, pos),
        PreValidate::Midi => validate_midi(data, pos),
        PreValidate::Aiff => validate_aiff(data, pos),
        PreValidate::Xz => validate_xz(data, pos),
        PreValidate::Bzip2 => validate_bzip2(data, pos),
        PreValidate::RealMedia => validate_realmedia(data, pos),
        PreValidate::Ico => validate_ico(data, pos),
        PreValidate::Orf => validate_orf(data, pos),
        PreValidate::Pef => validate_pef(data, pos),
        PreValidate::MachO => validate_macho(data, pos),
        PreValidate::Cr3 => validate_cr3(data, pos),
        PreValidate::Sr2 => validate_sr2(data, pos),
        PreValidate::Epub => validate_epub(data, pos),
        PreValidate::Odt => validate_odt(data, pos),
        PreValidate::Msg => validate_msg(data, pos),
        PreValidate::WavPack => validate_wavpack(data, pos),
        PreValidate::Cdr => validate_cdr(data, pos),
        PreValidate::Swf => validate_swf(data, pos),
        PreValidate::Dcr => validate_dcr(data, pos),
        PreValidate::Crw => validate_crw(data, pos),
        PreValidate::Mrw => validate_mrw(data, pos),
        PreValidate::Kdbx => validate_kdbx(data, pos),
        PreValidate::Kdb => validate_kdb(data, pos),
        PreValidate::E01 => validate_e01(data, pos),
        PreValidate::Pcap => validate_pcap(data, pos),
        PreValidate::Dmp => validate_dmp(data, pos),
        PreValidate::Plist => validate_plist(data, pos),
        PreValidate::Ts => validate_ts(data, pos),
        PreValidate::M2ts => validate_m2ts(data, pos),
        PreValidate::Luks => validate_luks(data, pos),
        PreValidate::X3f => validate_x3f(data, pos),
        PreValidate::Ape => validate_ape(data, pos),
        PreValidate::Au => validate_au(data, pos),
        PreValidate::Ttf => validate_ttf(data, pos),
        PreValidate::Woff => validate_woff(data, pos),
        PreValidate::Chm => validate_chm(data, pos),
        PreValidate::Blend => validate_blend(data, pos),
        PreValidate::Indd => validate_indd(data, pos),
        PreValidate::Wtv => validate_wtv(data, pos),
        PreValidate::Iso => validate_iso(data, pos),
        PreValidate::Dicom => validate_dicom(data, pos),
        PreValidate::Tar => validate_tar(data, pos),
        PreValidate::Php => validate_php(data, pos),
        PreValidate::Shebang => validate_shebang(data, pos),
        PreValidate::JpegCom => validate_jpeg_com(data, pos),
        PreValidate::JavaClass => validate_java_class(data, pos),
        PreValidate::Cab => validate_cab(data, pos),
        PreValidate::Otf => validate_otf(data, pos),
        PreValidate::Woff2 => validate_woff2(data, pos),
        PreValidate::Dex => validate_dex(data, pos),
        PreValidate::Aac => validate_aac(data, pos),
        PreValidate::Djvu => validate_djvu(data, pos),
        PreValidate::Xcf => validate_xcf(data, pos),
        PreValidate::Pcx => validate_pcx(data, pos),
        PreValidate::Jar => validate_jar(data, pos),
        PreValidate::Lzh => validate_lzh(data, pos),
        PreValidate::Hdf5 => validate_hdf5(data, pos),
        PreValidate::Fits => validate_fits(data, pos),
        PreValidate::Vdi => validate_vdi(data, pos),
        PreValidate::Lnk => validate_lnk(data, pos),
        PreValidate::Prefetch => validate_prefetch(data, pos),
        PreValidate::Evt => validate_evt(data, pos),
        PreValidate::Pem => validate_pem(data, pos),
        PreValidate::Par2 => validate_par2(data, pos),
        PreValidate::Wav => validate_wav(data, pos),
        PreValidate::Avi => validate_avi(data, pos),
        PreValidate::Pyc => validate_pyc(data, pos),
        PreValidate::Dpx => validate_dpx(data, pos),
        PreValidate::Exr => validate_exr(data, pos),
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

fn validate_jpeg_dqt(data: &[u8], pos: usize) -> bool {
    // FF D8 FF DB [len_hi] [len_lo] — JPEG starting directly with a DQT
    // (Define Quantization Table) segment, no APP0/APP1 header.
    // DQT segment length (u16 BE at pos+4..+6) includes its own 2 length
    // bytes: min 67 (one 8-bit table), max 518 (four 16-bit tables).
    if need(data, pos, 6) {
        return true;
    }
    let dqt_len = u16::from_be_bytes([data[pos + 4], data[pos + 5]]) as usize;
    if !(67..=518).contains(&dqt_len) {
        return false;
    }
    !jpeg_is_embedded(data, pos)
}

fn validate_jpeg_com(data: &[u8], pos: usize) -> bool {
    // FF D8 FF FE [len_hi] [len_lo] — JPEG starting with a COM (Comment) segment.
    // COM segment length (u16 BE at pos+4..+6) includes the 2 length bytes:
    // minimum valid length is 2 (empty comment).
    if need(data, pos, 6) {
        return true;
    }
    let com_len = u16::from_be_bytes([data[pos + 4], data[pos + 5]]) as usize;
    if com_len < 2 {
        return false;
    }
    !jpeg_is_embedded(data, pos)
}

fn validate_java_class(data: &[u8], pos: usize) -> bool {
    // CA FE BA BE [minor_hi] [minor_lo] [major_hi] [major_lo]
    // Major version: Java 1 = 45, Java 24 = 68; allow up to 80 for future versions.
    if need(data, pos, 8) {
        return true;
    }
    let major = u16::from_be_bytes([data[pos + 6], data[pos + 7]]);
    (45..=80).contains(&major)
}

fn validate_cab(data: &[u8], pos: usize) -> bool {
    // MSCF + reserved1 (u32 LE @4, must be 0) + cabinet_size (u32 LE @8, must be > 0)
    if need(data, pos, 12) {
        return true;
    }
    let reserved1 =
        u32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]]);
    if reserved1 != 0 {
        return false;
    }
    let cab_size =
        u32::from_le_bytes([data[pos + 8], data[pos + 9], data[pos + 10], data[pos + 11]]);
    cab_size > 0
}

fn validate_otf(data: &[u8], pos: usize) -> bool {
    // OTTO + numTables (u16 BE @4) in [1, 50]
    if need(data, pos, 6) {
        return true;
    }
    let num_tables = u16::from_be_bytes([data[pos + 4], data[pos + 5]]);
    (1..=50).contains(&num_tables)
}

fn validate_woff2(data: &[u8], pos: usize) -> bool {
    // wOF2 + flavor (u32 BE @4) in {0x00010000, 0x4F54544F} + numTables (u16 BE @12) in [1, 50]
    if need(data, pos, 14) {
        return true;
    }
    let flavor = u32::from_be_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]]);
    if flavor != 0x0001_0000 && flavor != 0x4F54_544F {
        return false;
    }
    let num_tables = u16::from_be_bytes([data[pos + 12], data[pos + 13]]);
    (1..=50).contains(&num_tables)
}

fn validate_dex(data: &[u8], pos: usize) -> bool {
    // dex\n + version (3 ASCII digits) + null
    // e.g. "035\0", "036\0", "037\0", "038\0", "039\0", "040\0"
    if need(data, pos, 8) {
        return true;
    }
    let v = &data[pos + 4..pos + 8];
    v[0].is_ascii_digit() && v[1].is_ascii_digit() && v[2].is_ascii_digit() && v[3] == 0x00
}

fn validate_vdi(data: &[u8], pos: usize) -> bool {
    // pos points to the VDI magic `7F 10 DA BE` at file offset 64 (header_offset=64).
    // The VDI pre-header is: 64-byte description + 4-byte magic + 4-byte version.
    // The main header starts immediately after (file offset 72 = pos+8).
    // uImageType is the first field of the main header (u32 LE at pos+8).
    if need(data, pos, 12) {
        return true;
    }
    let image_type =
        u32::from_le_bytes([data[pos + 8], data[pos + 9], data[pos + 10], data[pos + 11]]);
    // 1=normal/dynamic, 2=fixed, 3=undo, 4=diff — all are valid VDI images
    matches!(image_type, 1..=4)
}

fn validate_lnk(data: &[u8], pos: usize) -> bool {
    // pos points to the 20-byte LNK magic (HeaderSize + full Shell Link CLSID).
    // FileAttributes (u32 LE) is at bytes 24–27.
    if need(data, pos, 28) {
        return true;
    }
    let attrs = u32::from_le_bytes([
        data[pos + 24],
        data[pos + 25],
        data[pos + 26],
        data[pos + 27],
    ]);
    // FileAttributes must be non-zero (every file has at least one attribute set)
    // and must not use reserved high bits (bits 16–31).
    attrs != 0 && attrs & 0xFFFF_0000 == 0
}

fn validate_prefetch(data: &[u8], pos: usize) -> bool {
    // Windows Prefetch header: Version (u32 LE @0) + Signature "SCCA" @4–7.
    // pos already points to the version field; we just verify the SCCA signature.
    if need(data, pos, 8) {
        return true;
    }
    &data[pos + 4..pos + 8] == b"SCCA"
}

fn validate_evt(data: &[u8], pos: usize) -> bool {
    // Legacy Windows Event Log (EVT) header:
    //   @0–3:  HeaderSize = 48  (already matched by magic)
    //   @4–7:  Signature "LfLe" (already matched by magic)
    //   @8–11: MajorVersion (u32 LE) — must be 1
    //   @12–15: MinorVersion (u32 LE) — must be 1
    if need(data, pos, 16) {
        return true;
    }
    let major = u32::from_le_bytes([data[pos + 8], data[pos + 9], data[pos + 10], data[pos + 11]]);
    let minor = u32::from_le_bytes([
        data[pos + 12],
        data[pos + 13],
        data[pos + 14],
        data[pos + 15],
    ]);
    major == 1 && minor == 1
}

fn validate_pem(data: &[u8], pos: usize) -> bool {
    // PEM format: "-----BEGIN TYPE-----\n..."
    // After the 10-byte magic "-----BEGIN":
    //   @10: must be a space (separator before the type label)
    //   @11: must be an ASCII uppercase letter (start of label, e.g. 'C' for CERTIFICATE)
    if need(data, pos, 12) {
        return true;
    }
    data[pos + 10] == b' ' && data[pos + 11].is_ascii_uppercase()
}

fn validate_par2(data: &[u8], pos: usize) -> bool {
    // PAR2 packet structure (offsets relative to pos):
    //   0–7   magic   "PAR2\0PKT" (8 bytes, already matched by header)
    //   8–15  packet_length (u64 LE) — total length of this packet including header
    // Minimum valid packet: 8-byte magic + 8-byte length + at least one byte = 17 B,
    // but the spec states header alone is 64 bytes, so require length ≥ 64.
    if need(data, pos, 16) {
        return true;
    }
    let pkt_len = u64::from_le_bytes([
        data[pos + 8],
        data[pos + 9],
        data[pos + 10],
        data[pos + 11],
        data[pos + 12],
        data[pos + 13],
        data[pos + 14],
        data[pos + 15],
    ]);
    pkt_len >= 64
}

fn validate_wav(data: &[u8], pos: usize) -> bool {
    // RIFF-WAV layout (offsets relative to pos):
    //   0–3   "RIFF" (already matched)
    //   4–7   chunk size (u32 LE) = file size - 8
    //   8–11  "WAVE" (already matched by 12-byte header pattern)
    // Minimum valid WAV: RIFF header + fmt chunk (24 bytes) + data chunk header (8 bytes)
    //   → chunk_size ≥ 36.
    if need(data, pos, 8) {
        return true;
    }
    let chunk_size =
        u32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]]);
    chunk_size >= 36
}

fn validate_avi(data: &[u8], pos: usize) -> bool {
    // RIFF-AVI layout (offsets relative to pos):
    //   0–3   "RIFF" (already matched)
    //   4–7   chunk size (u32 LE) = file size - 8
    //   8–11  "AVI " (already matched by 12-byte header pattern)
    // Minimum valid AVI: RIFF header + at least one LIST chunk header (12 bytes)
    //   → chunk_size ≥ 12.
    if need(data, pos, 8) {
        return true;
    }
    let chunk_size =
        u32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]]);
    chunk_size >= 12
}

fn validate_pyc(data: &[u8], pos: usize) -> bool {
    // Python bytecode layout (offsets relative to pos):
    //   0–3   version magic (already matched, e.g. 0x330D0D0A for 3.6)
    //   4–7   flags (u32 LE): only bits 0–1 are defined (use-hash and checked-hash);
    //          all other bits must be zero.
    if need(data, pos, 8) {
        return true;
    }
    let flags = u32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]]);
    flags <= 3
}

fn validate_dpx(data: &[u8], pos: usize) -> bool {
    // DPX layout (SMPTE 268M, offsets relative to pos):
    //   0–3   magic "SDPX" or "XPDS" (already matched)
    //   4–7   image_offset (u32) — not validated here
    //   8–15  version ID string (8-byte null-padded ASCII): "V1.0\0…" or "V2.0\0…"
    // Only DPX versions 1.0 and 2.0 are in common use.
    if need(data, pos, 12) {
        return true;
    }
    let v = &data[pos + 8..pos + 12];
    v[0] == b'V' && v[1].is_ascii_digit() && v[2] == b'.' && v[3] == b'0'
}

fn validate_exr(data: &[u8], pos: usize) -> bool {
    // OpenEXR layout (offsets relative to pos):
    //   0–3   magic 0x762F3101 (already matched)
    //   4–7   version (u32 LE): bits 0–7 = version number (must be 2);
    //          bits 8–11 = flags (tile/longnames/multipart/deepdata);
    //          bits 12–31 = reserved (must be 0).
    // → byte @4 == 2, upper nibble of byte @5 == 0, bytes @6 and @7 == 0.
    if need(data, pos, 8) {
        return true;
    }
    data[pos + 4] == 2 && (data[pos + 5] & 0xF0) == 0 && data[pos + 6] == 0 && data[pos + 7] == 0
}

fn validate_jar(data: &[u8], pos: usize) -> bool {
    // ZIP LFH layout (offsets relative to pos):
    //   26–27  filename length (u16 LE)
    //   30+    filename bytes
    // JAR files must have their first local file entry filename start with "META-INF".
    if need(data, pos, 30) {
        return true;
    }
    let fname_len = u16::from_le_bytes([data[pos + 26], data[pos + 27]]) as usize;
    if fname_len == 0 || fname_len > 256 {
        return false;
    }
    if need(data, pos, 30 + fname_len) {
        return true;
    }
    data[pos + 30..pos + 30 + fname_len].starts_with(b"META-INF")
}

fn validate_lzh(data: &[u8], pos: usize) -> bool {
    // pos points to the "-lh?-" magic (header_offset = 2, so actual file starts
    // at pos - 2 in the byte stream).  The method character at pos+3 must be a
    // recognised LZH compression method.
    if need(data, pos, 5) {
        return true;
    }
    let method = data[pos + 3];
    matches!(method, b'0'..=b'7' | b'd' | b's')
}

fn validate_hdf5(data: &[u8], pos: usize) -> bool {
    // HDF5 superblock version byte immediately follows the 8-byte signature.
    // Only versions 0–3 are defined; anything higher is invalid or future.
    if need(data, pos, 9) {
        return true;
    }
    data[pos + 8] <= 3
}

fn validate_fits(data: &[u8], pos: usize) -> bool {
    // FITS keyword card: "SIMPLE  =" (bytes 0–8) + value indicator (byte 9 = space)
    // + value field right-justified in bytes 10–29, where 'T' (logical True)
    // must appear at the last value position (byte 29).
    if need(data, pos, 30) {
        return true;
    }
    // Byte @9 must be a space (FITS value indicator separator)
    if data[pos + 9] != b' ' {
        return false;
    }
    // The logical value 'T' must be at column 30 (byte index 29)
    data[pos + 29] == b'T'
}

fn validate_aac(data: &[u8], pos: usize) -> bool {
    // ADTS frame header layout (bytes relative to pos):
    //   0:   0xFF (sync high — already matched by magic)
    //   1:   sync[3:0] + ID + layer[1:0] + protection_absent
    //        layer bits (1–2) MUST be 00 for AAC ADTS (not MPEG audio layers)
    //   2:   profile[1:0] + sampling_freq_idx[3:0] + private + channel_cfg[2]
    //        sampling_freq_idx in [0, 12]; 13–15 are reserved/invalid
    //   3–5: channel_cfg + copyright fields + aac_frame_length[12:0] (13 bits)
    //        Minimum valid frame length = 7 (header only, protection_absent=1).
    //        Maximum plausible = 8191 (max 13-bit value per spec).
    if need(data, pos, 3) {
        return true;
    }
    let b1 = data[pos + 1];
    let b2 = data[pos + 2];
    // Layer must be 00 (bits 1–2 of byte 1)
    if b1 & 0x06 != 0x00 {
        return false;
    }
    // sampling_freq_index (bits 5–2 of byte 2) must be in [0, 12]
    let sfi = (b2 >> 2) & 0x0F;
    if sfi > 12 {
        return false;
    }
    // If we have enough bytes, also validate the 13-bit frame-length field.
    // This eliminates hits where bytes 0–2 happen to look like an ADTS header
    // but are part of unrelated binary data.
    if !need(data, pos, 6) {
        let frame_len = ((data[pos + 3] & 0x03) as u32) << 11
            | (data[pos + 4] as u32) << 3
            | (data[pos + 5] as u32) >> 5;
        if frame_len < 7 || frame_len > 8191 {
            return false;
        }
    }
    true
}

fn validate_djvu(data: &[u8], pos: usize) -> bool {
    // "AT&TFORM" (8 bytes) + u32 BE size (4 bytes) + form type (4 bytes) = 16 bytes
    // Form type at bytes 12–15 must be one of the known DjVu types.
    if need(data, pos, 16) {
        return true;
    }
    let form_type = &data[pos + 12..pos + 16];
    matches!(form_type, b"DJVU" | b"DJVM" | b"DJVI" | b"THUM")
}

fn validate_xcf(data: &[u8], pos: usize) -> bool {
    // "gimp xcf v" (10 bytes) + version string.
    // Version is either "file\0" (legacy) or exactly 3 ASCII decimal digits + "\0"
    // (e.g. "001\0" through "019\0").
    if need(data, pos, 15) {
        return true;
    }
    let ver = &data[pos + 10..pos + 15];
    // Legacy format: "file\0"
    if ver == b"file\0" {
        return true;
    }
    // Modern format: 3 decimal digits + null
    ver[0].is_ascii_digit() && ver[1].is_ascii_digit() && ver[2].is_ascii_digit() && ver[3] == 0x00
}

fn validate_pcx(data: &[u8], pos: usize) -> bool {
    // PCX 128-byte fixed header; need at least 68 bytes for the full validation.
    if need(data, pos, 68) {
        return true;
    }
    // @1: version — only {0, 2, 3, 4, 5} are defined (1 = obsolete, 6+ invalid)
    let version = data[pos + 1];
    if !matches!(version, 0 | 2 | 3 | 4 | 5) {
        return false;
    }
    // @2: encoding — 0 = uncompressed (rare), 1 = RLE (standard)
    let encoding = data[pos + 2];
    if encoding > 1 {
        return false;
    }
    // @3: bits per plane — must be a power of two in {1, 2, 4, 8}
    let bpp = data[pos + 3];
    if !matches!(bpp, 1 | 2 | 4 | 8) {
        return false;
    }
    // @4–5: xMin (LE), @8–9: xMax (LE) — xMax must be >= xMin
    let x_min = u16::from_le_bytes([data[pos + 4], data[pos + 5]]);
    let x_max = u16::from_le_bytes([data[pos + 8], data[pos + 9]]);
    if x_max < x_min {
        return false;
    }
    // @6–7: yMin (LE), @10–11: yMax (LE) — yMax must be >= yMin
    let y_min = u16::from_le_bytes([data[pos + 6], data[pos + 7]]);
    let y_max = u16::from_le_bytes([data[pos + 10], data[pos + 11]]);
    if y_max < y_min {
        return false;
    }
    // @64: reserved — must be 0 per ZSoft specification
    if data[pos + 64] != 0x00 {
        return false;
    }
    // @65: color planes — {1, 3, 4} are valid; 2 is theoretically valid but extremely rare
    let planes = data[pos + 65];
    if !matches!(planes, 1 | 3 | 4) {
        return false;
    }
    // @66–67: bytes per line (LE) — must be > 0
    let bpl = u16::from_le_bytes([data[pos + 66], data[pos + 67]]);
    bpl > 0
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
    let file_size =
        u32::from_le_bytes([data[pos + 2], data[pos + 3], data[pos + 4], data[pos + 5]]);
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

    // Reject brands handled by dedicated signatures or non-video ISOBMFF
    // formats to avoid false positives and duplicate output.
    let brand = &data[pos + 8..pos + 12];
    if brand == b"qt  "           // QuickTime MOV — dedicated sig
        || brand == b"M4V "       // iTunes M4V — dedicated sig
        || brand == b"M4A "       // iTunes M4A audio — dedicated sig
        || brand.starts_with(b"3gp")  // 3GPP — dedicated sig
        || brand.starts_with(b"3g2")  // 3GPP2 — dedicated sig
        || brand.starts_with(b"jp")   // JPEG 2000: jp2 , jpx , jpm , jpxb
        || brand == b"heic"       // Apple HEIC — dedicated sig
        || brand == b"heix"       // Apple HEIC variant — dedicated sig
        || brand == b"mif1"       // HEIF generic container
        || brand == b"avif"       // AV1 Image File Format
        || brand == b"crx "
    // Canon CR3 — dedicated sig
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
        let next_size =
            u32::from_be_bytes([data[next], data[next + 1], data[next + 2], data[next + 3]]);
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
    let fmt = data[pos + 6];
    if !matches!(fmt, 0x00 | 0x01) {
        return false;
    }
    // RAR4: archive header at offset 7.
    //   @9: HEAD_TYPE must be 0x73 (archive header marker).
    //   @10-11: HEAD_FLAGS (u16 LE):
    //     bit 0 (0x0001) = MHD_VOLUME (multi-volume archive)
    //     bit 8 (0x0100) = MHD_FIRSTVOLUME (first volume, RAR 3.0+)
    //   If it's a volume but NOT the first volume → continuation → useless for recovery.
    if fmt == 0x00 {
        if need(data, pos, 12) {
            return true;
        }
        if data[pos + 9] != 0x73 {
            return false;
        }
        let flags = u16::from_le_bytes([data[pos + 10], data[pos + 11]]);
        let is_volume = flags & 0x0001 != 0;
        let is_first = flags & 0x0100 != 0;
        if is_volume && !is_first {
            return false;
        }
    }
    true
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
                    return doc == b"matroska";
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
    // <?xml version — the XML spec mandates "version" as the first attribute
    // in the XML declaration.  Checking 13 bytes: "<?xml version" rejects
    // embedded `<?xml ` inside video/binary data that lacks a proper declaration.
    if need(data, pos, 13) {
        return true;
    }
    data[pos + 5] == b' '
        && data[pos + 6] == b'v'
        && data[pos + 7] == b'e'
        && data[pos + 8] == b'r'
        && data[pos + 9] == b's'
        && data[pos + 10] == b'i'
        && data[pos + 11] == b'o'
        && data[pos + 12] == b'n'
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
    // Rejects Sony SR2: those have private tag 0x7200 in IFD0.
    if need(data, pos, 10) {
        return true;
    }
    let entry_count = u16::from_le_bytes([data[pos + 8], data[pos + 9]]) as usize;
    if !(5..=50).contains(&entry_count) {
        return false;
    }
    let window_end = data.len().min(pos + 512);
    if !data[pos..window_end].windows(4).any(|w| w == b"SONY") {
        return false;
    }
    // Reject SR2: private tag 0x7200 present as IFD entry (LE tag bytes 0x00 0x72).
    // IFD0 entries start at byte 10; each entry is 12 bytes.
    let ifd_entries_end = (pos + 10 + entry_count * 12).min(data.len());
    let has_sr2_tag = (pos + 10..ifd_entries_end)
        .step_by(12)
        .any(|e| e + 2 <= data.len() && data[e] == 0x00 && data[e + 1] == 0x72);
    !has_sr2_tag
}

fn validate_cr2(data: &[u8], pos: usize) -> bool {
    // Canon CR2: TIFF LE magic + IFD offset (wildcard) + CR\x02\x00 at +8.
    // The CR marker bytes are already guaranteed by the scan magic; here we
    // just verify the IFD offset at +4 is plausible (8–4096 bytes).
    if need(data, pos, 8) {
        return true;
    }
    let ifd_offset =
        u32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]]) as usize;
    (8..=4096).contains(&ifd_offset)
}

fn validate_rw2(data: &[u8], pos: usize) -> bool {
    // Panasonic RW2: TIFF variant magic II\x55\x00 (already matched).
    // Verify the IFD0 offset at +4 and a plausible IFD entry count.
    if need(data, pos, 10) {
        return true;
    }
    let ifd_offset =
        u32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]]) as usize;
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

fn validate_tiff_le(data: &[u8], pos: usize) -> bool {
    // Generic little-endian TIFF (II\x2A\x00).
    // 1. IFD0 offset (u32 LE @ +4) must be in [8, 65536].
    // 2. Reject Sony ARW ("SONY" in first 512 bytes — use dedicated .arw signature).
    // 3. Reject Canon CR2 (CR\x02\x00 at offset +8 — use dedicated .cr2 signature).
    // 4. Reject Nikon NEF ("NIKON" in first 512 bytes — use dedicated .nef signature).
    // 5. IFD entry count at IFD0 offset must be in [1, 500] (if within chunk).
    // 6. IFD0 must contain ImageWidth (tag 256) — rejects EXIF blocks embedded in
    //    JPEGs, which share the TIFF magic but only carry metadata in IFD0.
    if need(data, pos, 8) {
        return true;
    }
    let ifd_off =
        u32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]]) as usize;
    if !(8..=65536).contains(&ifd_off) {
        return false;
    }
    // Reject CR2 (Canon marker)
    if data.len() >= pos + 12 && &data[pos + 8..pos + 12] == b"CR\x02\x00" {
        return false;
    }
    let window_end = data.len().min(pos + 512);
    let window = &data[pos..window_end];
    // Reject ARW (Sony)
    if window.windows(4).any(|w| w == b"SONY") {
        return false;
    }
    // Reject NEF (Nikon) — has its own signature
    if window.windows(5).any(|w| w == b"NIKON") {
        return false;
    }
    // Reject DCR (Kodak) — has its own signature
    if window.windows(5).any(|w| w == b"Kodak" || w == b"KODAK") {
        return false;
    }
    // Plausible IFD entry count; require ImageWidth (tag 256) in IFD0.
    if data.len() >= pos + ifd_off + 2 {
        let entry_count =
            u16::from_le_bytes([data[pos + ifd_off], data[pos + ifd_off + 1]]) as usize;
        if !(1..=500).contains(&entry_count) {
            return false;
        }
        // Walk IFD0 entries: each is 12 bytes, tag is u16 LE at entry base.
        // If all entries fit in the available data, require ImageWidth (256).
        let entries_end = pos + ifd_off + 2 + entry_count * 12;
        if data.len() >= entries_end {
            let has_image_width = (0..entry_count).any(|i| {
                let e = pos + ifd_off + 2 + i * 12;
                u16::from_le_bytes([data[e], data[e + 1]]) == 256
            });
            if !has_image_width {
                return false;
            }
        }
    }
    true
}

fn validate_tiff_be(data: &[u8], pos: usize) -> bool {
    // Generic big-endian TIFF (MM\x00\x2A).
    // IFD0 offset (u32 BE @ +4) must be in [8, 65536].
    // IFD entry count at IFD0 must be in [1, 500] (if within chunk).
    // IFD0 must contain ImageWidth (tag 256) — same EXIF-in-JPEG rejection as LE.
    if need(data, pos, 8) {
        return true;
    }
    let ifd_off =
        u32::from_be_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]]) as usize;
    if !(8..=65536).contains(&ifd_off) {
        return false;
    }
    if data.len() >= pos + ifd_off + 2 {
        let entry_count =
            u16::from_be_bytes([data[pos + ifd_off], data[pos + ifd_off + 1]]) as usize;
        if !(1..=500).contains(&entry_count) {
            return false;
        }
        let entries_end = pos + ifd_off + 2 + entry_count * 12;
        if data.len() >= entries_end {
            let has_image_width = (0..entry_count).any(|i| {
                let e = pos + ifd_off + 2 + i * 12;
                u16::from_be_bytes([data[e], data[e + 1]]) == 256
            });
            if !has_image_width {
                return false;
            }
        }
    }
    true
}

fn validate_nef(data: &[u8], pos: usize) -> bool {
    // Nikon NEF: TIFF LE file with "NIKON" manufacturer string in first 512 bytes.
    // IFD0 offset (u32 LE @ +4) must also be in [8, 4096].
    if need(data, pos, 8) {
        return true;
    }
    let ifd_off =
        u32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]]) as usize;
    if !(8..=4096).contains(&ifd_off) {
        return false;
    }
    let window_end = data.len().min(pos + 512);
    data[pos..window_end].windows(5).any(|w| w == b"NIKON")
}

fn validate_heic(data: &[u8], pos: usize) -> bool {
    // Apple HEIC / HEIF: ftyp box with heic or heix major brand.
    // Magic already anchors on "ftyp" + first 3 brand bytes ("hei").
    // The 12-byte signature covers the full brand; here we just verify
    // the ftyp box size (u32 BE @ pos) is in [12, 512].
    // Require 8 bytes so a short chunk gets benefit of doubt.
    if need(data, pos, 8) {
        return true;
    }
    let box_size = u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
    (12..=512).contains(&box_size)
}

fn validate_mov(data: &[u8], pos: usize) -> bool {
    // QuickTime MOV: magic anchors on ftyp + "qt  " brand (12 bytes).
    // Validate that the ftyp box size (u32 BE @pos) is in [12, 512].
    if need(data, pos, 12) {
        return true;
    }
    let box_size = u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
    (12..=512).contains(&box_size)
}

fn validate_m4v(data: &[u8], pos: usize) -> bool {
    // iTunes M4V: magic anchors on ftyp + "M4V " brand (12 bytes).
    // Validate that the ftyp box size (u32 BE @pos) is in [12, 512].
    if need(data, pos, 12) {
        return true;
    }
    let box_size = u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
    (12..=512).contains(&box_size)
}

fn validate_3gp(data: &[u8], pos: usize) -> bool {
    // 3GPP / 3GPP2: magic anchors on ftyp + "3g" (10 bytes).
    // Full brand (bytes 8-11) must start with "3gp" or "3g2".
    // Box size (u32 BE @pos) must be in [12, 512].
    if need(data, pos, 12) {
        return true;
    }
    let box_size = u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
    if !(12..=512).contains(&box_size) {
        return false;
    }
    let brand = &data[pos + 8..pos + 12];
    brand.starts_with(b"3gp") || brand.starts_with(b"3g2")
}

fn validate_webm(data: &[u8], pos: usize) -> bool {
    // WebM: EBML file with DocType element == "webm" (not "matroska").
    // Identical structure to MKV; only the expected DocType string differs.
    if need(data, pos, 5) {
        return true;
    }
    if data[pos + 4] == 0x00 {
        return false;
    }
    const WINDOW: usize = 80;
    let search_end = data.len().min(pos + WINDOW);
    let have_full_window = search_end == pos + WINDOW;
    let window = &data[pos + 5..search_end];
    let mut i = 0;
    while i + 1 < window.len() {
        if window[i] == 0x42 && window[i + 1] == 0x82 {
            if i + 2 >= window.len() {
                return true;
            }
            let vint = window[i + 2];
            if vint & 0x80 != 0 {
                let doc_len = (vint & 0x7F) as usize;
                let doc_start = i + 3;
                if doc_start + doc_len <= window.len() {
                    let doc = &window[doc_start..doc_start + doc_len];
                    return doc == b"webm";
                }
            }
            return true;
        }
        i += 1;
    }
    !have_full_window
}

fn validate_wmv(data: &[u8], pos: usize) -> bool {
    // ASF Header Object: 16-byte GUID at pos, then u64 LE object size @16.
    // Object size must be ≥ 30 (minimum valid ASF header body).
    if need(data, pos, 24) {
        return true;
    }
    let size_bytes: [u8; 8] = data[pos + 16..pos + 24].try_into().unwrap();
    u64::from_le_bytes(size_bytes) >= 30
}

fn validate_flv(data: &[u8], pos: usize) -> bool {
    // FLV file header (9 bytes): "FLV" + version(0x01) + type_flags + data_offset(u32 BE).
    // Reserved bits in type_flags (offset 4) must be zero: bits 7-3 and bit 1.
    // DataOffset (u32 BE @5) must be 9 for version 1.
    if need(data, pos, 9) {
        return true;
    }
    if data[pos + 4] & 0b1111_1010 != 0 {
        return false;
    }
    let offset = u32::from_be_bytes([data[pos + 5], data[pos + 6], data[pos + 7], data[pos + 8]]);
    offset == 9
}

fn validate_mpeg(data: &[u8], pos: usize) -> bool {
    // MPEG Program Stream pack header: 00 00 01 BA + 9 bytes of SCR/mux_rate fields.
    //
    // The ISO 13818-1 standard mandates several fixed "marker_bit = 1" positions
    // within the pack header.  Checking all of them simultaneously reduces the
    // false-positive rate to ~1-in-256 on random data and near-zero on H.264/H.265
    // Annex-B NAL streams (which also contain `00 00 01 BA` bytes in video payload).
    //
    // MPEG-2 pack header layout (offsets relative to `pos`):
    //   +4 : 01 SCR_b[32:30] M SCR_b[29:28]   — bit 2 must be 1 (marker)
    //   +5 : SCR_b[27:20]
    //   +6 : SCR_b[19:15] M SCR_b[14:13]      — bit 2 must be 1 (marker)
    //   +7 : SCR_b[12:5]
    //   +8 : SCR_b[4:0]  M  SCR_ext[8:7]      — bit 2 must be 1 (marker)
    //   +9 : SCR_ext[6:0] M                   — bit 0 must be 1 (marker)
    //  +10 : mux_rate[21:14]
    //  +11 : mux_rate[13:6]
    //  +12 : mux_rate[5:0] M M                — bits 1:0 must be 11 (two markers)
    //  +13 : 1 1 1 1 1 stuffing_length[2:0]   — top 5 bits must be 11111
    if need(data, pos, 5) {
        return true;
    }
    let b4 = data[pos + 4];
    if (b4 & 0xC0) == 0x40 {
        // MPEG-2: verify all mandatory marker bits in the 14-byte pack header.
        if need(data, pos, 14) {
            return true; // short buffer — benefit of doubt
        }
        (b4 & 0x04 != 0)                     // byte  4, bit 2: marker
            && (data[pos + 6] & 0x04 != 0)   // byte  6, bit 2: marker
            && (data[pos + 8] & 0x04 != 0)   // byte  8, bit 2: marker
            && (data[pos + 9] & 0x01 != 0)   // byte  9, bit 0: marker
            && (data[pos + 12] & 0x03 == 0x03) // byte 12, bits 1:0: two markers
            && (data[pos + 13] & 0xF8 == 0xF8) // byte 13, top 5: stuffing fixed bits
    } else if (b4 & 0xF0) == 0x20 {
        // MPEG-1: bit 0 of byte 4 is a marker bit in the SCR field.
        b4 & 0x01 != 0
    } else {
        false
    }
}

fn validate_webp(data: &[u8], pos: usize) -> bool {
    // RIFF + 4-byte size + "WEBP" subtype (12 bytes total).
    // Magic already anchors on RIFF[0..4] + WEBP[8..12]; re-check WEBP and
    // verify the RIFF chunk size (u32 LE @4) is ≥ 4 (minimum: 4 bytes for "WEBP").
    if need(data, pos, 12) {
        return true;
    }
    if &data[pos + 8..pos + 12] != b"WEBP" {
        return false;
    }
    let riff_size =
        u32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]]);
    riff_size >= 4
}

fn validate_m4a(data: &[u8], pos: usize) -> bool {
    // ISOBMFF ftyp box with "M4A " major brand.
    // Magic already anchors on ftyp + "M4A "; validate ftyp box size in [12, 512].
    if need(data, pos, 12) {
        return true;
    }
    let box_size = u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
    (12..=512).contains(&box_size)
}

fn validate_gz(data: &[u8], pos: usize) -> bool {
    // GZip: ID1=0x1F ID2=0x8B CM=8 FLG XFL OS.
    //
    // RFC 1952 checks:
    //   @2 CM  — compression method, must be 8 (deflate).
    //   @3 FLG — bit 5 (0x20) is reserved, must be 0.
    //   @8 XFL — extra flags: 0 (none), 2 (max compression), 4 (fastest).
    //            Random byte lands in {0,2,4} ~1.2% of the time.
    //   @9 OS  — originating OS: 0–13 are defined, 255 = unknown.
    //            Random byte is valid ~5.9% of the time.
    //
    // Combined XFL + OS rejects ~99.9% of false positives on bytes 8–9 alone.
    if need(data, pos, 10) {
        return true;
    }
    if data[pos + 2] != 8 {
        return false;
    }
    if data[pos + 3] & 0x20 != 0 {
        return false;
    }
    let xfl = data[pos + 8];
    if xfl != 0 && xfl != 2 && xfl != 4 {
        return false;
    }
    let os = data[pos + 9];
    os <= 13 || os == 255
}

fn validate_eml(data: &[u8], pos: usize) -> bool {
    // mbox "From " line format: "From user@domain.tld Day Mon DD HH:MM:SS YYYY"
    // We scan the first 80 bytes after "From " for:
    //   1. All bytes must be printable ASCII (0x20–0x7E) or common whitespace (\t, \r, \n)
    //   2. An '@' sign must appear (sender email address)
    // This rejects XMP/RDF "From rdf:parseType..." and binary data.
    let check_len = 80;
    let header_len = 5; // "From "
    if need(data, pos, header_len + check_len) {
        // Short buffer — require at least one printable byte
        if need(data, pos, 6) {
            return true;
        }
        let next = data[pos + 5];
        return (0x20..=0x7E).contains(&next);
    }
    let window = &data[pos + header_len..pos + header_len + check_len];
    let mut has_at = false;
    for &b in window {
        if b == b'@' {
            has_at = true;
        }
        if b == b'\n' {
            // End of "From " line — stop checking
            break;
        }
        if !(b == b'\t' || b == b'\r' || (0x20..=0x7E).contains(&b)) {
            return false; // Non-printable byte → binary data
        }
    }
    has_at
}

fn validate_elf(data: &[u8], pos: usize) -> bool {
    // ELF e_ident: 7F ELF EI_CLASS EI_DATA EI_VERSION ...
    // EI_CLASS @4: 1 = 32-bit, 2 = 64-bit.
    // EI_DATA  @5: 1 = little-endian, 2 = big-endian.
    // EI_VERSION @6: must be 1.
    if need(data, pos, 16) {
        return true;
    }
    let ei_class = data[pos + 4];
    let ei_data = data[pos + 5];
    let ei_version = data[pos + 6];
    matches!(ei_class, 1 | 2) && matches!(ei_data, 1 | 2) && ei_version == 1
}

fn validate_regf(data: &[u8], pos: usize) -> bool {
    // Windows Registry REGF hive.
    // Major version (u32 LE @20) must be 1.
    // Minor version (u32 LE @24) must be in [2, 6].
    if need(data, pos, 28) {
        return true;
    }
    let major = u32::from_le_bytes([
        data[pos + 20],
        data[pos + 21],
        data[pos + 22],
        data[pos + 23],
    ]);
    let minor = u32::from_le_bytes([
        data[pos + 24],
        data[pos + 25],
        data[pos + 26],
        data[pos + 27],
    ]);
    major == 1 && (2..=6).contains(&minor)
}

fn validate_psd(data: &[u8], pos: usize) -> bool {
    // Adobe Photoshop Document: "8BPS" magic + version + reserved + channels.
    // Version (u16 BE @4): 1 = PSD, 2 = PSB.
    // Number of channels (u16 BE @6): must be in [1, 56].
    if need(data, pos, 26) {
        return true;
    }
    let version = u16::from_be_bytes([data[pos + 4], data[pos + 5]]);
    if !matches!(version, 1 | 2) {
        return false;
    }
    let channels = u16::from_be_bytes([data[pos + 6], data[pos + 7]]);
    (1..=56).contains(&channels)
}

fn validate_vhd(data: &[u8], pos: usize) -> bool {
    // VHD footer: "conectix" creator string (already matched by magic).
    // Disk type (u32 BE @60): 2 = fixed, 3 = dynamic, 4 = differencing.
    if need(data, pos, 68) {
        return true;
    }
    let disk_type = u32::from_be_bytes([
        data[pos + 60],
        data[pos + 61],
        data[pos + 62],
        data[pos + 63],
    ]);
    (2..=4).contains(&disk_type)
}

fn validate_vhdx(data: &[u8], pos: usize) -> bool {
    // VHDX: "vhdxfile" (8 bytes) — globally unique, no additional check needed.
    // Return benefit of doubt when the buffer is short; accept when magic present.
    if need(data, pos, 8) {
        return true;
    }
    true
}

fn validate_qcow2(data: &[u8], pos: usize) -> bool {
    // QCOW2: QFI\xFB magic + version (u32 BE @4) in {2,3} +
    // cluster_bits (u32 BE @20) in [9, 21].
    if need(data, pos, 24) {
        return true;
    }
    let version = u32::from_be_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]]);
    if !matches!(version, 2 | 3) {
        return false;
    }
    let cluster_bits = u32::from_be_bytes([
        data[pos + 20],
        data[pos + 21],
        data[pos + 22],
        data[pos + 23],
    ]);
    (9..=21).contains(&cluster_bits)
}

// ── Phase 52 validators ───────────────────────────────────────────────────────

/// MIDI — Standard MIDI File header chunk.
/// Chunk length (u32 BE @4) must be 6; format (u16 BE @8) must be 0, 1, or 2.
fn validate_midi(data: &[u8], pos: usize) -> bool {
    let end = pos + 10;
    if data.len() < end {
        return true;
    }
    let chunk_len =
        u32::from_be_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]]);
    if chunk_len != 6 {
        return false;
    }
    let format = u16::from_be_bytes([data[pos + 8], data[pos + 9]]);
    format <= 2
}

/// AIFF / AIFC — IFF FORM chunk with AIFF or AIFC sub-type.
/// Byte @11 must be 'F' (AIFF) or 'C' (AIFC); FORM data size must be > 4.
fn validate_aiff(data: &[u8], pos: usize) -> bool {
    let end = pos + 12;
    if data.len() < end {
        return true;
    }
    let subtype = data[pos + 11];
    if subtype != b'F' && subtype != b'C' {
        return false;
    }
    let form_size =
        u32::from_be_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]]);
    form_size > 4
}

/// XZ compressed stream.
/// Byte @6 must be 0x00 (reserved); byte @7 check type must be in
/// {0x00=none, 0x01=CRC-32, 0x04=CRC-64, 0x0A=SHA-256}.
fn validate_xz(data: &[u8], pos: usize) -> bool {
    let end = pos + 8;
    if data.len() < end {
        return true;
    }
    if data[pos + 6] != 0x00 {
        return false;
    }
    matches!(data[pos + 7], 0x00 | 0x01 | 0x04 | 0x0A)
}

/// BZip2 compressed file.
/// Byte @3 must be a level digit '1'-'9' (0x31-0x39).
/// Bytes @4-9 must match the BWT block magic constant (Pi in packed binary):
/// 0x31 0x41 0x59 0x26 0x53 0x59.
fn validate_bzip2(data: &[u8], pos: usize) -> bool {
    let end = pos + 10;
    if data.len() < end {
        return true;
    }
    let level = data[pos + 3];
    if !(b'1'..=b'9').contains(&level) {
        return false;
    }
    const BLOCK_MAGIC: [u8; 6] = [0x31, 0x41, 0x59, 0x26, 0x53, 0x59];
    data[pos + 4..pos + 10] == BLOCK_MAGIC
}

/// RealMedia File Format (.RMF).
/// Object version (u16 BE @4) must be 0 or 1.
/// Header size (u32 BE @6) must be >= 18.
fn validate_realmedia(data: &[u8], pos: usize) -> bool {
    let end = pos + 10;
    if data.len() < end {
        return true;
    }
    let obj_version = u16::from_be_bytes([data[pos + 4], data[pos + 5]]);
    if obj_version > 1 {
        return false;
    }
    let header_size =
        u32::from_be_bytes([data[pos + 6], data[pos + 7], data[pos + 8], data[pos + 9]]);
    header_size >= 18
}

/// Windows ICO file.
/// Image count (u16 LE @4) must be in [1, 200].
fn validate_ico(data: &[u8], pos: usize) -> bool {
    // ICO header: 00 00 01 00 <count:u16 LE> then 16-byte image directory entries.
    // We validate the first directory entry to reject false positives from the
    // extremely weak 4-byte magic (3 of which are zeros).
    let end = pos + 22; // header(6) + one 16-byte directory entry
    if data.len() < end {
        // Not enough data for even one entry — fall back to count-only check
        let end6 = pos + 6;
        if data.len() < end6 {
            return true;
        }
        let count = u16::from_le_bytes([data[pos + 4], data[pos + 5]]);
        return (1..=200).contains(&count);
    }
    let count = u16::from_le_bytes([data[pos + 4], data[pos + 5]]);
    if !(1..=200).contains(&count) {
        return false;
    }
    // First directory entry starts at pos+6:
    //   +0 width, +1 height, +2 color_count, +3 reserved (must be 0)
    //   +4..+6 planes (u16 LE, must be 0 or 1)
    //   +6..+8 bpp (u16 LE, must be in standard set)
    //   +8..+12 data_size (u32 LE, must be > 0)
    //   +12..+16 data_offset (u32 LE, must be >= header+entries size)
    let entry = pos + 6;
    let reserved = data[entry + 3];
    if reserved != 0 {
        return false;
    }
    let planes = u16::from_le_bytes([data[entry + 4], data[entry + 5]]);
    if planes > 1 {
        return false;
    }
    let bpp = u16::from_le_bytes([data[entry + 6], data[entry + 7]]);
    if !matches!(bpp, 0 | 1 | 4 | 8 | 16 | 24 | 32) {
        return false;
    }
    let data_size = u32::from_le_bytes([
        data[entry + 8],
        data[entry + 9],
        data[entry + 10],
        data[entry + 11],
    ]);
    if data_size == 0 || data_size > 1_048_576 {
        return false;
    }
    let data_offset = u32::from_le_bytes([
        data[entry + 12],
        data[entry + 13],
        data[entry + 14],
        data[entry + 15],
    ]);
    let min_offset = 6 + 16 * count as u32;
    // data_offset must be within the ICO file (max_size = 1 MiB)
    // and the entry's data must fit within that bound.
    if data_offset < min_offset || data_offset > 1_048_576 {
        return false;
    }
    // planes and bpp can't BOTH be zero in a real icon
    if planes == 0 && bpp == 0 {
        return false;
    }
    true
}

/// Olympus ORF (TIFF LE with RO magic 0x524F).
/// IFD offset (u32 LE @4) must be in [8, 4096].
fn validate_orf(data: &[u8], pos: usize) -> bool {
    let end = pos + 8;
    if data.len() < end {
        return true;
    }
    let ifd_offset =
        u32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]]);
    (8..=4096).contains(&ifd_offset)
}

/// Pentax PEF (TIFF LE file with "PENTAX " string in first 512 bytes).
/// IFD offset (u32 LE @4) must also be in [8, 4096].
fn validate_pef(data: &[u8], pos: usize) -> bool {
    let end = pos + 8;
    if data.len() < end {
        return true;
    }
    let ifd_offset =
        u32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]]);
    if !(8..=4096).contains(&ifd_offset) {
        return false;
    }
    let scan_end = (pos + 512).min(data.len());
    data[pos..scan_end].windows(7).any(|w| w == b"PENTAX ")
}

/// Mach-O 64-bit little-endian executable.
/// filetype (u32 LE @12) must be in [1, 12]; ncmds (u32 LE @16) in [1, 512].
fn validate_macho(data: &[u8], pos: usize) -> bool {
    let end = pos + 20;
    if data.len() < end {
        return true;
    }
    let filetype = u32::from_le_bytes([
        data[pos + 12],
        data[pos + 13],
        data[pos + 14],
        data[pos + 15],
    ]);
    if !(1..=12).contains(&filetype) {
        return false;
    }
    let ncmds = u32::from_le_bytes([
        data[pos + 16],
        data[pos + 17],
        data[pos + 18],
        data[pos + 19],
    ]);
    (1..=512).contains(&ncmds)
}

fn validate_cr3(data: &[u8], pos: usize) -> bool {
    // Canon CR3: ISOBMFF ftyp box with "crx " brand.
    // Magic anchors bytes 4-11 ("ftypcrx "); validate box size (u32 BE @pos) in [12, 512].
    if need(data, pos, 8) {
        return true;
    }
    let box_size = u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
    (12..=512).contains(&box_size)
}

fn validate_sr2(data: &[u8], pos: usize) -> bool {
    // Sony SR2: TIFF LE with IFD at offset 8 (anchored in magic).
    // Requires "SONY" in first 512 bytes AND private tag 0x7200 (SR2Private) as an
    // IFD0 entry — uniquely present in SR2, absent in ARW.
    if need(data, pos, 10) {
        return true;
    }
    let entry_count = u16::from_le_bytes([data[pos + 8], data[pos + 9]]) as usize;
    if !(3..=50).contains(&entry_count) {
        return false;
    }
    let window_end = data.len().min(pos + 512);
    if !data[pos..window_end].windows(4).any(|w| w == b"SONY") {
        return false;
    }
    // Check for tag 0x7200 (SR2Private) in IFD0 entries.
    // Each entry is 12 bytes; the tag is the first 2 bytes in LE (0x00 0x72).
    let ifd_entries_end = (pos + 10 + entry_count * 12).min(data.len());
    (pos + 10..ifd_entries_end)
        .step_by(12)
        .any(|e| e + 2 <= data.len() && data[e] == 0x00 && data[e + 1] == 0x72)
}

fn validate_epub(data: &[u8], pos: usize) -> bool {
    // EPUB: ZIP container.  First local file entry must be named "mimetype" (8 bytes,
    // uncompressed store, no extra field typical) and its content contains "epub+zip".
    // ZIP LFH layout: +26 fname_len (u16 LE), +28 extra_len (u16 LE), +30 fname.
    if need(data, pos, 38) {
        return true;
    }
    let fname_len = u16::from_le_bytes([data[pos + 26], data[pos + 27]]) as usize;
    if fname_len != 8 {
        return false;
    }
    if &data[pos + 30..pos + 38] != b"mimetype" {
        return false;
    }
    let extra_len = u16::from_le_bytes([data[pos + 28], data[pos + 29]]) as usize;
    let content_start = pos + 30 + fname_len + extra_len;
    if content_start >= data.len() {
        return true; // content straddles chunk boundary — benefit of doubt
    }
    let content_end = data.len().min(content_start + 32);
    data[content_start..content_end]
        .windows(8)
        .any(|w| w == b"epub+zip")
}

fn validate_odt(data: &[u8], pos: usize) -> bool {
    // OpenDocument (ODT/ODS/ODP/…): ZIP container.  First local file entry must be
    // named "mimetype" and its content contains "opendocument" (covers all ODF subtypes).
    if need(data, pos, 38) {
        return true;
    }
    let fname_len = u16::from_le_bytes([data[pos + 26], data[pos + 27]]) as usize;
    if fname_len != 8 {
        return false;
    }
    if &data[pos + 30..pos + 38] != b"mimetype" {
        return false;
    }
    // Reject EPUB — it also has fname="mimetype" but different content.
    let extra_len = u16::from_le_bytes([data[pos + 28], data[pos + 29]]) as usize;
    let content_start = pos + 30 + fname_len + extra_len;
    if content_start >= data.len() {
        return true;
    }
    let content_end = data.len().min(content_start + 50);
    data[content_start..content_end]
        .windows(12)
        .any(|w| w == b"opendocument")
}

fn validate_msg(data: &[u8], pos: usize) -> bool {
    // Outlook MSG: OLE2 compound document.
    // ByteOrder field (u16 LE @28) must equal 0xFFFE (standard OLE2).
    // Scan first 4 KiB for the MAPI stream name "__substg1.0_" — present in all
    // MSG files, absent in DOC/XLS/PPT compound documents.
    if need(data, pos, 30) {
        return true;
    }
    let byte_order = u16::from_le_bytes([data[pos + 28], data[pos + 29]]);
    if byte_order != 0xFFFE {
        return false;
    }
    let scan_end = data.len().min(pos + 4096);
    data[pos..scan_end]
        .windows(12)
        .any(|w| w == b"__substg1.0_")
}

fn validate_wavpack(data: &[u8], pos: usize) -> bool {
    // WavPack: "wvpk" block header (magic already matched).
    // ck_size (u32 LE @4): block data size minus 8 header bytes; must be > 0.
    // version (u16 LE @8): WavPack file format version; valid range [0x0402, 0x0410].
    if need(data, pos, 10) {
        return true;
    }
    let ck_size = u32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]]);
    if ck_size == 0 {
        return false;
    }
    let version = u16::from_le_bytes([data[pos + 8], data[pos + 9]]);
    (0x0402..=0x0410).contains(&version)
}

fn validate_cdr(data: &[u8], pos: usize) -> bool {
    // CorelDRAW CDR: RIFF container.  Magic anchors "CDR" at bytes 8–10.
    // Byte @11 is the version suffix: '4'–'9' (CDR4–CDR9), 'A'–'Z' (CDRv10+),
    // or space (some CDR12 variants).
    if need(data, pos, 12) {
        return true;
    }
    matches!(data[pos + 11], b'4'..=b'9' | b'A'..=b'Z' | b' ')
}

fn validate_swf(data: &[u8], pos: usize) -> bool {
    // Shockwave Flash: FWS (uncompressed), CWS (zlib), or ZWS (LZMA).
    //
    // Version byte @3: Flash 3 was the first widely deployed version; the
    // final release was Flash Player 44 (2024).  Accepting [3, 45] tightens
    // the 20% pass rate of [1, 50] on random data to ~17%.
    //
    // File length (u32 LE @4): the *uncompressed* SWF size.  Must be >= 21
    // (8 header + 5 RECT + 4 frame-rate/count + end tag) and <= 100 MiB
    // (the extraction cap).  A random u32 LE exceeds 100 MiB ~97.7% of the
    // time, so this single check eliminates almost all false positives.
    if need(data, pos, 9) {
        return true;
    }
    let version = data[pos + 3];
    if !(3..=45).contains(&version) {
        return false;
    }
    let file_len = u32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]]);
    if !(21..=104_857_600).contains(&file_len) {
        return false;
    }
    // FWS (uncompressed): byte 8 starts the RECT structure.  Top 5 bits are
    // Nbits (number of bits per RECT field).  Real values are [1, 25]; higher
    // would imply stage dimensions > 1M pixels in twips, which is unrealistic.
    if data[pos] == b'F' {
        let nbits = data[pos + 8] >> 3;
        if nbits == 0 || nbits > 25 {
            return false;
        }
    }
    // CWS (zlib): byte 8 is the zlib CMF byte.  CM (lower 4 bits) must be 8
    // (deflate) and CINFO (upper 4 bits) must be ≤ 7.
    if data[pos] == b'C' {
        let cmf = data[pos + 8];
        if cmf & 0x0F != 8 || cmf >> 4 > 7 {
            return false;
        }
    }
    true
}

fn validate_dcr(data: &[u8], pos: usize) -> bool {
    // Kodak DCR: TIFF LE file identified by "Kodak" or "KODAK" Make string in the
    // first 512 bytes.  IFD0 offset (u32 LE @4) must be in [8, 65536].
    // Rejects CR2 (Canon marker at +8), NEF (NIKON string), ARW/SR2 (SONY string),
    // and PEF (PENTAX string) to avoid duplicate hits.
    if need(data, pos, 8) {
        return true;
    }
    let ifd_off =
        u32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]]) as usize;
    if !(8..=65536).contains(&ifd_off) {
        return false;
    }
    if data.len() >= pos + 12 && &data[pos + 8..pos + 12] == b"CR\x02\x00" {
        return false;
    }
    let window_end = data.len().min(pos + 512);
    let window = &data[pos..window_end];
    if window.windows(5).any(|w| w == b"NIKON") {
        return false;
    }
    if window.windows(4).any(|w| w == b"SONY") {
        return false;
    }
    if window.windows(7).any(|w| w == b"PENTAX ") {
        return false;
    }
    window.windows(5).any(|w| w == b"Kodak" || w == b"KODAK")
}

fn validate_iso(data: &[u8], pos: usize) -> bool {
    // ISO 9660: `CD001` magic at offset 32769 within the file (pos points here).
    // The PVD starts one byte before pos (Descriptor Type at pos-1).
    //
    // Check 1: Version byte at pos+5 must be 0x01.
    if need(data, pos, 6) {
        return true;
    }
    if data[pos + 5] != 0x01 {
        return false;
    }
    // Check 2: Descriptor Type byte at pos-1 must be 0x01 (Primary Volume
    // Descriptor).  We only carve PVD hits, not Supplementary VDs.
    if pos >= 1 && data[pos - 1] != 0x01 {
        return false;
    }
    // Check 3: Volume Space Size is stored as a Both-Byte-Order (BBO) field —
    // 4-byte LE at PVD+80 followed by 4-byte BE at PVD+84 (both encoding the
    // same value).  For random/garbage data the probability of these matching is
    // ~1 in 4 billion, making this an extremely strong false-positive filter.
    //
    // PVD+80 = (pos-1)+80 = pos+79 in the buffer.
    if !need(data, pos, 87) {
        let vss_le = u32::from_le_bytes([
            data[pos + 79],
            data[pos + 80],
            data[pos + 81],
            data[pos + 82],
        ]);
        let vss_be = u32::from_be_bytes([
            data[pos + 83],
            data[pos + 84],
            data[pos + 85],
            data[pos + 86],
        ]);
        if vss_le != vss_be || vss_le == 0 {
            return false;
        }
    }
    true
}

fn validate_dicom(data: &[u8], pos: usize) -> bool {
    // DICOM: `DICM` magic at offset 128 within the file (pos points here).
    // After the 4-byte "DICM" marker, a DICOM file contains data elements.
    // The first element should be from group 0x0002 (File Meta Information).
    // Each element starts with: group (u16 LE) + element (u16 LE) + VR (2 ASCII).
    // Validate: group == 0x0002 and VR is two uppercase ASCII letters.
    if need(data, pos, 10) {
        return true;
    }
    let group = u16::from_le_bytes([data[pos + 4], data[pos + 5]]);
    if group != 0x0002 {
        return false;
    }
    // VR (Value Representation) is 2 uppercase ASCII characters.
    let vr0 = data[pos + 8];
    let vr1 = data[pos + 9];
    vr0.is_ascii_uppercase() && vr1.is_ascii_uppercase()
}

fn validate_tar(data: &[u8], pos: usize) -> bool {
    // POSIX ustar TAR: `ustar\0` magic at offset 257 within a 512-byte header
    // block (pos points to `ustar\0`).  The 2-byte version field at pos+6 must
    // be `"00"` (POSIX) or `"  "` (GNU tar ustar variant).
    if need(data, pos, 8) {
        return true;
    }
    let ver = &data[pos + 6..pos + 8];
    ver == b"00" || ver == b"  "
}

fn validate_ape(data: &[u8], pos: usize) -> bool {
    // Monkey's Audio APE: "MAC " magic (4 bytes), then sub-version (u16 LE @4),
    // then version (u16 LE @6) which must be in [3930, 4100].
    if need(data, pos, 8) {
        return true;
    }
    let version = u16::from_le_bytes([data[pos + 6], data[pos + 7]]);
    (3930..=4100).contains(&version)
}

fn validate_au(data: &[u8], pos: usize) -> bool {
    // Sun AU: ".snd" magic (4 bytes), then data_offset (u32 BE @4) >= 24,
    // and encoding (u32 BE @12) in known valid set.
    if need(data, pos, 16) {
        return true;
    }
    let data_offset =
        u32::from_be_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]]);
    if data_offset < 24 {
        return false;
    }
    let encoding = u32::from_be_bytes([
        data[pos + 12],
        data[pos + 13],
        data[pos + 14],
        data[pos + 15],
    ]);
    // Known encodings: 1=MULAW, 2-7=PCM/float variants, 23-27=G.7xx
    matches!(encoding, 1..=7 | 23..=27)
}

fn validate_ttf(data: &[u8], pos: usize) -> bool {
    // TrueType Font: sfVersion 0x00010000 (4 bytes), then numTables (u16 BE @4).
    // Also validate searchRange (u16 BE @6) is consistent with numTables:
    //   searchRange = (highest power of 2 <= numTables) * 16.
    if need(data, pos, 12) {
        return true;
    }
    let num_tables = u16::from_be_bytes([data[pos + 4], data[pos + 5]]);
    if !(4..=50).contains(&num_tables) {
        return false;
    }
    let search_range = u16::from_be_bytes([data[pos + 6], data[pos + 7]]);
    let entry_selector = u16::from_be_bytes([data[pos + 8], data[pos + 9]]);
    // searchRange must be (1 << entry_selector) * 16.
    let expected_sr = (1u16 << entry_selector).saturating_mul(16);
    if search_range != expected_sr {
        return false;
    }
    // entry_selector = floor(log2(numTables)): (1 << entry_selector) <= numTables.
    (1u16 << entry_selector) <= num_tables
        && (entry_selector == 15 || (1u16 << (entry_selector + 1)) > num_tables)
}

fn validate_woff(data: &[u8], pos: usize) -> bool {
    // WOFF web font: "wOFF" magic (4 bytes).
    // flavor @4 (u32 BE) in {0x00010000 TTF, 0x4F54544F OTF, 0x74727565 true}.
    // length @8 (u32 BE) >= 44 (minimum WOFF header size).
    // numTables @12 (u16 BE) in [1, 50].
    if need(data, pos, 14) {
        return true;
    }
    let flavor = u32::from_be_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]]);
    if !matches!(flavor, 0x0001_0000 | 0x4F54_544F | 0x7472_7565) {
        return false;
    }
    let length = u32::from_be_bytes([data[pos + 8], data[pos + 9], data[pos + 10], data[pos + 11]]);
    if length < 44 {
        return false;
    }
    let num_tables = u16::from_be_bytes([data[pos + 12], data[pos + 13]]);
    (1..=50).contains(&num_tables)
}

fn validate_chm(data: &[u8], pos: usize) -> bool {
    // CHM: 12-byte magic is fully deterministic (ITSF + version 3 + length 96).
    // The TOML magic encodes all 12 bytes; just confirm enough data is present.
    !need(data, pos, 12)
}

fn validate_blend(data: &[u8], pos: usize) -> bool {
    // Blender: "BLENDER" (7 bytes), then pointer-size byte @7 in {'-'=32-bit, '_'=64-bit},
    // then endian byte @8 in {'v'=little-endian, 'V'=big-endian}.
    if need(data, pos, 9) {
        return true;
    }
    let ptr = data[pos + 7];
    let endian = data[pos + 8];
    matches!(ptr, b'-' | b'_') && matches!(endian, b'v' | b'V')
}

fn validate_indd(data: &[u8], pos: usize) -> bool {
    // Adobe InDesign: 16-byte GUID is globally unique — already encoded in TOML magic.
    // Just confirm sufficient data present.
    !need(data, pos, 16)
}

fn validate_wtv(data: &[u8], pos: usize) -> bool {
    // Windows WTV: 16-byte GUID is globally unique — already encoded in TOML magic.
    // Just confirm sufficient data present.
    !need(data, pos, 16)
}

fn validate_php(data: &[u8], pos: usize) -> bool {
    // PHP: "<?php" opener (5 bytes already matched by magic).
    // Byte @5 must be whitespace: space, tab, CR, or LF.
    // This rejects `<?phpinfo()` style obfuscation while accepting all standard
    // PHP files (`<?php `, `<?php\n`, `<?php\r\n`, `<?php\t`).
    if need(data, pos, 6) {
        return true;
    }
    matches!(data[pos + 5], b' ' | b'\t' | b'\r' | b'\n')
}

fn validate_shebang(data: &[u8], pos: usize) -> bool {
    // Unix shebang: "#!/" must be followed by a valid interpreter path.
    // Real shebangs: "#!/bin/sh", "#!/usr/bin/env", "#!/usr/bin/python", etc.
    // After '/', byte 3 must be 'b' (bin) or 'u' (usr) — covers all standard paths.
    // Then we verify the next ~30 bytes are printable ASCII (interpreter path),
    // rejecting binary data that coincidentally contains "#!/".
    if need(data, pos, 7) {
        return true;
    }
    if data[pos + 2] != b'/' {
        return false;
    }
    let b3 = data[pos + 3];
    if b3 != b'b' && b3 != b'u' {
        return false;
    }
    // Check bytes 4..6 are printable ASCII (part of path like "in/" or "sr/")
    for i in 4..7 {
        let b = data[pos + i];
        if !(0x20..=0x7E).contains(&b) {
            return false;
        }
    }
    true
}

fn validate_crw(data: &[u8], pos: usize) -> bool {
    // Canon CRW (CIFF): magic `II\x1A\x00\x00\x00HEAPCCDR` — 14 bytes total.
    // The pre-validator just confirms enough bytes are present; the magic
    // already encodes the full 14-byte signature.
    if need(data, pos, 14) {
        return true;
    }
    &data[pos + 6..pos + 14] == b"HEAPCCDR"
}

fn validate_mrw(data: &[u8], pos: usize) -> bool {
    // Minolta MRW: `\x00MRM` header (4 bytes), then 4-byte length field,
    // then first block tag at offset 8.  Tag must be `PRD`, `TTW`, or `WBG`.
    if need(data, pos, 12) {
        return true;
    }
    let tag = &data[pos + 8..pos + 11];
    tag == b"PRD" || tag == b"TTW" || tag == b"WBG"
}

fn validate_kdbx(data: &[u8], pos: usize) -> bool {
    // KeePass 2.x: sig1 (4B) + sig2 (4B) + minor (u16 LE) + major (u16 LE).
    // Major version at offset 10 must be 3 or 4.
    if need(data, pos, 12) {
        return true;
    }
    let major = u16::from_le_bytes([data[pos + 10], data[pos + 11]]);
    major == 3 || major == 4
}

fn validate_kdb(data: &[u8], pos: usize) -> bool {
    // KeePass 1.x: sig1 (4B) + sig2 (4B) + minor (u16 LE) + major (u16 LE).
    // Major version at offset 10 must be 1 or 2.
    if need(data, pos, 12) {
        return true;
    }
    let major = u16::from_le_bytes([data[pos + 10], data[pos + 11]]);
    major == 1 || major == 2
}

fn validate_e01(data: &[u8], pos: usize) -> bool {
    // EnCase EVF/E01: 8-byte magic `EVF\x09\x0D\x0A\xFF\x00`, then segment
    // number (u16 LE @8).  Only accept the first segment (== 1).
    if need(data, pos, 10) {
        return true;
    }
    let segment = u16::from_le_bytes([data[pos + 8], data[pos + 9]]);
    segment == 1
}

fn validate_pcap(data: &[u8], pos: usize) -> bool {
    // PCAP: detect byte order from magic, then validate major == 2, minor == 4.
    if need(data, pos, 8) {
        return true;
    }
    let (major, minor) = if data[pos..pos + 4] == [0xD4, 0xC3, 0xB2, 0xA1] {
        // Little-endian
        (
            u16::from_le_bytes([data[pos + 4], data[pos + 5]]),
            u16::from_le_bytes([data[pos + 6], data[pos + 7]]),
        )
    } else {
        // Big-endian (A1 B2 C3 D4)
        (
            u16::from_be_bytes([data[pos + 4], data[pos + 5]]),
            u16::from_be_bytes([data[pos + 6], data[pos + 7]]),
        )
    };
    major == 2 && minor == 4
}

fn validate_dmp(data: &[u8], pos: usize) -> bool {
    // Windows Minidump: `MDMP` (4B) + version (2B) + stream_count (u32 LE @8).
    // stream_count must be > 0.
    if need(data, pos, 12) {
        return true;
    }
    let stream_count =
        u32::from_le_bytes([data[pos + 8], data[pos + 9], data[pos + 10], data[pos + 11]]);
    stream_count > 0
}

fn validate_plist(data: &[u8], pos: usize) -> bool {
    // Apple binary plist: `bplist00` magic (8 bytes).  Must be at least 34
    // bytes long (8 magic + some data + 26-byte trailer).
    // The magic is already encoded in the TOML header; just enforce min length.
    !need(data, pos, 34)
}

fn validate_ts(data: &[u8], pos: usize) -> bool {
    // MPEG-TS: sync byte 0x47 at stride 188.  Require at least 3 consecutive
    // sync bytes at offsets 0, 188, and 376 relative to pos.
    if need(data, pos, 377) {
        return true;
    }
    if data[pos] != 0x47 || data[pos + 188] != 0x47 || data[pos + 376] != 0x47 {
        return false;
    }
    // When enough data is available, require 2 more sync bytes (packets 4 and 5).
    // This drops false-positive probability from (1/256)^2 to (1/256)^4 per
    // candidate position, effectively eliminating random hits on a TB-size drive.
    if pos + 753 <= data.len() && (data[pos + 564] != 0x47 || data[pos + 752] != 0x47) {
        return false;
    }
    true
}

fn validate_m2ts(data: &[u8], pos: usize) -> bool {
    // Blu-ray M2TS: each 192-byte packet starts with a 4-byte timestamp, then
    // a 188-byte MPEG-TS packet (sync byte 0x47 at offset 4 within each packet).
    // Require sync bytes at offsets 4, 196, and 388 relative to pos.
    if need(data, pos, 389) {
        return true;
    }
    if data[pos + 4] != 0x47 || data[pos + 196] != 0x47 || data[pos + 388] != 0x47 {
        return false;
    }
    // When enough data is available, require 2 more sync bytes (packets 4 and 5).
    if pos + 773 <= data.len() && (data[pos + 580] != 0x47 || data[pos + 772] != 0x47) {
        return false;
    }
    true
}

fn validate_luks(data: &[u8], pos: usize) -> bool {
    // LUKS: `LUKS\xBA\xBE` magic (6 bytes), then version (u16 BE @6) in {1, 2}.
    if need(data, pos, 8) {
        return true;
    }
    let version = u16::from_be_bytes([data[pos + 6], data[pos + 7]]);
    version == 1 || version == 2
}

fn validate_x3f(data: &[u8], pos: usize) -> bool {
    // Sigma X3F: `FOVb` magic (4 bytes), then version (u32 LE @4).
    // Major version byte (little-endian high byte at offset 5) must be 2 or 3.
    if need(data, pos, 8) {
        return true;
    }
    let major = data[pos + 5]; // u32 LE: byte 4 = minor, byte 5 = major
    major == 2 || major == 3
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

    // ── JPEG DQT ──────────────────────────────────────────────────────────────

    fn make_jpeg_dqt(dqt_len: u16) -> Vec<u8> {
        // FF D8 FF DB [len_hi] [len_lo] + padding
        let mut data = vec![0u8; 8];
        data[0] = 0xFF;
        data[1] = 0xD8;
        data[2] = 0xFF;
        data[3] = 0xDB;
        let [hi, lo] = dqt_len.to_be_bytes();
        data[4] = hi;
        data[5] = lo;
        data
    }

    #[test]
    fn jpeg_dqt_valid_one_table_accepted() {
        // DQT length 67 = minimum (1 eight-bit table: 2 len + 1 precision + 64 values).
        let data = make_jpeg_dqt(67);
        assert!(validate_jpeg_dqt(&data, 0));
    }

    #[test]
    fn jpeg_dqt_valid_two_tables_accepted() {
        // DQT length 132 = 2+2*65, two 8-bit tables packed in one segment.
        let data = make_jpeg_dqt(132);
        assert!(validate_jpeg_dqt(&data, 0));
    }

    #[test]
    fn jpeg_dqt_too_short_rejected() {
        // Length below minimum (67) → rejected.
        let data = make_jpeg_dqt(20);
        assert!(!validate_jpeg_dqt(&data, 0));
    }

    #[test]
    fn jpeg_dqt_too_long_rejected() {
        // Length above maximum (518) → likely not a DQT → rejected.
        let data = make_jpeg_dqt(600);
        assert!(!validate_jpeg_dqt(&data, 0));
    }

    #[test]
    fn jpeg_dqt_truncated_accepted() {
        // Fewer than 6 bytes available — give benefit of doubt.
        let data = vec![0xFF, 0xD8, 0xFF, 0xDB];
        assert!(validate_jpeg_dqt(&data, 0));
    }

    // ── JPEG COM ──────────────────────────────────────────────────────────────

    #[test]
    fn jpeg_com_valid_accepted() {
        // FF D8 FF FE with COM length = 20 (valid comment segment).
        let mut data = vec![0u8; 8];
        data[0..4].copy_from_slice(&[0xFF, 0xD8, 0xFF, 0xFE]);
        data[4..6].copy_from_slice(&20u16.to_be_bytes());
        assert!(validate_jpeg_com(&data, 0));
    }

    #[test]
    fn jpeg_com_zero_length_rejected() {
        // COM length = 0 → invalid (must be ≥ 2).
        let mut data = vec![0u8; 6];
        data[0..4].copy_from_slice(&[0xFF, 0xD8, 0xFF, 0xFE]);
        data[4..6].copy_from_slice(&0u16.to_be_bytes());
        assert!(!validate_jpeg_com(&data, 0));
    }

    #[test]
    fn jpeg_com_truncated_accepted() {
        // Fewer than 6 bytes — benefit of doubt.
        let data = vec![0xFF, 0xD8, 0xFF, 0xFE];
        assert!(validate_jpeg_com(&data, 0));
    }

    // ── Java CLASS ────────────────────────────────────────────────────────────

    fn make_class(major: u16) -> Vec<u8> {
        let mut data = vec![0u8; 8];
        data[0..4].copy_from_slice(&[0xCA, 0xFE, 0xBA, 0xBE]);
        data[4..6].copy_from_slice(&0u16.to_be_bytes()); // minor
        data[6..8].copy_from_slice(&major.to_be_bytes());
        data
    }

    #[test]
    fn java_class_java8_accepted() {
        // Java 8 = major version 52.
        assert!(validate_java_class(&make_class(52), 0));
    }

    #[test]
    fn java_class_java21_accepted() {
        // Java 21 (LTS) = major version 65.
        assert!(validate_java_class(&make_class(65), 0));
    }

    #[test]
    fn java_class_too_old_rejected() {
        // Version 44 is before Java 1 (45) — likely not a class file.
        assert!(!validate_java_class(&make_class(44), 0));
    }

    #[test]
    fn java_class_too_new_rejected() {
        // Version 200 is far beyond current Java — likely false positive.
        assert!(!validate_java_class(&make_class(200), 0));
    }

    // ── CAB ───────────────────────────────────────────────────────────────────

    fn make_cab(reserved1: u32, cab_size: u32) -> Vec<u8> {
        let mut data = vec![0u8; 12];
        data[0..4].copy_from_slice(b"MSCF");
        data[4..8].copy_from_slice(&reserved1.to_le_bytes());
        data[8..12].copy_from_slice(&cab_size.to_le_bytes());
        data
    }

    #[test]
    fn cab_valid_accepted() {
        assert!(validate_cab(&make_cab(0, 4096), 0));
    }

    #[test]
    fn cab_nonzero_reserved_rejected() {
        assert!(!validate_cab(&make_cab(1, 4096), 0));
    }

    #[test]
    fn cab_zero_size_rejected() {
        assert!(!validate_cab(&make_cab(0, 0), 0));
    }

    // ── OTF ───────────────────────────────────────────────────────────────────

    #[test]
    fn otf_valid_accepted() {
        // OTTO + numTables = 14 (typical font).
        let mut data = vec![0u8; 6];
        data[0..4].copy_from_slice(b"OTTO");
        data[4..6].copy_from_slice(&14u16.to_be_bytes());
        assert!(validate_otf(&data, 0));
    }

    #[test]
    fn otf_zero_tables_rejected() {
        let mut data = vec![0u8; 6];
        data[0..4].copy_from_slice(b"OTTO");
        data[4..6].copy_from_slice(&0u16.to_be_bytes());
        assert!(!validate_otf(&data, 0));
    }

    #[test]
    fn otf_too_many_tables_rejected() {
        let mut data = vec![0u8; 6];
        data[0..4].copy_from_slice(b"OTTO");
        data[4..6].copy_from_slice(&200u16.to_be_bytes());
        assert!(!validate_otf(&data, 0));
    }

    // ── WOFF2 ─────────────────────────────────────────────────────────────────

    fn make_woff2(flavor: u32, num_tables: u16) -> Vec<u8> {
        let mut data = vec![0u8; 14];
        data[0..4].copy_from_slice(b"wOF2");
        data[4..8].copy_from_slice(&flavor.to_be_bytes());
        // length @8 (skip), numTables @12
        data[12..14].copy_from_slice(&num_tables.to_be_bytes());
        data
    }

    #[test]
    fn woff2_truetype_flavor_accepted() {
        assert!(validate_woff2(&make_woff2(0x0001_0000, 14), 0));
    }

    #[test]
    fn woff2_cff_flavor_accepted() {
        assert!(validate_woff2(&make_woff2(0x4F54_544F, 10), 0));
    }

    #[test]
    fn woff2_bad_flavor_rejected() {
        assert!(!validate_woff2(&make_woff2(0xDEAD_BEEF, 14), 0));
    }

    #[test]
    fn woff2_zero_tables_rejected() {
        assert!(!validate_woff2(&make_woff2(0x0001_0000, 0), 0));
    }

    // ── DEX ───────────────────────────────────────────────────────────────────

    fn make_dex(version: &[u8; 3]) -> Vec<u8> {
        let mut data = vec![0u8; 8];
        data[0..4].copy_from_slice(b"dex\n");
        data[4..7].copy_from_slice(version);
        data[7] = 0x00;
        data
    }

    #[test]
    fn dex_v035_accepted() {
        assert!(validate_dex(&make_dex(b"035"), 0));
    }

    #[test]
    fn dex_v039_accepted() {
        assert!(validate_dex(&make_dex(b"039"), 0));
    }

    #[test]
    fn dex_non_digit_version_rejected() {
        // Version "abc" is not digits — rejected.
        let mut data = make_dex(b"035");
        data[4] = b'a';
        assert!(!validate_dex(&data, 0));
    }

    #[test]
    fn dex_missing_null_terminator_rejected() {
        // Non-null terminator — rejected.
        let mut data = make_dex(b"035");
        data[7] = 0xFF;
        assert!(!validate_dex(&data, 0));
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
        v.extend_from_slice(b"\x9F"); // VINT: single-byte size = 31
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
    fn mkv_webm_doctype_rejected() {
        // WebM now has its own signature; the MKV validator rejects "webm".
        let data = make_mkv_header(b"webm");
        assert!(!validate_mkv(&data, 0));
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
        v[0..4].copy_from_slice(b"II\x2A\x00"); // TIFF LE magic
        v[4..8].copy_from_slice(&8u32.to_le_bytes()); // IFD at offset 8
        v[8..10].copy_from_slice(&12u16.to_le_bytes()); // 12 IFD entries
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

    // ── RAR ───────────────────────────────────────────────────────────────────

    fn make_rar4(flags: u16) -> Vec<u8> {
        let mut d = vec![0u8; 12];
        d[0..7].copy_from_slice(b"Rar!\x1a\x07\x00");
        d[7] = 0x00; // CRC lo
        d[8] = 0x00; // CRC hi
        d[9] = 0x73; // HEAD_TYPE = archive header
        d[10..12].copy_from_slice(&flags.to_le_bytes());
        d
    }

    #[test]
    fn rar4_standalone_accepted() {
        assert!(validate_rar(&make_rar4(0x0000), 0));
    }

    #[test]
    fn rar4_first_volume_accepted() {
        assert!(validate_rar(&make_rar4(0x0101), 0));
    }

    #[test]
    fn rar4_continuation_volume_rejected() {
        // Volume bit set, first-volume bit NOT set
        assert!(!validate_rar(&make_rar4(0x0001), 0));
    }

    #[test]
    fn rar4_continuation_with_extra_flags_rejected() {
        // flags = 0x0011 — volume + lock, but not first volume
        assert!(!validate_rar(&make_rar4(0x0011), 0));
    }

    #[test]
    fn rar4_bad_head_type_rejected() {
        let mut d = make_rar4(0x0000);
        d[9] = 0x74; // file header, not archive header
        assert!(!validate_rar(&d, 0));
    }

    #[test]
    fn rar5_type_accepted() {
        let mut d = vec![0u8; 12];
        d[0..7].copy_from_slice(b"Rar!\x1a\x07\x01");
        assert!(validate_rar(&d, 0));
    }

    #[test]
    fn rar_invalid_type_rejected() {
        let mut d = vec![0u8; 12];
        d[0..7].copy_from_slice(b"Rar!\x1a\x07\x02");
        assert!(!validate_rar(&d, 0));
    }

    #[test]
    fn rar_real_continuation_rejected() {
        // Actual bytes from a false positive: flags = 0x0001
        let d: Vec<u8> = vec![
            0x52, 0x61, 0x72, 0x21, 0x1a, 0x07, 0x00, 0xf1, 0xfb, 0x73, 0x01, 0x00,
        ];
        assert!(!validate_rar(&d, 0));
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
        assert!(validate_tiff_le(&data, 0));
        assert!(validate_tiff_be(&data, 0));
        assert!(validate_nef(&data, 0));
        assert!(validate_heic(&data, 0));
        assert!(validate_mov(&data, 0));
        assert!(validate_m4v(&data, 0));
        assert!(validate_3gp(&data, 0));
        assert!(validate_webm(&data, 0));
        assert!(validate_wmv(&data, 0));
        assert!(validate_flv(&data, 0));
        assert!(validate_mpeg(&data, 0));
    }

    // ── TIFF LE ───────────────────────────────────────────────────────────────

    fn make_tiff_le_header(ifd_off: u32, entry_count: u16, maker: Option<&[u8]>) -> Vec<u8> {
        let mut v = vec![0u8; 512];
        v[0..4].copy_from_slice(b"II\x2A\x00");
        v[4..8].copy_from_slice(&ifd_off.to_le_bytes());
        if ifd_off < 512 {
            let pos = ifd_off as usize;
            v[pos..pos + 2].copy_from_slice(&entry_count.to_le_bytes());
            // Write a minimal ImageWidth (tag 256, SHORT, count=1, value=100) as
            // the first IFD entry so the ImageWidth presence check passes.
            if entry_count >= 1 && pos + 2 + 12 <= 512 {
                let e = pos + 2;
                v[e..e + 2].copy_from_slice(&256u16.to_le_bytes()); // tag
                v[e + 2..e + 4].copy_from_slice(&3u16.to_le_bytes()); // SHORT
                v[e + 4..e + 8].copy_from_slice(&1u32.to_le_bytes()); // count=1
                v[e + 8..e + 12].copy_from_slice(&100u32.to_le_bytes()); // value
            }
        }
        if let Some(m) = maker {
            v[100..100 + m.len()].copy_from_slice(m);
        }
        v
    }

    #[test]
    fn tiff_le_plain_accepted() {
        let data = make_tiff_le_header(8, 12, None);
        assert!(validate_tiff_le(&data, 0));
    }

    #[test]
    fn tiff_le_rejects_sony_arw() {
        let data = make_tiff_le_header(8, 12, Some(b"SONY"));
        assert!(!validate_tiff_le(&data, 0));
    }

    #[test]
    fn tiff_le_rejects_nikon_nef() {
        let data = make_tiff_le_header(8, 12, Some(b"NIKON"));
        assert!(!validate_tiff_le(&data, 0));
    }

    #[test]
    fn tiff_le_rejects_canon_cr2() {
        let mut data = make_tiff_le_header(16, 12, None);
        // Place Canon CR2 marker at offset 8
        data[8..12].copy_from_slice(b"CR\x02\x00");
        assert!(!validate_tiff_le(&data, 0));
    }

    #[test]
    fn tiff_le_bad_ifd_offset_rejected() {
        let data = make_tiff_le_header(100_000, 12, None); // IFD offset > 65536
        assert!(!validate_tiff_le(&data, 0));
    }

    #[test]
    fn tiff_le_rejects_exif_only_ifd0() {
        // Simulates an EXIF block embedded inside a JPEG: IFD0 has metadata tags
        // but no ImageWidth (tag 256).  This is the false-positive produced when
        // the scanner finds II\x2A\x00 inside a JPEG APP1 segment.
        let mut data = vec![0u8; 512];
        data[0..4].copy_from_slice(b"II\x2A\x00");
        data[4..8].copy_from_slice(&8u32.to_le_bytes()); // IFD at 8
        data[8..10].copy_from_slice(&3u16.to_le_bytes()); // 3 entries
                                                          // Entry 0: Make (tag 271) — not ImageWidth
        let e0 = 10usize;
        data[e0..e0 + 2].copy_from_slice(&271u16.to_le_bytes());
        // Entry 1: Model (tag 272)
        let e1 = e0 + 12;
        data[e1..e1 + 2].copy_from_slice(&272u16.to_le_bytes());
        // Entry 2: ExifIFD (tag 34665) — still no ImageWidth
        let e2 = e1 + 12;
        data[e2..e2 + 2].copy_from_slice(&34665u16.to_le_bytes());
        assert!(!validate_tiff_le(&data, 0));
    }

    #[test]
    fn tiff_le_accepts_when_ifd_beyond_chunk() {
        // IFD falls outside the available data — validator must pass through
        // rather than rejecting, because it cannot know yet.
        let mut data = vec![0u8; 16]; // only 16 bytes available
        data[0..4].copy_from_slice(b"II\x2A\x00");
        data[4..8].copy_from_slice(&8u32.to_le_bytes());
        data[8..10].copy_from_slice(&3u16.to_le_bytes()); // entries beyond 16 bytes
        assert!(validate_tiff_le(&data, 0));
    }

    // ── TIFF BE ───────────────────────────────────────────────────────────────

    #[test]
    fn tiff_be_valid_accepted() {
        let mut data = vec![0u8; 512];
        data[0..4].copy_from_slice(b"MM\x00\x2A");
        data[4..8].copy_from_slice(&8u32.to_be_bytes()); // IFD at 8
        data[8..10].copy_from_slice(&10u16.to_be_bytes()); // 10 entries
                                                           // ImageWidth (tag 256) as first IFD entry (BE)
        data[10..12].copy_from_slice(&256u16.to_be_bytes()); // tag
        data[12..14].copy_from_slice(&3u16.to_be_bytes()); // SHORT
        data[14..18].copy_from_slice(&1u32.to_be_bytes()); // count=1
        data[18..22].copy_from_slice(&100u32.to_be_bytes()); // value
        assert!(validate_tiff_be(&data, 0));
    }

    #[test]
    fn tiff_be_bad_ifd_offset_rejected() {
        let mut data = vec![0u8; 8];
        data[0..4].copy_from_slice(b"MM\x00\x2A");
        data[4..8].copy_from_slice(&0u32.to_be_bytes()); // IFD at 0 — invalid
        assert!(!validate_tiff_be(&data, 0));
    }

    // ── NEF ───────────────────────────────────────────────────────────────────

    #[test]
    fn nef_with_nikon_string_accepted() {
        let data = make_tiff_le_header(8, 20, Some(b"NIKON"));
        assert!(validate_nef(&data, 0));
    }

    #[test]
    fn nef_without_nikon_string_rejected() {
        let data = make_tiff_le_header(8, 20, None);
        assert!(!validate_nef(&data, 0));
    }

    #[test]
    fn nef_bad_ifd_offset_rejected() {
        // IFD offset > 4096 — rejected even with NIKON string.
        let mut data = vec![0u8; 512];
        data[0..4].copy_from_slice(b"II\x2A\x00");
        data[4..8].copy_from_slice(&8000u32.to_le_bytes()); // too large
        data[100..105].copy_from_slice(b"NIKON");
        assert!(!validate_nef(&data, 0));
    }

    // ── HEIC ──────────────────────────────────────────────────────────────────

    fn make_heic_ftyp(box_size: u32, brand: &[u8; 4]) -> Vec<u8> {
        let mut data = vec![0u8; 24];
        data[0..4].copy_from_slice(&box_size.to_be_bytes());
        data[4..8].copy_from_slice(b"ftyp");
        data[8..12].copy_from_slice(brand);
        data
    }

    #[test]
    fn heic_valid_box_size_accepted() {
        let data = make_heic_ftyp(24, b"heic");
        assert!(validate_heic(&data, 0));
    }

    #[test]
    fn heic_heix_brand_accepted() {
        let data = make_heic_ftyp(28, b"heix");
        assert!(validate_heic(&data, 0));
    }

    #[test]
    fn heic_too_small_box_rejected() {
        let data = make_heic_ftyp(8, b"heic"); // box_size < 12
        assert!(!validate_heic(&data, 0));
    }

    #[test]
    fn heic_oversized_box_rejected() {
        let data = make_heic_ftyp(1024, b"heic"); // box_size > 512
        assert!(!validate_heic(&data, 0));
    }

    // ── MOV ───────────────────────────────────────────────────────────────────

    fn make_isobmff_ftyp(box_size: u32, brand: &[u8; 4]) -> Vec<u8> {
        let mut data = vec![0u8; box_size.max(32) as usize];
        data[0..4].copy_from_slice(&box_size.to_be_bytes());
        data[4..8].copy_from_slice(b"ftyp");
        data[8..12].copy_from_slice(brand);
        data
    }

    #[test]
    fn mov_valid_box_accepted() {
        let data = make_isobmff_ftyp(20, b"qt  ");
        assert!(validate_mov(&data, 0));
    }

    #[test]
    fn mov_oversized_box_rejected() {
        let data = make_isobmff_ftyp(1024, b"qt  "); // box_size > 512
        assert!(!validate_mov(&data, 0));
    }

    #[test]
    fn mp4_rejects_qt_brand() {
        let data = make_isobmff_ftyp(20, b"qt  ");
        assert!(!validate_mp4(&data, 0));
    }

    #[test]
    fn mp4_rejects_jp2_brand() {
        // Real-world false positive: JPEG 2000 files carved as MP4.
        let data = make_isobmff_ftyp(20, b"jp2 ");
        assert!(!validate_mp4(&data, 0));
    }

    #[test]
    fn mp4_rejects_jpx_brand() {
        let data = make_isobmff_ftyp(20, b"jpx ");
        assert!(!validate_mp4(&data, 0));
    }

    #[test]
    fn mp4_rejects_jpm_brand() {
        let data = make_isobmff_ftyp(20, b"jpm ");
        assert!(!validate_mp4(&data, 0));
    }

    #[test]
    fn mp4_rejects_m4a_brand() {
        let data = make_isobmff_ftyp(20, b"M4A ");
        assert!(!validate_mp4(&data, 0));
    }

    #[test]
    fn mp4_rejects_heic_brand() {
        let data = make_isobmff_ftyp(20, b"heic");
        assert!(!validate_mp4(&data, 0));
    }

    #[test]
    fn mp4_rejects_avif_brand() {
        let data = make_isobmff_ftyp(20, b"avif");
        assert!(!validate_mp4(&data, 0));
    }

    #[test]
    fn mp4_rejects_crx_brand() {
        let data = make_isobmff_ftyp(20, b"crx ");
        assert!(!validate_mp4(&data, 0));
    }

    // ── M4V ───────────────────────────────────────────────────────────────────

    #[test]
    fn m4v_valid_box_accepted() {
        let data = make_isobmff_ftyp(20, b"M4V ");
        assert!(validate_m4v(&data, 0));
    }

    #[test]
    fn mp4_rejects_m4v_brand() {
        let data = make_isobmff_ftyp(20, b"M4V ");
        assert!(!validate_mp4(&data, 0));
    }

    // ── 3GP ───────────────────────────────────────────────────────────────────

    #[test]
    fn three_gp_brand_accepted() {
        let data = make_isobmff_ftyp(20, b"3gp5");
        assert!(validate_3gp(&data, 0));
    }

    #[test]
    fn three_g2_brand_accepted() {
        let data = make_isobmff_ftyp(20, b"3g2a");
        assert!(validate_3gp(&data, 0));
    }

    #[test]
    fn three_gp_wrong_brand_rejected() {
        let data = make_isobmff_ftyp(20, b"isom");
        assert!(!validate_3gp(&data, 0));
    }

    #[test]
    fn mp4_rejects_3gp_brand() {
        let data = make_isobmff_ftyp(20, b"3gp5");
        assert!(!validate_mp4(&data, 0));
    }

    // ── WebM ──────────────────────────────────────────────────────────────────

    #[test]
    fn webm_doctype_accepted() {
        let data = make_mkv_header(b"webm");
        assert!(validate_webm(&data, 0));
    }

    #[test]
    fn webm_rejects_matroska_doctype() {
        let data = make_mkv_header(b"matroska");
        assert!(!validate_webm(&data, 0));
    }

    // ── WMV ───────────────────────────────────────────────────────────────────

    fn make_asf_header(object_size: u64) -> Vec<u8> {
        let mut data = vec![0u8; 32];
        // ASF Header Object GUID
        data[0..16].copy_from_slice(&[
            0x30, 0x26, 0xB2, 0x75, 0x8E, 0x66, 0xCF, 0x11, 0xA6, 0xD9, 0x00, 0xAA, 0x00, 0x62,
            0xCE, 0x6C,
        ]);
        data[16..24].copy_from_slice(&object_size.to_le_bytes());
        data
    }

    #[test]
    fn wmv_valid_size_accepted() {
        let data = make_asf_header(1024);
        assert!(validate_wmv(&data, 0));
    }

    #[test]
    fn wmv_tiny_size_rejected() {
        let data = make_asf_header(10); // size < 30
        assert!(!validate_wmv(&data, 0));
    }

    // ── FLV ───────────────────────────────────────────────────────────────────

    fn make_flv_header(flags: u8, data_offset: u32) -> Vec<u8> {
        let mut data = vec![0u8; 9];
        data[0..4].copy_from_slice(b"FLV\x01");
        data[4] = flags;
        data[5..9].copy_from_slice(&data_offset.to_be_bytes());
        data
    }

    #[test]
    fn flv_video_only_accepted() {
        // TypeFlagsVideo bit (bit 0) set, all reserved bits zero, offset 9.
        let data = make_flv_header(0x01, 9);
        assert!(validate_flv(&data, 0));
    }

    #[test]
    fn flv_audio_video_accepted() {
        // TypeFlagsAudio (bit 2) + TypeFlagsVideo (bit 0), offset 9.
        let data = make_flv_header(0x05, 9);
        assert!(validate_flv(&data, 0));
    }

    #[test]
    fn flv_bad_reserved_bits_rejected() {
        let data = make_flv_header(0b0000_0010, 9); // reserved bit 1 set
        assert!(!validate_flv(&data, 0));
    }

    #[test]
    fn flv_wrong_offset_rejected() {
        let data = make_flv_header(0x01, 5); // DataOffset != 9
        assert!(!validate_flv(&data, 0));
    }

    // ── MPEG-PS ───────────────────────────────────────────────────────────────

    fn make_mpeg2_pack() -> Vec<u8> {
        // Minimal valid MPEG-2 PS pack header with all marker bits set.
        // ISO 13818-1 Table 2-33.
        let mut data = vec![0u8; 14];
        data[0..4].copy_from_slice(b"\x00\x00\x01\xBA");
        data[4] = 0x44; // 01 000 1 00 — top-2=01, marker(bit2)=1
                        // data[5] = 0x00; // SCR bytes — no markers required
        data[6] = 0x04; // SCR_b[19:15] 1 SCR_b[14:13] — marker(bit2)=1
                        // data[7] = 0x00; // SCR bytes
        data[8] = 0x04; // SCR_b[4:0] 1 SCR_ext[8:7] — marker(bit2)=1
        data[9] = 0x01; // SCR_ext[6:0] 1 — marker(bit0)=1
                        // data[10..12] = mux_rate bytes
        data[12] = 0x03; // mux_rate[5:0] 1 1 — both marker bits set
        data[13] = 0xF8; // 11111 stuffing_length — fixed top-5 bits
        data
    }

    #[test]
    fn mpeg2_pack_header_accepted() {
        assert!(validate_mpeg(&make_mpeg2_pack(), 0));
    }

    #[test]
    fn mpeg2_missing_marker_bit_rejected() {
        // Clear one of the mandatory marker bits — should be rejected.
        let mut data = make_mpeg2_pack();
        data[12] = 0x01; // only one marker bit instead of two
        assert!(!validate_mpeg(&data, 0));
    }

    #[test]
    fn mpeg2_h264_annex_b_false_positive_rejected() {
        // Simulate H.264 Annex-B data that happens to contain 00 00 01 BA.
        // The bytes following the start code are typical NAL payload — no marker bits.
        let mut data = vec![0u8; 14];
        data[0..4].copy_from_slice(b"\x00\x00\x01\xBA");
        data[4] = 0x65; // top-2 bits == 01 but marker bit (bit2) == 0  → reject
        assert!(!validate_mpeg(&data, 0));
    }

    #[test]
    fn mpeg1_pack_header_accepted() {
        let mut data = vec![0u8; 14];
        data[0..4].copy_from_slice(b"\x00\x00\x01\xBA");
        data[4] = 0x21; // 0010_0001 — top-4=0010, marker(bit0)=1
        assert!(validate_mpeg(&data, 0));
    }

    #[test]
    fn mpeg1_missing_marker_bit_rejected() {
        let mut data = vec![0u8; 14];
        data[0..4].copy_from_slice(b"\x00\x00\x01\xBA");
        data[4] = 0x20; // 0010_0000 — top-4=0010 but marker(bit0)=0
        assert!(!validate_mpeg(&data, 0));
    }

    #[test]
    fn mpeg_unknown_version_rejected() {
        let mut data = vec![0u8; 14];
        data[0..4].copy_from_slice(b"\x00\x00\x01\xBA");
        data[4] = 0x00; // neither MPEG-1 nor MPEG-2 pattern
        assert!(!validate_mpeg(&data, 0));
    }

    // ── WebP ──────────────────────────────────────────────────────────────────

    fn make_webp(riff_size: u32) -> Vec<u8> {
        let mut data = vec![0u8; 12];
        data[0..4].copy_from_slice(b"RIFF");
        data[4..8].copy_from_slice(&riff_size.to_le_bytes());
        data[8..12].copy_from_slice(b"WEBP");
        data
    }

    #[test]
    fn webp_valid_accepted() {
        let data = make_webp(1024);
        assert!(validate_webp(&data, 0));
    }

    #[test]
    fn webp_tiny_riff_size_rejected() {
        let data = make_webp(0); // size < 4
        assert!(!validate_webp(&data, 0));
    }

    #[test]
    fn webp_wrong_subtype_rejected() {
        let mut data = make_webp(1024);
        data[8..12].copy_from_slice(b"WAVE"); // not WEBP
        assert!(!validate_webp(&data, 0));
    }

    // ── M4A ───────────────────────────────────────────────────────────────────

    #[test]
    fn m4a_valid_box_accepted() {
        let data = make_isobmff_ftyp(20, b"M4A ");
        assert!(validate_m4a(&data, 0));
    }

    #[test]
    fn m4a_oversized_box_rejected() {
        let data = make_isobmff_ftyp(1024, b"M4A "); // box_size > 512
        assert!(!validate_m4a(&data, 0));
    }

    // ── GZip ──────────────────────────────────────────────────────────────────

    fn make_gz_full(cm: u8, flg: u8, xfl: u8, os: u8) -> Vec<u8> {
        let mut data = vec![0u8; 10];
        data[0] = 0x1F;
        data[1] = 0x8B;
        data[2] = cm;
        data[3] = flg;
        data[8] = xfl;
        data[9] = os;
        data
    }

    #[test]
    fn gz_deflate_method_accepted() {
        let data = make_gz_full(8, 0x00, 0, 3); // Unix
        assert!(validate_gz(&data, 0));
    }

    #[test]
    fn gz_wrong_cm_rejected() {
        let data = make_gz_full(7, 0x00, 0, 3);
        assert!(!validate_gz(&data, 0));
    }

    #[test]
    fn gz_reserved_flag_bit_rejected() {
        let data = make_gz_full(8, 0x20, 0, 3);
        assert!(!validate_gz(&data, 0));
    }

    #[test]
    fn gz_xfl_max_compression_accepted() {
        let data = make_gz_full(8, 0x00, 2, 255); // XFL=2 (max), OS=unknown
        assert!(validate_gz(&data, 0));
    }

    #[test]
    fn gz_xfl_fastest_accepted() {
        let data = make_gz_full(8, 0x00, 4, 0); // XFL=4 (fastest), OS=FAT
        assert!(validate_gz(&data, 0));
    }

    #[test]
    fn gz_invalid_xfl_rejected() {
        let data = make_gz_full(8, 0x00, 6, 3); // XFL=6 not in {0,2,4}
        assert!(!validate_gz(&data, 0));
    }

    #[test]
    fn gz_invalid_os_rejected() {
        let data = make_gz_full(8, 0x00, 0, 14); // OS=14 not in [0,13]∪{255}
        assert!(!validate_gz(&data, 0));
    }

    // ── EML ───────────────────────────────────────────────────────────────────

    fn make_eml_line(after_from: &[u8]) -> Vec<u8> {
        let mut data = Vec::with_capacity(5 + after_from.len());
        data.extend_from_slice(b"From ");
        data.extend_from_slice(after_from);
        // Pad to 85 bytes so the full 80-byte window is available
        while data.len() < 85 {
            data.push(b' ');
        }
        data
    }

    #[test]
    fn eml_real_mbox_from_accepted() {
        let data = make_eml_line(b"user@example.com Mon Jan  1 00:00:00 2024\n");
        assert!(validate_eml(&data, 0));
    }

    #[test]
    fn eml_mailer_daemon_accepted() {
        let data = make_eml_line(b"MAILER-DAEMON@host.com Fri Feb 14 10:30:00 2025\n");
        assert!(validate_eml(&data, 0));
    }

    #[test]
    fn eml_xmp_rdf_rejected() {
        // Real false positive: "From rdf:parseType=\"Resource\">"
        let data = make_eml_line(b"rdf:parseType=\"Resource\">\n");
        assert!(!validate_eml(&data, 0));
    }

    #[test]
    fn eml_binary_after_from_rejected() {
        let mut data = Vec::with_capacity(85);
        data.extend_from_slice(b"From ");
        data.extend_from_slice(&[0x01, 0xFF, 0x00, 0x47]); // binary garbage
        while data.len() < 85 {
            data.push(0x00);
        }
        assert!(!validate_eml(&data, 0));
    }

    #[test]
    fn eml_no_at_sign_rejected() {
        // All printable but no '@' — not an email From line
        let data = make_eml_line(b"just some text without email address here\n");
        assert!(!validate_eml(&data, 0));
    }

    #[test]
    fn eml_short_buffer_printable_accepted() {
        // Only 6 bytes — falls back to printable-char check
        let data = b"From u".to_vec();
        assert!(validate_eml(&data, 0));
    }

    #[test]
    fn eml_short_buffer_binary_rejected() {
        let data = b"From \x01".to_vec();
        assert!(!validate_eml(&data, 0));
    }

    // ── ELF ───────────────────────────────────────────────────────────────────

    fn make_elf(ei_class: u8, ei_data: u8, ei_version: u8) -> Vec<u8> {
        let mut data = vec![0u8; 16];
        data[0..4].copy_from_slice(b"\x7FELF");
        data[4] = ei_class;
        data[5] = ei_data;
        data[6] = ei_version;
        data
    }

    #[test]
    fn elf_64bit_le_accepted() {
        let data = make_elf(2, 1, 1);
        assert!(validate_elf(&data, 0));
    }

    #[test]
    fn elf_32bit_be_accepted() {
        let data = make_elf(1, 2, 1);
        assert!(validate_elf(&data, 0));
    }

    #[test]
    fn elf_invalid_class_rejected() {
        let data = make_elf(3, 1, 1); // EI_CLASS == 3 is invalid
        assert!(!validate_elf(&data, 0));
    }

    #[test]
    fn elf_wrong_version_rejected() {
        let data = make_elf(2, 1, 2); // EI_VERSION must be 1
        assert!(!validate_elf(&data, 0));
    }

    // ── REGF ──────────────────────────────────────────────────────────────────

    fn make_regf(major: u32, minor: u32) -> Vec<u8> {
        let mut data = vec![0u8; 28];
        data[0..4].copy_from_slice(b"regf");
        data[20..24].copy_from_slice(&major.to_le_bytes());
        data[24..28].copy_from_slice(&minor.to_le_bytes());
        data
    }

    #[test]
    fn regf_valid_accepted() {
        let data = make_regf(1, 3);
        assert!(validate_regf(&data, 0));
    }

    #[test]
    fn regf_wrong_major_rejected() {
        let data = make_regf(2, 3); // major must be 1
        assert!(!validate_regf(&data, 0));
    }

    #[test]
    fn regf_minor_out_of_range_rejected() {
        let data = make_regf(1, 7); // minor must be in [2,6]
        assert!(!validate_regf(&data, 0));
    }

    // ── PSD ───────────────────────────────────────────────────────────────────

    fn make_psd(version: u16, channels: u16) -> Vec<u8> {
        let mut data = vec![0u8; 26];
        data[0..4].copy_from_slice(b"8BPS");
        data[4..6].copy_from_slice(&version.to_be_bytes());
        data[6..8].copy_from_slice(&channels.to_be_bytes());
        data
    }

    #[test]
    fn psd_version1_accepted() {
        let data = make_psd(1, 3); // PSD with 3 channels
        assert!(validate_psd(&data, 0));
    }

    #[test]
    fn psd_version2_psb_accepted() {
        let data = make_psd(2, 4); // PSB with 4 channels
        assert!(validate_psd(&data, 0));
    }

    #[test]
    fn psd_wrong_version_rejected() {
        let data = make_psd(3, 3); // version 3 is invalid
        assert!(!validate_psd(&data, 0));
    }

    #[test]
    fn psd_zero_channels_rejected() {
        let data = make_psd(1, 0); // 0 channels is invalid
        assert!(!validate_psd(&data, 0));
    }

    // ── VHD ───────────────────────────────────────────────────────────────────

    fn make_vhd(disk_type: u32) -> Vec<u8> {
        let mut data = vec![0u8; 68];
        data[0..8].copy_from_slice(b"conectix");
        data[60..64].copy_from_slice(&disk_type.to_be_bytes());
        data
    }

    #[test]
    fn vhd_fixed_disk_accepted() {
        let data = make_vhd(2); // fixed
        assert!(validate_vhd(&data, 0));
    }

    #[test]
    fn vhd_dynamic_disk_accepted() {
        let data = make_vhd(3); // dynamic
        assert!(validate_vhd(&data, 0));
    }

    #[test]
    fn vhd_unknown_disk_type_rejected() {
        let data = make_vhd(5); // unknown type
        assert!(!validate_vhd(&data, 0));
    }

    // ── VHDX ──────────────────────────────────────────────────────────────────

    #[test]
    fn vhdx_valid_magic_accepted() {
        let mut data = vec![0u8; 8];
        data[0..8].copy_from_slice(b"vhdxfile");
        assert!(validate_vhdx(&data, 0));
    }

    #[test]
    fn vhdx_short_buffer_benefit_of_doubt() {
        let data = vec![b'v', b'h', b'd', b'x']; // only 4 bytes
        assert!(validate_vhdx(&data, 0));
    }

    // ── QCOW2 ─────────────────────────────────────────────────────────────────

    fn make_qcow2(version: u32, cluster_bits: u32) -> Vec<u8> {
        let mut data = vec![0u8; 24];
        data[0..4].copy_from_slice(b"QFI\xFB");
        data[4..8].copy_from_slice(&version.to_be_bytes());
        data[20..24].copy_from_slice(&cluster_bits.to_be_bytes());
        data
    }

    #[test]
    fn qcow2_version2_accepted() {
        let data = make_qcow2(2, 16);
        assert!(validate_qcow2(&data, 0));
    }

    #[test]
    fn qcow2_version3_accepted() {
        let data = make_qcow2(3, 12);
        assert!(validate_qcow2(&data, 0));
    }

    #[test]
    fn qcow2_wrong_version_rejected() {
        let data = make_qcow2(1, 16); // version 1 is QCOW, not QCOW2
        assert!(!validate_qcow2(&data, 0));
    }

    #[test]
    fn qcow2_cluster_bits_too_small_rejected() {
        let data = make_qcow2(2, 8); // cluster_bits < 9
        assert!(!validate_qcow2(&data, 0));
    }

    #[test]
    fn qcow2_cluster_bits_too_large_rejected() {
        let data = make_qcow2(2, 22); // cluster_bits > 21
        assert!(!validate_qcow2(&data, 0));
    }

    // ── MIDI ──────────────────────────────────────────────────────────────────

    fn make_midi(format: u16) -> Vec<u8> {
        let mut d = vec![0u8; 20];
        d[0..4].copy_from_slice(b"MThd");
        d[4..8].copy_from_slice(&6u32.to_be_bytes());
        d[8..10].copy_from_slice(&format.to_be_bytes());
        d[10..12].copy_from_slice(&1u16.to_be_bytes()); // num_tracks
        d[12..14].copy_from_slice(&480u16.to_be_bytes()); // division
        d
    }

    #[test]
    fn midi_format0_accepted() {
        assert!(validate_midi(&make_midi(0), 0));
    }

    #[test]
    fn midi_format2_accepted() {
        assert!(validate_midi(&make_midi(2), 0));
    }

    #[test]
    fn midi_bad_format_rejected() {
        assert!(!validate_midi(&make_midi(3), 0));
    }

    #[test]
    fn midi_wrong_chunk_len_rejected() {
        let mut d = make_midi(1);
        d[4..8].copy_from_slice(&7u32.to_be_bytes()); // must be 6
        assert!(!validate_midi(&d, 0));
    }

    // ── AIFF ──────────────────────────────────────────────────────────────────

    fn make_aiff(subtype: u8) -> Vec<u8> {
        let mut d = vec![0u8; 16];
        d[0..4].copy_from_slice(b"FORM");
        d[4..8].copy_from_slice(&100u32.to_be_bytes());
        d[8..11].copy_from_slice(b"AIF");
        d[11] = subtype;
        d
    }

    #[test]
    fn aiff_classic_accepted() {
        assert!(validate_aiff(&make_aiff(b'F'), 0));
    }

    #[test]
    fn aifc_accepted() {
        assert!(validate_aiff(&make_aiff(b'C'), 0));
    }

    #[test]
    fn aiff_wrong_subtype_rejected() {
        assert!(!validate_aiff(&make_aiff(b'X'), 0));
    }

    // ── XZ ────────────────────────────────────────────────────────────────────

    fn make_xz(reserved: u8, check: u8) -> Vec<u8> {
        let mut d = vec![0u8; 12];
        d[0..6].copy_from_slice(&[0xFD, 0x37, 0x7A, 0x58, 0x5A, 0x00]);
        d[6] = reserved;
        d[7] = check;
        d
    }

    #[test]
    fn xz_crc32_accepted() {
        assert!(validate_xz(&make_xz(0x00, 0x01), 0));
    }

    #[test]
    fn xz_sha256_accepted() {
        assert!(validate_xz(&make_xz(0x00, 0x0A), 0));
    }

    #[test]
    fn xz_bad_reserved_rejected() {
        assert!(!validate_xz(&make_xz(0x01, 0x01), 0));
    }

    #[test]
    fn xz_bad_check_type_rejected() {
        assert!(!validate_xz(&make_xz(0x00, 0x02), 0));
    }

    // ── BZip2 ─────────────────────────────────────────────────────────────────

    fn make_bzip2(level: u8) -> Vec<u8> {
        let mut d = vec![0u8; 12];
        d[0..3].copy_from_slice(b"BZh");
        d[3] = level;
        d[4..10].copy_from_slice(&[0x31, 0x41, 0x59, 0x26, 0x53, 0x59]);
        d
    }

    #[test]
    fn bzip2_level1_accepted() {
        assert!(validate_bzip2(&make_bzip2(b'1'), 0));
    }

    #[test]
    fn bzip2_level9_accepted() {
        assert!(validate_bzip2(&make_bzip2(b'9'), 0));
    }

    #[test]
    fn bzip2_bad_level_rejected() {
        assert!(!validate_bzip2(&make_bzip2(b'0'), 0));
    }

    #[test]
    fn bzip2_bad_block_magic_rejected() {
        let mut d = make_bzip2(b'5');
        d[4] = 0xFF; // corrupt block magic
        assert!(!validate_bzip2(&d, 0));
    }

    // ── RealMedia ─────────────────────────────────────────────────────────────

    fn make_realmedia(version: u16, header_size: u32) -> Vec<u8> {
        let mut d = vec![0u8; 12];
        d[0..4].copy_from_slice(b".RMF");
        d[4..6].copy_from_slice(&version.to_be_bytes());
        d[6..10].copy_from_slice(&header_size.to_be_bytes());
        d
    }

    #[test]
    fn realmedia_version0_accepted() {
        assert!(validate_realmedia(&make_realmedia(0, 18), 0));
    }

    #[test]
    fn realmedia_version1_accepted() {
        assert!(validate_realmedia(&make_realmedia(1, 18), 0));
    }

    #[test]
    fn realmedia_bad_version_rejected() {
        assert!(!validate_realmedia(&make_realmedia(2, 18), 0));
    }

    #[test]
    fn realmedia_small_header_rejected() {
        assert!(!validate_realmedia(&make_realmedia(0, 17), 0));
    }

    // ── XML ──────────────────────────────────────────────────────────────────

    fn make_xml_decl(suffix: &[u8]) -> Vec<u8> {
        // "<?xml " + suffix
        let mut d = Vec::with_capacity(6 + suffix.len());
        d.extend_from_slice(b"<?xml ");
        d.extend_from_slice(suffix);
        d
    }

    #[test]
    fn xml_valid_declaration_accepted() {
        let d = make_xml_decl(b"version=\"1.0\" encoding=\"UTF-8\"?>");
        assert!(validate_xml(&d, 0));
    }

    #[test]
    fn xml_space_without_version_rejected() {
        // "<?xml something" — space present but no "version"
        let d = make_xml_decl(b"somethi");
        assert!(!validate_xml(&d, 0));
    }

    #[test]
    fn xml_random_bytes_after_space_rejected() {
        let d = make_xml_decl(&[0xFF, 0x00, 0x47, 0x11, 0x22, 0x33, 0x44]);
        assert!(!validate_xml(&d, 0));
    }

    #[test]
    fn xml_short_buffer_passes() {
        // Only 6 bytes — not enough to check "version", passes via need()
        let d = b"<?xml ".to_vec();
        assert!(validate_xml(&d, 0));
    }

    // ── ICO ───────────────────────────────────────────────────────────────────

    /// Build an ICO with one directory entry.
    fn make_ico_full(
        count: u16,
        reserved: u8,
        planes: u16,
        bpp: u16,
        data_size: u32,
        data_offset: u32,
    ) -> Vec<u8> {
        let mut d = vec![0u8; 22];
        d[0..4].copy_from_slice(&[0x00, 0x00, 0x01, 0x00]);
        d[4..6].copy_from_slice(&count.to_le_bytes());
        // entry: width=32, height=32, colors=0, reserved, planes, bpp, data_size, data_offset
        d[6] = 32;
        d[7] = 32;
        d[8] = 0;
        d[9] = reserved;
        d[10..12].copy_from_slice(&planes.to_le_bytes());
        d[12..14].copy_from_slice(&bpp.to_le_bytes());
        d[14..18].copy_from_slice(&data_size.to_le_bytes());
        d[18..22].copy_from_slice(&data_offset.to_le_bytes());
        d
    }

    fn make_ico(count: u16) -> Vec<u8> {
        let offset = 6 + 16 * count as u32;
        make_ico_full(count, 0, 1, 32, 1024, offset)
    }

    #[test]
    fn ico_count1_accepted() {
        assert!(validate_ico(&make_ico(1), 0));
    }

    #[test]
    fn ico_count200_accepted() {
        assert!(validate_ico(&make_ico(200), 0));
    }

    #[test]
    fn ico_count0_rejected() {
        assert!(!validate_ico(&make_ico(0), 0));
    }

    #[test]
    fn ico_count201_rejected() {
        assert!(!validate_ico(&make_ico(201), 0));
    }

    #[test]
    fn ico_reserved_nonzero_rejected() {
        let d = make_ico_full(1, 0xFF, 1, 32, 1024, 22);
        assert!(!validate_ico(&d, 0));
    }

    #[test]
    fn ico_planes_2_rejected() {
        let d = make_ico_full(1, 0, 2, 32, 1024, 22);
        assert!(!validate_ico(&d, 0));
    }

    #[test]
    fn ico_invalid_bpp_rejected() {
        let d = make_ico_full(1, 0, 1, 15, 1024, 22);
        assert!(!validate_ico(&d, 0));
    }

    #[test]
    fn ico_zero_data_size_rejected() {
        let d = make_ico_full(1, 0, 1, 32, 0, 22);
        assert!(!validate_ico(&d, 0));
    }

    #[test]
    fn ico_offset_too_small_rejected() {
        // data_offset < 6 + 16*1 = 22
        let d = make_ico_full(1, 0, 1, 32, 1024, 10);
        assert!(!validate_ico(&d, 0));
    }

    #[test]
    fn ico_short_buffer_count_only() {
        // Only 8 bytes — falls back to count-only check
        let mut d = vec![0u8; 8];
        d[0..4].copy_from_slice(&[0x00, 0x00, 0x01, 0x00]);
        d[4..6].copy_from_slice(&1u16.to_le_bytes());
        assert!(validate_ico(&d, 0));
    }

    #[test]
    fn ico_data_size_too_large_rejected() {
        let d = make_ico_full(1, 0, 1, 32, 0xFFFF_FF00, 22);
        assert!(!validate_ico(&d, 0));
    }

    #[test]
    fn ico_data_offset_too_large_rejected() {
        // data_offset beyond 1 MiB — impossible for a real ICO
        let d = make_ico_full(1, 0, 1, 32, 1024, 0x0048_09FF);
        assert!(!validate_ico(&d, 0));
    }

    #[test]
    fn ico_planes_and_bpp_both_zero_rejected() {
        let d = make_ico_full(1, 0, 0, 0, 1024, 22);
        assert!(!validate_ico(&d, 0));
    }

    #[test]
    fn ico_planes_zero_bpp_nonzero_accepted() {
        // planes=0, bpp=32 is valid (planes unspecified)
        let d = make_ico_full(1, 0, 0, 32, 1024, 22);
        assert!(validate_ico(&d, 0));
    }

    #[test]
    fn ico_mp4_false_positive_rejected() {
        // Real false positive from MP4 data: 00 00 01 00 15 00 00 00 0a 00 00 00 00 00 00 00 00 ff ff ff ff ff
        let d: Vec<u8> = vec![
            0x00, 0x00, 0x01, 0x00, 0x15, 0x00, 0x00, 0x00, 0x0a, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0xff,
        ];
        assert!(!validate_ico(&d, 0));
    }

    // ── ORF ───────────────────────────────────────────────────────────────────

    fn make_orf(ifd_offset: u32) -> Vec<u8> {
        let mut d = vec![0u8; 12];
        d[0..4].copy_from_slice(&[0x49, 0x49, 0x52, 0x4F]);
        d[4..8].copy_from_slice(&ifd_offset.to_le_bytes());
        d
    }

    #[test]
    fn orf_typical_ifd_offset_accepted() {
        assert!(validate_orf(&make_orf(8), 0));
    }

    #[test]
    fn orf_ifd_offset_zero_rejected() {
        assert!(!validate_orf(&make_orf(0), 0));
    }

    #[test]
    fn orf_ifd_offset_too_large_rejected() {
        assert!(!validate_orf(&make_orf(8192), 0));
    }

    // ── PEF ───────────────────────────────────────────────────────────────────

    fn make_pef(ifd_offset: u32, has_pentax: bool) -> Vec<u8> {
        let mut d = vec![0u8; 512];
        d[0..4].copy_from_slice(&[0x49, 0x49, 0x2A, 0x00]);
        d[4..8].copy_from_slice(&ifd_offset.to_le_bytes());
        if has_pentax {
            d[100..107].copy_from_slice(b"PENTAX ");
        }
        d
    }

    #[test]
    fn pef_with_pentax_string_accepted() {
        assert!(validate_pef(&make_pef(8, true), 0));
    }

    #[test]
    fn pef_without_pentax_string_rejected() {
        assert!(!validate_pef(&make_pef(8, false), 0));
    }

    #[test]
    fn pef_bad_ifd_offset_rejected() {
        assert!(!validate_pef(&make_pef(0, true), 0));
    }

    // ── Mach-O ────────────────────────────────────────────────────────────────

    fn make_macho(filetype: u32, ncmds: u32) -> Vec<u8> {
        let mut d = vec![0u8; 24];
        d[0..4].copy_from_slice(&[0xCF, 0xFA, 0xED, 0xFE]); // 64-bit LE magic
        d[4..8].copy_from_slice(&0x0100000Cu32.to_le_bytes()); // cputype arm64
        d[8..12].copy_from_slice(&0u32.to_le_bytes()); // cpusubtype
        d[12..16].copy_from_slice(&filetype.to_le_bytes());
        d[16..20].copy_from_slice(&ncmds.to_le_bytes());
        d
    }

    #[test]
    fn macho_executable_accepted() {
        assert!(validate_macho(&make_macho(2, 10), 0)); // filetype=2=MH_EXECUTE
    }

    #[test]
    fn macho_dylib_accepted() {
        assert!(validate_macho(&make_macho(6, 5), 0)); // filetype=6=MH_DYLIB
    }

    #[test]
    fn macho_filetype0_rejected() {
        assert!(!validate_macho(&make_macho(0, 10), 0));
    }

    #[test]
    fn macho_filetype_too_large_rejected() {
        assert!(!validate_macho(&make_macho(13, 10), 0));
    }

    #[test]
    fn macho_ncmds0_rejected() {
        assert!(!validate_macho(&make_macho(2, 0), 0));
    }

    // ── CR3 ───────────────────────────────────────────────────────────────────

    fn make_cr3(box_size: u32) -> Vec<u8> {
        let mut buf = vec![0u8; 12];
        buf[0..4].copy_from_slice(&box_size.to_be_bytes());
        buf[4..8].copy_from_slice(b"ftyp");
        buf[8..12].copy_from_slice(b"crx ");
        buf
    }

    #[test]
    fn cr3_valid_box_size_accepted() {
        assert!(validate_cr3(&make_cr3(24), 0));
    }

    #[test]
    fn cr3_box_size_too_small_rejected() {
        assert!(!validate_cr3(&make_cr3(4), 0));
    }

    #[test]
    fn cr3_box_size_too_large_rejected() {
        assert!(!validate_cr3(&make_cr3(1024), 0));
    }

    // ── SR2 ───────────────────────────────────────────────────────────────────

    fn make_sr2(entry_count: u16, has_sony: bool, has_sr2_tag: bool) -> Vec<u8> {
        // TIFF LE header: II + 0x2A + IFD offset 8
        let mut buf = vec![0u8; 10 + (entry_count as usize) * 12 + 20];
        buf[0..4].copy_from_slice(b"II\x2A\x00");
        buf[4..8].copy_from_slice(&8u32.to_le_bytes()); // IFD at offset 8
        buf[8..10].copy_from_slice(&entry_count.to_le_bytes());
        if has_sony && buf.len() > 20 {
            buf[12..16].copy_from_slice(b"SONY"); // put "SONY" in header area
        }
        if has_sr2_tag && entry_count > 0 {
            // Write tag 0x7200 as first IFD entry (LE: 0x00 0x72)
            buf[10] = 0x00;
            buf[11] = 0x72;
        }
        buf
    }

    #[test]
    fn sr2_valid_accepted() {
        assert!(validate_sr2(&make_sr2(8, true, true), 0));
    }

    #[test]
    fn sr2_no_sony_rejected() {
        assert!(!validate_sr2(&make_sr2(8, false, true), 0));
    }

    #[test]
    fn sr2_no_sr2_tag_rejected() {
        assert!(!validate_sr2(&make_sr2(8, true, false), 0));
    }

    // ── ARW rejects SR2 ───────────────────────────────────────────────────────

    #[test]
    fn arw_rejects_sr2_tag() {
        // ARW validator must reject data that has the SR2 private tag 0x7200.
        let data = make_sr2(8, true, true);
        assert!(!validate_arw(&data, 0));
    }

    // ── EPUB ──────────────────────────────────────────────────────────────────

    fn make_epub_lfh(fname: &[u8], content: &[u8]) -> Vec<u8> {
        let mut buf = vec![0u8; 30 + fname.len() + content.len()];
        buf[0..4].copy_from_slice(b"PK\x03\x04");
        buf[26..28].copy_from_slice(&(fname.len() as u16).to_le_bytes());
        buf[28..30].copy_from_slice(&0u16.to_le_bytes()); // no extra
        buf[30..30 + fname.len()].copy_from_slice(fname);
        buf[30 + fname.len()..].copy_from_slice(content);
        buf
    }

    #[test]
    fn epub_valid_accepted() {
        assert!(validate_epub(
            &make_epub_lfh(b"mimetype", b"application/epub+zip"),
            0
        ));
    }

    #[test]
    fn epub_wrong_filename_rejected() {
        assert!(!validate_epub(
            &make_epub_lfh(b"META-INF", b"application/epub+zip"),
            0
        ));
    }

    // ── ODT ───────────────────────────────────────────────────────────────────

    #[test]
    fn odt_valid_accepted() {
        assert!(validate_odt(
            &make_epub_lfh(b"mimetype", b"application/vnd.oasis.opendocument.text"),
            0
        ));
    }

    #[test]
    fn odt_ods_variant_accepted() {
        assert!(validate_odt(
            &make_epub_lfh(
                b"mimetype",
                b"application/vnd.oasis.opendocument.spreadsheet"
            ),
            0
        ));
    }

    #[test]
    fn odt_wrong_content_rejected() {
        assert!(!validate_odt(
            &make_epub_lfh(b"mimetype", b"application/epub+zip"),
            0
        ));
    }

    // ── MSG ───────────────────────────────────────────────────────────────────

    fn make_msg(has_byte_order: bool, has_mapi: bool) -> Vec<u8> {
        let mut buf = vec![0u8; 4096];
        buf[0..8].copy_from_slice(b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1");
        if has_byte_order {
            buf[28] = 0xFE;
            buf[29] = 0xFF;
        }
        if has_mapi {
            let marker = b"__substg1.0_";
            buf[512..512 + 12].copy_from_slice(marker);
        }
        buf
    }

    #[test]
    fn msg_valid_accepted() {
        assert!(validate_msg(&make_msg(true, true), 0));
    }

    #[test]
    fn msg_no_mapi_rejected() {
        assert!(!validate_msg(&make_msg(true, false), 0));
    }

    #[test]
    fn msg_wrong_byte_order_rejected() {
        assert!(!validate_msg(&make_msg(false, true), 0));
    }

    // ── WavPack ───────────────────────────────────────────────────────────────

    fn make_wavpack(ck_size: u32, version: u16) -> Vec<u8> {
        let mut buf = vec![0u8; 12];
        buf[0..4].copy_from_slice(b"wvpk");
        buf[4..8].copy_from_slice(&ck_size.to_le_bytes());
        buf[8..10].copy_from_slice(&version.to_le_bytes());
        buf
    }

    #[test]
    fn wavpack_valid_accepted() {
        assert!(validate_wavpack(&make_wavpack(100, 0x0407), 0));
    }

    #[test]
    fn wavpack_zero_size_rejected() {
        assert!(!validate_wavpack(&make_wavpack(0, 0x0407), 0));
    }

    #[test]
    fn wavpack_invalid_version_rejected() {
        assert!(!validate_wavpack(&make_wavpack(100, 0x0500), 0));
    }

    // ── CDR ───────────────────────────────────────────────────────────────────

    fn make_cdr(suffix: u8) -> Vec<u8> {
        let mut buf = vec![0u8; 12];
        buf[0..4].copy_from_slice(b"RIFF");
        buf[8..11].copy_from_slice(b"CDR");
        buf[11] = suffix;
        buf
    }

    #[test]
    fn cdr_version5_accepted() {
        assert!(validate_cdr(&make_cdr(b'5'), 0));
    }

    #[test]
    fn cdr_version_v_accepted() {
        assert!(validate_cdr(&make_cdr(b'V'), 0));
    }

    #[test]
    fn cdr_invalid_suffix_rejected() {
        assert!(!validate_cdr(&make_cdr(0x01), 0));
    }

    // ── SWF ───────────────────────────────────────────────────────────────────

    /// Build a minimal SWF header.  `byte8` is the first post-header byte:
    ///   FWS → RECT Nbits field (top 5 bits); CWS → zlib CMF; ZWS → LZMA.
    fn make_swf(compression: u8, version: u8, file_len: u32, byte8: u8) -> Vec<u8> {
        let mut buf = vec![0u8; 12];
        buf[0] = compression;
        buf[1] = b'W';
        buf[2] = b'S';
        buf[3] = version;
        buf[4..8].copy_from_slice(&file_len.to_le_bytes());
        buf[8] = byte8;
        buf
    }

    #[test]
    fn swf_fws_accepted() {
        // Nbits = 15 → byte 8 = 15 << 3 = 0x78
        assert!(validate_swf(&make_swf(b'F', 10, 200, 15 << 3), 0));
    }

    #[test]
    fn swf_cws_accepted() {
        // zlib CMF = 0x78 (CM=8, CINFO=7)
        assert!(validate_swf(&make_swf(b'C', 8, 150, 0x78), 0));
    }

    #[test]
    fn swf_zws_accepted() {
        assert!(validate_swf(&make_swf(b'Z', 13, 80, 0x00), 0));
    }

    #[test]
    fn swf_version_too_low_rejected() {
        assert!(!validate_swf(&make_swf(b'F', 2, 200, 15 << 3), 0));
    }

    #[test]
    fn swf_version_too_high_rejected() {
        assert!(!validate_swf(&make_swf(b'F', 60, 200, 15 << 3), 0));
    }

    #[test]
    fn swf_file_too_short_rejected() {
        assert!(!validate_swf(&make_swf(b'F', 10, 4, 15 << 3), 0));
    }

    #[test]
    fn swf_file_too_large_rejected() {
        // Random u32 LE > 100 MiB — the #1 false-positive killer.
        assert!(!validate_swf(&make_swf(b'F', 10, 200_000_000, 15 << 3), 0));
    }

    #[test]
    fn swf_fws_nbits_zero_rejected() {
        assert!(!validate_swf(&make_swf(b'F', 10, 200, 0x00), 0)); // Nbits = 0
    }

    #[test]
    fn swf_fws_nbits_too_high_rejected() {
        assert!(!validate_swf(&make_swf(b'F', 10, 200, 31 << 3), 0)); // Nbits = 31
    }

    #[test]
    fn swf_cws_bad_zlib_rejected() {
        // CMF with CM != 8 → rejected.
        assert!(!validate_swf(&make_swf(b'C', 10, 200, 0x00), 0));
    }

    // ── DCR ───────────────────────────────────────────────────────────────────

    fn make_dcr(make_str: &[u8]) -> Vec<u8> {
        let mut buf = vec![0u8; 512];
        buf[0..4].copy_from_slice(b"II\x2A\x00");
        buf[4..8].copy_from_slice(&8u32.to_le_bytes());
        // embed make string at offset 20
        let end = 20 + make_str.len().min(buf.len() - 20);
        buf[20..end].copy_from_slice(&make_str[..end - 20]);
        buf
    }

    #[test]
    fn dcr_kodak_accepted() {
        assert!(validate_dcr(&make_dcr(b"Kodak"), 0));
    }

    #[test]
    fn dcr_kodak_upper_accepted() {
        assert!(validate_dcr(&make_dcr(b"KODAK"), 0));
    }

    #[test]
    fn dcr_no_kodak_rejected() {
        assert!(!validate_dcr(&make_dcr(b"Canon"), 0));
    }

    #[test]
    fn dcr_nikon_rejected() {
        assert!(!validate_dcr(&make_dcr(b"NIKON"), 0));
    }

    #[test]
    fn dcr_sony_rejected() {
        assert!(!validate_dcr(&make_dcr(b"SONY"), 0));
    }

    // ── CRW ───────────────────────────────────────────────────────────────────

    fn make_crw() -> Vec<u8> {
        let mut buf = vec![0u8; 14];
        buf[0..6].copy_from_slice(b"II\x1A\x00\x00\x00");
        buf[6..14].copy_from_slice(b"HEAPCCDR");
        buf
    }

    #[test]
    fn crw_accepted() {
        assert!(validate_crw(&make_crw(), 0));
    }

    #[test]
    fn crw_wrong_tag_rejected() {
        let mut buf = make_crw();
        buf[6..14].copy_from_slice(b"NOTCRWXX");
        assert!(!validate_crw(&buf, 0));
    }

    // ── MRW ───────────────────────────────────────────────────────────────────

    fn make_mrw(tag: &[u8; 3]) -> Vec<u8> {
        let mut buf = vec![0u8; 16];
        buf[0..4].copy_from_slice(b"\x00MRM");
        buf[4..8].copy_from_slice(&8u32.to_be_bytes()); // length field
        buf[8..11].copy_from_slice(tag);
        buf
    }

    #[test]
    fn mrw_prd_accepted() {
        assert!(validate_mrw(&make_mrw(b"PRD"), 0));
    }

    #[test]
    fn mrw_ttw_accepted() {
        assert!(validate_mrw(&make_mrw(b"TTW"), 0));
    }

    #[test]
    fn mrw_wbg_accepted() {
        assert!(validate_mrw(&make_mrw(b"WBG"), 0));
    }

    #[test]
    fn mrw_bad_tag_rejected() {
        assert!(!validate_mrw(&make_mrw(b"XYZ"), 0));
    }

    // ── KDBX / KDB ───────────────────────────────────────────────────────────

    fn make_keepass(sig2: &[u8; 4], major: u16) -> Vec<u8> {
        let mut buf = vec![0u8; 12];
        buf[0..4].copy_from_slice(b"\x03\xD9\xA2\x9A");
        buf[4..8].copy_from_slice(sig2);
        buf[8..10].copy_from_slice(&0u16.to_le_bytes()); // minor
        buf[10..12].copy_from_slice(&major.to_le_bytes());
        buf
    }

    #[test]
    fn kdbx_major3_accepted() {
        assert!(validate_kdbx(&make_keepass(b"\x67\xFB\x4B\xB5", 3), 0));
    }

    #[test]
    fn kdbx_major4_accepted() {
        assert!(validate_kdbx(&make_keepass(b"\x67\xFB\x4B\xB5", 4), 0));
    }

    #[test]
    fn kdbx_major1_rejected() {
        assert!(!validate_kdbx(&make_keepass(b"\x67\xFB\x4B\xB5", 1), 0));
    }

    #[test]
    fn kdb_major1_accepted() {
        assert!(validate_kdb(&make_keepass(b"\x65\xFB\x4B\xB5", 1), 0));
    }

    #[test]
    fn kdb_major2_accepted() {
        assert!(validate_kdb(&make_keepass(b"\x65\xFB\x4B\xB5", 2), 0));
    }

    #[test]
    fn kdb_major4_rejected() {
        assert!(!validate_kdb(&make_keepass(b"\x65\xFB\x4B\xB5", 4), 0));
    }

    // ── E01 ───────────────────────────────────────────────────────────────────

    fn make_e01(segment: u16) -> Vec<u8> {
        let mut buf = vec![0u8; 10];
        buf[0..8].copy_from_slice(b"EVF\x09\x0D\x0A\xFF\x00");
        buf[8..10].copy_from_slice(&segment.to_le_bytes());
        buf
    }

    #[test]
    fn e01_segment1_accepted() {
        assert!(validate_e01(&make_e01(1), 0));
    }

    #[test]
    fn e01_segment2_rejected() {
        assert!(!validate_e01(&make_e01(2), 0));
    }

    #[test]
    fn e01_segment0_rejected() {
        assert!(!validate_e01(&make_e01(0), 0));
    }

    // ── PCAP ──────────────────────────────────────────────────────────────────

    fn make_pcap_le() -> Vec<u8> {
        let mut buf = vec![0u8; 8];
        buf[0..4].copy_from_slice(&[0xD4, 0xC3, 0xB2, 0xA1]);
        buf[4..6].copy_from_slice(&2u16.to_le_bytes()); // major
        buf[6..8].copy_from_slice(&4u16.to_le_bytes()); // minor
        buf
    }

    fn make_pcap_be() -> Vec<u8> {
        let mut buf = vec![0u8; 8];
        buf[0..4].copy_from_slice(&[0xA1, 0xB2, 0xC3, 0xD4]);
        buf[4..6].copy_from_slice(&2u16.to_be_bytes()); // major
        buf[6..8].copy_from_slice(&4u16.to_be_bytes()); // minor
        buf
    }

    #[test]
    fn pcap_le_accepted() {
        assert!(validate_pcap(&make_pcap_le(), 0));
    }

    #[test]
    fn pcap_be_accepted() {
        assert!(validate_pcap(&make_pcap_be(), 0));
    }

    #[test]
    fn pcap_le_wrong_version_rejected() {
        let mut buf = make_pcap_le();
        buf[4..6].copy_from_slice(&1u16.to_le_bytes()); // wrong major
        assert!(!validate_pcap(&buf, 0));
    }

    // ── DMP ───────────────────────────────────────────────────────────────────

    fn make_dmp(stream_count: u32) -> Vec<u8> {
        let mut buf = vec![0u8; 12];
        buf[0..6].copy_from_slice(b"MDMP\x93\xA7");
        buf[8..12].copy_from_slice(&stream_count.to_le_bytes());
        buf
    }

    #[test]
    fn dmp_streams_accepted() {
        assert!(validate_dmp(&make_dmp(5), 0));
    }

    #[test]
    fn dmp_zero_streams_rejected() {
        assert!(!validate_dmp(&make_dmp(0), 0));
    }

    // ── PLIST ─────────────────────────────────────────────────────────────────

    #[test]
    fn plist_sufficient_length_accepted() {
        let buf = vec![0u8; 64];
        assert!(validate_plist(&buf, 0));
    }

    #[test]
    fn plist_too_short_rejected() {
        let buf = vec![0u8; 10];
        assert!(!validate_plist(&buf, 0));
    }

    // ── TS ────────────────────────────────────────────────────────────────────

    /// Build a TS buffer with 5 valid sync bytes (enough for the extended check).
    fn make_ts() -> Vec<u8> {
        let mut buf = vec![0u8; 800];
        buf[0] = 0x47;
        buf[188] = 0x47;
        buf[376] = 0x47;
        buf[564] = 0x47;
        buf[752] = 0x47;
        buf
    }

    #[test]
    fn ts_stride_accepted() {
        assert!(validate_ts(&make_ts(), 0));
    }

    #[test]
    fn ts_missing_second_sync_rejected() {
        let mut buf = make_ts();
        buf[188] = 0x00;
        assert!(!validate_ts(&buf, 0));
    }

    #[test]
    fn ts_missing_third_sync_rejected() {
        let mut buf = make_ts();
        buf[376] = 0x00;
        assert!(!validate_ts(&buf, 0));
    }

    #[test]
    fn ts_missing_fourth_sync_rejected() {
        let mut buf = make_ts();
        buf[564] = 0x00;
        assert!(!validate_ts(&buf, 0));
    }

    #[test]
    fn ts_missing_fifth_sync_rejected() {
        let mut buf = make_ts();
        buf[752] = 0x00;
        assert!(!validate_ts(&buf, 0));
    }

    #[test]
    fn ts_short_buffer_skips_extended_check() {
        // Only 3 sync bytes in a 400-byte buffer — extended check is skipped.
        let mut buf = vec![0u8; 400];
        buf[0] = 0x47;
        buf[188] = 0x47;
        buf[376] = 0x47;
        assert!(validate_ts(&buf, 0));
    }

    // ── M2TS ──────────────────────────────────────────────────────────────────

    /// Build an M2TS buffer with 5 valid sync bytes.
    fn make_m2ts() -> Vec<u8> {
        let mut buf = vec![0u8; 800];
        buf[4] = 0x47;
        buf[196] = 0x47;
        buf[388] = 0x47;
        buf[580] = 0x47;
        buf[772] = 0x47;
        buf
    }

    #[test]
    fn m2ts_stride_accepted() {
        assert!(validate_m2ts(&make_m2ts(), 0));
    }

    #[test]
    fn m2ts_missing_second_sync_rejected() {
        let mut buf = make_m2ts();
        buf[196] = 0x00;
        assert!(!validate_m2ts(&buf, 0));
    }

    #[test]
    fn m2ts_missing_fifth_sync_rejected() {
        let mut buf = make_m2ts();
        buf[772] = 0x00;
        assert!(!validate_m2ts(&buf, 0));
    }

    #[test]
    fn m2ts_short_buffer_skips_extended_check() {
        // Only 3 sync bytes in a 400-byte buffer — extended check is skipped.
        let mut buf = vec![0u8; 400];
        buf[4] = 0x47;
        buf[196] = 0x47;
        buf[388] = 0x47;
        assert!(validate_m2ts(&buf, 0));
    }

    // ── LUKS ──────────────────────────────────────────────────────────────────

    fn make_luks(version: u16) -> Vec<u8> {
        let mut buf = vec![0u8; 8];
        buf[0..6].copy_from_slice(b"LUKS\xBA\xBE");
        buf[6..8].copy_from_slice(&version.to_be_bytes());
        buf
    }

    #[test]
    fn luks_v1_accepted() {
        assert!(validate_luks(&make_luks(1), 0));
    }

    #[test]
    fn luks_v2_accepted() {
        assert!(validate_luks(&make_luks(2), 0));
    }

    #[test]
    fn luks_v3_rejected() {
        assert!(!validate_luks(&make_luks(3), 0));
    }

    // ── X3F ───────────────────────────────────────────────────────────────────

    fn make_x3f(major: u8, minor: u8) -> Vec<u8> {
        let mut buf = vec![0u8; 8];
        buf[0..4].copy_from_slice(b"FOVb");
        buf[4] = minor; // u32 LE: byte 4 = minor
        buf[5] = major; // byte 5 = major
        buf
    }

    #[test]
    fn x3f_v2_accepted() {
        assert!(validate_x3f(&make_x3f(2, 0), 0));
    }

    #[test]
    fn x3f_v3_accepted() {
        assert!(validate_x3f(&make_x3f(3, 1), 0));
    }

    #[test]
    fn x3f_v1_rejected() {
        assert!(!validate_x3f(&make_x3f(1, 0), 0));
    }

    #[test]
    fn x3f_v4_rejected() {
        assert!(!validate_x3f(&make_x3f(4, 0), 0));
    }

    // ── ISO ───────────────────────────────────────────────────────────────────

    /// Build a full PVD buffer with descriptor type, magic, version, and
    /// Volume Space Size BBO fields so all three validation checks can fire.
    ///
    /// `pos` is the position of the `CD001` magic within the returned buffer.
    /// We set pos=1 so that buf[0] holds the descriptor type byte (0x01).
    /// The VSS BBO fields live at buf[pos+79] (LE) and buf[pos+83] (BE).
    fn make_iso_pvd_full(vss: u32) -> (Vec<u8>, usize) {
        // Need at least pos + 87 bytes; pos = 1, so 88 bytes total.
        let pos = 1usize;
        let mut buf = vec![0u8; pos + 88];
        buf[pos - 1] = 0x01; // Descriptor Type: Primary Volume Descriptor
        buf[pos..pos + 5].copy_from_slice(b"CD001");
        buf[pos + 5] = 0x01; // Version
                             // Volume Space Size (Both Byte Order) at PVD+80 = buf[pos+79].
        buf[pos + 79..pos + 83].copy_from_slice(&vss.to_le_bytes());
        buf[pos + 83..pos + 87].copy_from_slice(&vss.to_be_bytes());
        (buf, pos)
    }

    /// Minimal buffer — only has magic + version, not enough for BBO check.
    fn make_iso_pvd_short() -> Vec<u8> {
        let mut buf = vec![0u8; 8];
        buf[0..5].copy_from_slice(b"CD001");
        buf[5] = 0x01; // version
        buf
    }

    #[test]
    fn iso_pvd_short_accepted() {
        // Insufficient data for BBO check → accepted (can't reject without data).
        assert!(validate_iso(&make_iso_pvd_short(), 0));
    }

    #[test]
    fn iso_pvd_full_accepted() {
        let (buf, pos) = make_iso_pvd_full(700_000);
        assert!(validate_iso(&buf, pos));
    }

    #[test]
    fn iso_bad_version_rejected() {
        let (mut buf, pos) = make_iso_pvd_full(700_000);
        buf[pos + 5] = 0x02; // corrupt version
        assert!(!validate_iso(&buf, pos));
    }

    #[test]
    fn iso_bad_descriptor_type_rejected() {
        let (mut buf, pos) = make_iso_pvd_full(700_000);
        buf[pos - 1] = 0x02; // Supplementary VD, not Primary
        assert!(!validate_iso(&buf, pos));
    }

    #[test]
    fn iso_bbo_mismatch_rejected() {
        let (mut buf, pos) = make_iso_pvd_full(700_000);
        // Corrupt the BE copy so LE != BE.
        buf[pos + 83] ^= 0xFF;
        assert!(!validate_iso(&buf, pos));
    }

    #[test]
    fn iso_zero_vss_rejected() {
        // VSS = 0 is invalid for a real ISO.
        let (buf, pos) = make_iso_pvd_full(0);
        assert!(!validate_iso(&buf, pos));
    }

    // ── DICOM ─────────────────────────────────────────────────────────────────

    #[test]
    fn dicom_sufficient_data_accepted() {
        // pos points to "DICM"; bytes at pos+4..+6 = group 0x0002, pos+8..+10 = VR "UL"
        let mut buf = vec![0u8; 16];
        buf[4..6].copy_from_slice(&0x0002u16.to_le_bytes()); // group
        buf[8] = b'U'; // VR byte 0
        buf[9] = b'L'; // VR byte 1
        assert!(validate_dicom(&buf, 0));
    }

    #[test]
    fn dicom_too_short_soft_passes() {
        // With insufficient data the validator soft-passes (returns true) —
        // the scan chunk boundary may have cut the header short.
        let buf = vec![0u8; 5];
        assert!(validate_dicom(&buf, 0));
    }

    #[test]
    fn dicom_wrong_group_rejected() {
        let mut buf = vec![0u8; 16];
        buf[4..6].copy_from_slice(&0x0008u16.to_le_bytes()); // wrong group
        buf[8] = b'U';
        buf[9] = b'L';
        assert!(!validate_dicom(&buf, 0));
    }

    #[test]
    fn dicom_invalid_vr_rejected() {
        let mut buf = vec![0u8; 16];
        buf[4..6].copy_from_slice(&0x0002u16.to_le_bytes());
        buf[8] = 0x01; // non-ASCII VR
        buf[9] = 0x02;
        assert!(!validate_dicom(&buf, 0));
    }

    // ── TAR ───────────────────────────────────────────────────────────────────

    fn make_tar(version: &[u8; 2]) -> Vec<u8> {
        let mut buf = vec![0u8; 10];
        buf[0..6].copy_from_slice(b"ustar\x00");
        buf[6..8].copy_from_slice(version);
        buf
    }

    #[test]
    fn tar_posix_version_accepted() {
        assert!(validate_tar(&make_tar(b"00"), 0));
    }

    #[test]
    fn tar_gnu_version_accepted() {
        assert!(validate_tar(&make_tar(b"  "), 0));
    }

    #[test]
    fn tar_bad_version_rejected() {
        assert!(!validate_tar(&make_tar(b"01"), 0));
    }

    // ── APE ───────────────────────────────────────────────────────────────────

    fn make_ape(version: u16) -> Vec<u8> {
        let mut buf = vec![0u8; 8];
        buf[0..4].copy_from_slice(b"MAC ");
        buf[6..8].copy_from_slice(&version.to_le_bytes());
        buf
    }

    #[test]
    fn ape_version_3990_accepted() {
        assert!(validate_ape(&make_ape(3990), 0));
    }

    #[test]
    fn ape_version_4000_accepted() {
        assert!(validate_ape(&make_ape(4000), 0));
    }

    #[test]
    fn ape_version_too_old_rejected() {
        assert!(!validate_ape(&make_ape(3000), 0));
    }

    #[test]
    fn ape_version_too_new_rejected() {
        assert!(!validate_ape(&make_ape(5000), 0));
    }

    // ── AU ────────────────────────────────────────────────────────────────────

    fn make_au(data_offset: u32, encoding: u32) -> Vec<u8> {
        let mut buf = vec![0u8; 16];
        buf[0..4].copy_from_slice(b".snd");
        buf[4..8].copy_from_slice(&data_offset.to_be_bytes());
        buf[12..16].copy_from_slice(&encoding.to_be_bytes());
        buf
    }

    #[test]
    fn au_mulaw_accepted() {
        assert!(validate_au(&make_au(24, 1), 0));
    }

    #[test]
    fn au_pcm16_accepted() {
        assert!(validate_au(&make_au(24, 3), 0));
    }

    #[test]
    fn au_small_offset_rejected() {
        assert!(!validate_au(&make_au(8, 1), 0));
    }

    #[test]
    fn au_unknown_encoding_rejected() {
        assert!(!validate_au(&make_au(24, 99), 0));
    }

    // ── TTF ───────────────────────────────────────────────────────────────────

    fn make_ttf(num_tables: u16) -> Vec<u8> {
        let mut buf = vec![0u8; 12];
        buf[0..4].copy_from_slice(&[0x00, 0x01, 0x00, 0x00]);
        buf[4..6].copy_from_slice(&num_tables.to_be_bytes());
        // Compute correct searchRange, entrySelector, rangeShift.
        let entry_selector = if num_tables > 0 {
            (num_tables as f64).log2().floor() as u16
        } else {
            0
        };
        let search_range = (1u16 << entry_selector) * 16;
        let range_shift = num_tables * 16 - search_range;
        buf[6..8].copy_from_slice(&search_range.to_be_bytes());
        buf[8..10].copy_from_slice(&entry_selector.to_be_bytes());
        buf[10..12].copy_from_slice(&range_shift.to_be_bytes());
        buf
    }

    #[test]
    fn ttf_16_tables_accepted() {
        assert!(validate_ttf(&make_ttf(16), 0));
    }

    #[test]
    fn ttf_too_few_tables_rejected() {
        assert!(!validate_ttf(&make_ttf(2), 0));
    }

    #[test]
    fn ttf_too_many_tables_rejected() {
        assert!(!validate_ttf(&make_ttf(100), 0));
    }

    // ── WOFF ──────────────────────────────────────────────────────────────────

    fn make_woff(flavor: u32, length: u32, num_tables: u16) -> Vec<u8> {
        let mut buf = vec![0u8; 14];
        buf[0..4].copy_from_slice(b"wOFF");
        buf[4..8].copy_from_slice(&flavor.to_be_bytes());
        buf[8..12].copy_from_slice(&length.to_be_bytes());
        buf[12..14].copy_from_slice(&num_tables.to_be_bytes());
        buf
    }

    #[test]
    fn woff_ttf_flavor_accepted() {
        assert!(validate_woff(&make_woff(0x0001_0000, 1024, 10), 0));
    }

    #[test]
    fn woff_otf_flavor_accepted() {
        assert!(validate_woff(&make_woff(0x4F54_544F, 512, 8), 0));
    }

    #[test]
    fn woff_bad_flavor_rejected() {
        assert!(!validate_woff(&make_woff(0xDEAD_BEEF, 1024, 10), 0));
    }

    #[test]
    fn woff_length_too_small_rejected() {
        assert!(!validate_woff(&make_woff(0x0001_0000, 10, 10), 0));
    }

    // ── CHM ───────────────────────────────────────────────────────────────────

    #[test]
    fn chm_full_magic_accepted() {
        let buf = vec![0u8; 16];
        assert!(validate_chm(&buf, 0));
    }

    #[test]
    fn chm_too_short_rejected() {
        let buf = vec![0u8; 5];
        assert!(!validate_chm(&buf, 0));
    }

    // ── BLEND ─────────────────────────────────────────────────────────────────

    fn make_blend(ptr_size: u8, endian: u8) -> Vec<u8> {
        let mut buf = vec![0u8; 12];
        buf[0..7].copy_from_slice(b"BLENDER");
        buf[7] = ptr_size;
        buf[8] = endian;
        buf
    }

    #[test]
    fn blend_32bit_le_accepted() {
        assert!(validate_blend(&make_blend(b'-', b'v'), 0));
    }

    #[test]
    fn blend_64bit_be_accepted() {
        assert!(validate_blend(&make_blend(b'_', b'V'), 0));
    }

    #[test]
    fn blend_bad_ptr_rejected() {
        assert!(!validate_blend(&make_blend(b'X', b'v'), 0));
    }

    #[test]
    fn blend_bad_endian_rejected() {
        assert!(!validate_blend(&make_blend(b'-', b'X'), 0));
    }

    // ── INDD / WTV ────────────────────────────────────────────────────────────

    #[test]
    fn indd_sufficient_data_accepted() {
        assert!(validate_indd(&[0u8; 16], 0));
    }

    #[test]
    fn indd_too_short_rejected() {
        assert!(!validate_indd(&[0u8; 10], 0));
    }

    #[test]
    fn wtv_sufficient_data_accepted() {
        assert!(validate_wtv(&[0u8; 16], 0));
    }

    #[test]
    fn wtv_too_short_rejected() {
        assert!(!validate_wtv(&[0u8; 10], 0));
    }

    // ── PHP ───────────────────────────────────────────────────────────────────

    fn make_php(byte5: u8) -> Vec<u8> {
        let mut buf = vec![0u8; 16];
        buf[0..5].copy_from_slice(b"<?php");
        buf[5] = byte5;
        buf
    }

    #[test]
    fn php_space_after_tag_accepted() {
        assert!(validate_php(&make_php(b' '), 0));
    }

    #[test]
    fn php_newline_after_tag_accepted() {
        assert!(validate_php(&make_php(b'\n'), 0));
    }

    #[test]
    fn php_cr_after_tag_accepted() {
        assert!(validate_php(&make_php(b'\r'), 0));
    }

    #[test]
    fn php_tab_after_tag_accepted() {
        assert!(validate_php(&make_php(b'\t'), 0));
    }

    #[test]
    fn php_no_space_rejected() {
        assert!(!validate_php(&make_php(b'i'), 0)); // "<?phpinfo" pattern
    }

    #[test]
    fn php_too_short_passes() {
        // Only 5 bytes — benefit of doubt.
        assert!(validate_php(b"<?php", 0));
    }

    // ── Shebang ───────────────────────────────────────────────────────────────

    fn make_shebang_path(path: &[u8]) -> Vec<u8> {
        let mut buf = Vec::with_capacity(2 + path.len());
        buf.extend_from_slice(b"#!");
        buf.extend_from_slice(path);
        // Pad to at least 7 bytes
        while buf.len() < 7 {
            buf.push(b' ');
        }
        buf
    }

    #[test]
    fn shebang_bin_sh_accepted() {
        assert!(validate_shebang(&make_shebang_path(b"/bin/sh"), 0));
    }

    #[test]
    fn shebang_usr_bin_env_accepted() {
        assert!(validate_shebang(
            &make_shebang_path(b"/usr/bin/env python3"),
            0
        ));
    }

    #[test]
    fn shebang_usr_bin_perl_accepted() {
        assert!(validate_shebang(&make_shebang_path(b"/usr/bin/perl"), 0));
    }

    #[test]
    fn shebang_no_slash_rejected() {
        assert!(!validate_shebang(&make_shebang_path(b"env python"), 0));
    }

    #[test]
    fn shebang_slash_then_binary_rejected() {
        // "#!/" + 0x1e 0xb0 0x3e 0x67 — real false positive from binary data
        let data = b"#!/\x1e\xb0\x3e\x67".to_vec();
        assert!(!validate_shebang(&data, 0));
    }

    #[test]
    fn shebang_slash_unknown_prefix_rejected() {
        // "#!/etc/..." — not a valid interpreter path
        assert!(!validate_shebang(&make_shebang_path(b"/etc/passwd"), 0));
    }

    #[test]
    fn shebang_too_short_passes() {
        // Only 2 bytes — benefit of doubt.
        assert!(validate_shebang(b"#!", 0));
    }

    // ── AAC ───────────────────────────────────────────────────────────────────

    fn make_aac(id: u8, sfi: u8) -> Vec<u8> {
        make_aac_with_len(id, sfi, 256)
    }

    /// Build a 7-byte ADTS header with an explicit frame length encoded in
    /// the 13-bit field at bytes 3–5.
    fn make_aac_with_len(id: u8, sfi: u8, frame_len: usize) -> Vec<u8> {
        // Byte 1: sync_lo(4) + ID(1) + layer(00) + protection_absent(1)
        let b1 = 0xF0 | (id << 3) | 0x01;
        // Byte 2: profile(00) + sampling_freq_idx(4 bits) + private(0) + channel_hi(0)
        let b2 = (sfi & 0x0F) << 2;
        // 13-bit frame_length field: byte3[1:0] << 11 | byte4 << 3 | byte5[7:5]
        let b3_lo = ((frame_len >> 11) & 0x03) as u8;
        let b4 = ((frame_len >> 3) & 0xFF) as u8;
        let b5_hi = (((frame_len & 0x07) as u8) << 5) | 0x1F; // VBR fullness
        vec![0xFF, b1, b2, b3_lo, b4, b5_hi, 0xFC]
    }

    #[test]
    fn aac_mpeg4_valid() {
        assert!(validate_aac(&make_aac(0, 3), 0)); // MPEG-4, 48000 Hz (sfi=3)
    }

    #[test]
    fn aac_mpeg2_valid() {
        assert!(validate_aac(&make_aac(1, 4), 0)); // MPEG-2, 44100 Hz (sfi=4)
    }

    #[test]
    fn aac_invalid_layer_rejected() {
        // layer bits set to 01 (Layer I) — invalid for ADTS
        let mut buf = make_aac(0, 4);
        buf[1] |= 0x02;
        assert!(!validate_aac(&buf, 0));
    }

    #[test]
    fn aac_reserved_sfi_rejected() {
        // sfi = 13 (reserved) — invalid
        assert!(!validate_aac(&make_aac(0, 13), 0));
    }

    #[test]
    fn aac_too_short_passes() {
        // Only 2 bytes — benefit of doubt (frame-length field not yet visible).
        assert!(validate_aac(b"\xFF\xF1", 0));
    }

    #[test]
    fn aac_frame_length_zero_rejected() {
        // frame_len = 0 in bytes 3-5 — impossible for any real ADTS frame.
        let mut buf = make_aac(0, 4);
        // Zero out bytes 3-5 so frame_len encodes as 0.
        buf[3] &= !0x03;
        buf[4] = 0x00;
        buf[5] = 0x00;
        assert!(!validate_aac(&buf, 0));
    }

    #[test]
    fn aac_frame_length_too_small_rejected() {
        // frame_len = 6 (< 7 minimum header size) — impossible ADTS frame.
        let buf = make_aac_with_len(0, 4, 6);
        assert!(!validate_aac(&buf, 0));
    }

    #[test]
    fn aac_frame_length_minimum_valid() {
        // frame_len = 7 (smallest allowed: header-only frame).
        let buf = make_aac_with_len(0, 4, 7);
        assert!(validate_aac(&buf, 0));
    }

    #[test]
    fn aac_frame_length_typical_accepted() {
        // frame_len = 512 — a common real-world value.
        let buf = make_aac_with_len(0, 4, 512);
        assert!(validate_aac(&buf, 0));
    }

    #[test]
    fn aac_frame_length_max_boundary_accepted() {
        // frame_len = 8191 — maximum 13-bit value per ADTS spec.
        let buf = make_aac_with_len(0, 4, 8191);
        assert!(validate_aac(&buf, 0));
    }

    // ── DjVu ──────────────────────────────────────────────────────────────────

    fn make_djvu(form_type: &[u8; 4]) -> Vec<u8> {
        let mut buf = vec![0u8; 16];
        buf[..8].copy_from_slice(b"AT&TFORM");
        // u32 BE size at bytes 8–11
        buf[8..12].copy_from_slice(&8u32.to_be_bytes());
        buf[12..16].copy_from_slice(form_type);
        buf
    }

    #[test]
    fn djvu_single_page_accepted() {
        assert!(validate_djvu(&make_djvu(b"DJVU"), 0));
    }

    #[test]
    fn djvu_multi_page_accepted() {
        assert!(validate_djvu(&make_djvu(b"DJVM"), 0));
    }

    #[test]
    fn djvu_include_accepted() {
        assert!(validate_djvu(&make_djvu(b"DJVI"), 0));
    }

    #[test]
    fn djvu_thumbnail_accepted() {
        assert!(validate_djvu(&make_djvu(b"THUM"), 0));
    }

    #[test]
    fn djvu_unknown_form_type_rejected() {
        assert!(!validate_djvu(&make_djvu(b"WAVE"), 0));
    }

    #[test]
    fn djvu_too_short_passes() {
        assert!(validate_djvu(b"AT&TFORM", 0));
    }

    // ── XCF ───────────────────────────────────────────────────────────────────

    fn make_xcf(version: &[u8]) -> Vec<u8> {
        let mut buf = Vec::with_capacity(15);
        buf.extend_from_slice(b"gimp xcf v");
        buf.extend_from_slice(version);
        while buf.len() < 15 {
            buf.push(0);
        }
        buf
    }

    #[test]
    fn xcf_legacy_version_accepted() {
        assert!(validate_xcf(&make_xcf(b"file\0"), 0));
    }

    #[test]
    fn xcf_modern_version_accepted() {
        assert!(validate_xcf(&make_xcf(b"003\0!"), 0));
    }

    #[test]
    fn xcf_invalid_version_rejected() {
        assert!(!validate_xcf(&make_xcf(b"xyz\0!"), 0));
    }

    #[test]
    fn xcf_too_short_passes() {
        assert!(validate_xcf(b"gimp xcf v", 0));
    }

    // ── PCX ───────────────────────────────────────────────────────────────────

    struct PcxParams {
        version: u8,
        encoding: u8,
        bpp: u8,
        x_min: u16,
        y_min: u16,
        x_max: u16,
        y_max: u16,
        reserved: u8,
        planes: u8,
        bpl: u16,
    }

    impl Default for PcxParams {
        fn default() -> Self {
            Self {
                version: 5,
                encoding: 1,
                bpp: 8,
                x_min: 0,
                y_min: 0,
                x_max: 639,
                y_max: 479,
                reserved: 0,
                planes: 3,
                bpl: 640,
            }
        }
    }

    fn make_pcx(p: PcxParams) -> Vec<u8> {
        let mut buf = vec![0u8; 128];
        buf[0] = 0x0A;
        buf[1] = p.version;
        buf[2] = p.encoding;
        buf[3] = p.bpp;
        buf[4..6].copy_from_slice(&p.x_min.to_le_bytes());
        buf[6..8].copy_from_slice(&p.y_min.to_le_bytes());
        buf[8..10].copy_from_slice(&p.x_max.to_le_bytes());
        buf[10..12].copy_from_slice(&p.y_max.to_le_bytes());
        buf[64] = p.reserved;
        buf[65] = p.planes;
        buf[66..68].copy_from_slice(&p.bpl.to_le_bytes());
        buf
    }

    #[test]
    fn pcx_valid_v5_accepted() {
        assert!(validate_pcx(&make_pcx(PcxParams::default()), 0));
    }

    #[test]
    fn pcx_valid_v3_mono_accepted() {
        assert!(validate_pcx(
            &make_pcx(PcxParams {
                version: 3,
                bpp: 1,
                x_max: 319,
                y_max: 199,
                planes: 1,
                bpl: 40,
                ..PcxParams::default()
            }),
            0
        ));
    }

    #[test]
    fn pcx_invalid_version_rejected() {
        assert!(!validate_pcx(
            &make_pcx(PcxParams {
                version: 1,
                ..PcxParams::default()
            }),
            0
        ));
    }

    #[test]
    fn pcx_invalid_encoding_rejected() {
        assert!(!validate_pcx(
            &make_pcx(PcxParams {
                encoding: 2,
                ..PcxParams::default()
            }),
            0
        ));
    }

    #[test]
    fn pcx_invalid_bpp_rejected() {
        assert!(!validate_pcx(
            &make_pcx(PcxParams {
                bpp: 16,
                ..PcxParams::default()
            }),
            0
        ));
    }

    #[test]
    fn pcx_xmax_less_than_xmin_rejected() {
        assert!(!validate_pcx(
            &make_pcx(PcxParams {
                x_min: 100,
                x_max: 50,
                ..PcxParams::default()
            }),
            0
        ));
    }

    #[test]
    fn pcx_ymax_less_than_ymin_rejected() {
        assert!(!validate_pcx(
            &make_pcx(PcxParams {
                y_min: 100,
                y_max: 50,
                ..PcxParams::default()
            }),
            0
        ));
    }

    #[test]
    fn pcx_nonzero_reserved_rejected() {
        assert!(!validate_pcx(
            &make_pcx(PcxParams {
                reserved: 1,
                ..PcxParams::default()
            }),
            0
        ));
    }

    #[test]
    fn pcx_invalid_planes_rejected() {
        assert!(!validate_pcx(
            &make_pcx(PcxParams {
                planes: 5,
                ..PcxParams::default()
            }),
            0
        ));
    }

    #[test]
    fn pcx_zero_bpl_rejected() {
        assert!(!validate_pcx(
            &make_pcx(PcxParams {
                bpl: 0,
                ..PcxParams::default()
            }),
            0
        ));
    }

    #[test]
    fn pcx_too_short_passes() {
        assert!(validate_pcx(b"\x0a\x05\x01\x08", 0));
    }

    // ── JAR ───────────────────────────────────────────────────────────────────

    fn make_jar_lfh(fname: &[u8]) -> Vec<u8> {
        let mut buf = vec![0u8; 30 + fname.len()];
        buf[0..4].copy_from_slice(b"PK\x03\x04");
        let fn_len = fname.len() as u16;
        buf[26..28].copy_from_slice(&fn_len.to_le_bytes());
        buf[30..30 + fname.len()].copy_from_slice(fname);
        buf
    }

    #[test]
    fn jar_meta_inf_accepted() {
        assert!(validate_jar(&make_jar_lfh(b"META-INF/"), 0));
    }

    #[test]
    fn jar_manifest_accepted() {
        assert!(validate_jar(&make_jar_lfh(b"META-INF/MANIFEST.MF"), 0));
    }

    #[test]
    fn jar_non_meta_inf_rejected() {
        assert!(!validate_jar(&make_jar_lfh(b"com/example/Main.class"), 0));
    }

    #[test]
    fn jar_empty_fname_rejected() {
        let mut buf = make_jar_lfh(b"");
        buf[26] = 0;
        buf[27] = 0;
        assert!(!validate_jar(&buf, 0));
    }

    #[test]
    fn jar_too_short_passes() {
        assert!(validate_jar(b"PK\x03\x04", 0));
    }

    // ── LZH ───────────────────────────────────────────────────────────────────

    fn make_lzh(method: u8) -> Vec<u8> {
        // "-lh?-" at positions 0-4 (pre_validate receives pos pointing here)
        vec![b'-', b'l', b'h', method, b'-', 0, 0, 0, 0, 0]
    }

    #[test]
    fn lzh_method_0_accepted() {
        assert!(validate_lzh(&make_lzh(b'0'), 0));
    }

    #[test]
    fn lzh_method_5_accepted() {
        assert!(validate_lzh(&make_lzh(b'5'), 0));
    }

    #[test]
    fn lzh_method_d_accepted() {
        assert!(validate_lzh(&make_lzh(b'd'), 0));
    }

    #[test]
    fn lzh_method_s_accepted() {
        assert!(validate_lzh(&make_lzh(b's'), 0));
    }

    #[test]
    fn lzh_invalid_method_rejected() {
        assert!(!validate_lzh(&make_lzh(b'z'), 0));
    }

    #[test]
    fn lzh_too_short_passes() {
        assert!(validate_lzh(b"-lh", 0));
    }

    // ── HDF5 ──────────────────────────────────────────────────────────────────

    fn make_hdf5(version: u8) -> Vec<u8> {
        let mut buf = vec![0u8; 16];
        buf[..8].copy_from_slice(b"\x89HDF\r\n\x1a\n");
        buf[8] = version;
        buf
    }

    #[test]
    fn hdf5_version_0_accepted() {
        assert!(validate_hdf5(&make_hdf5(0), 0));
    }

    #[test]
    fn hdf5_version_3_accepted() {
        assert!(validate_hdf5(&make_hdf5(3), 0));
    }

    #[test]
    fn hdf5_version_4_rejected() {
        assert!(!validate_hdf5(&make_hdf5(4), 0));
    }

    #[test]
    fn hdf5_too_short_passes() {
        assert!(validate_hdf5(b"\x89HDF\r\n\x1a\n", 0));
    }

    // ── FITS ──────────────────────────────────────────────────────────────────

    fn make_fits(space_at_9: bool, t_at_29: bool) -> Vec<u8> {
        let mut buf = vec![b' '; 36];
        buf[..9].copy_from_slice(b"SIMPLE  =");
        buf[9] = if space_at_9 { b' ' } else { b'X' };
        buf[29] = if t_at_29 { b'T' } else { b'F' };
        buf
    }

    #[test]
    fn fits_valid_accepted() {
        assert!(validate_fits(&make_fits(true, true), 0));
    }

    #[test]
    fn fits_missing_space_rejected() {
        assert!(!validate_fits(&make_fits(false, true), 0));
    }

    #[test]
    fn fits_false_flag_rejected() {
        assert!(!validate_fits(&make_fits(true, false), 0));
    }

    #[test]
    fn fits_too_short_passes() {
        assert!(validate_fits(b"SIMPLE  = ", 0));
    }

    // ── VDI ───────────────────────────────────────────────────────────────────

    fn make_vdi(image_type: u32) -> Vec<u8> {
        let mut buf = vec![0u8; 16];
        buf[..4].copy_from_slice(&[0x7F, 0x10, 0xDA, 0xBE]);
        // version at pos+4..pos+7 (leave as zeros = ok)
        buf[8..12].copy_from_slice(&image_type.to_le_bytes());
        buf
    }

    #[test]
    fn vdi_type_1_normal_accepted() {
        assert!(validate_vdi(&make_vdi(1), 0));
    }

    #[test]
    fn vdi_type_2_fixed_accepted() {
        assert!(validate_vdi(&make_vdi(2), 0));
    }

    #[test]
    fn vdi_type_4_diff_accepted() {
        assert!(validate_vdi(&make_vdi(4), 0));
    }

    #[test]
    fn vdi_type_0_rejected() {
        assert!(!validate_vdi(&make_vdi(0), 0));
    }

    #[test]
    fn vdi_type_5_rejected() {
        assert!(!validate_vdi(&make_vdi(5), 0));
    }

    #[test]
    fn vdi_too_short_passes() {
        assert!(validate_vdi(&[0x7F, 0x10, 0xDA, 0xBE], 0));
    }

    // ── LNK ───────────────────────────────────────────────────────────────────

    fn make_lnk(attrs: u32) -> Vec<u8> {
        let mut buf = vec![0u8; 32];
        // HeaderSize at pos+0..pos+3
        buf[0..4].copy_from_slice(&0x4Cu32.to_le_bytes());
        // CLSID at pos+4..pos+19 (skip — already checked by magic)
        // FileAttributes at pos+24..pos+27
        buf[24..28].copy_from_slice(&attrs.to_le_bytes());
        buf
    }

    #[test]
    fn lnk_normal_file_accepted() {
        assert!(validate_lnk(&make_lnk(0x0020), 0)); // FILE_ATTRIBUTE_ARCHIVE
    }

    #[test]
    fn lnk_zero_attrs_rejected() {
        assert!(!validate_lnk(&make_lnk(0), 0));
    }

    #[test]
    fn lnk_reserved_high_bits_rejected() {
        assert!(!validate_lnk(&make_lnk(0x0001_0020), 0));
    }

    #[test]
    fn lnk_too_short_passes() {
        assert!(validate_lnk(&[0x4C, 0x00, 0x00, 0x00], 0));
    }

    // ── Prefetch ──────────────────────────────────────────────────────────────

    fn make_prefetch(version: u32, sig: &[u8; 4]) -> Vec<u8> {
        let mut buf = vec![0u8; 16];
        buf[0..4].copy_from_slice(&version.to_le_bytes());
        buf[4..8].copy_from_slice(sig);
        buf
    }

    #[test]
    fn prefetch_xp_accepted() {
        assert!(validate_prefetch(&make_prefetch(17, b"SCCA"), 0));
    }

    #[test]
    fn prefetch_win10_accepted() {
        assert!(validate_prefetch(&make_prefetch(30, b"SCCA"), 0));
    }

    #[test]
    fn prefetch_wrong_sig_rejected() {
        assert!(!validate_prefetch(&make_prefetch(17, b"XXXX"), 0));
    }

    #[test]
    fn prefetch_too_short_passes() {
        assert!(validate_prefetch(&[0x11, 0x00, 0x00, 0x00], 0));
    }

    // ── EVT ───────────────────────────────────────────────────────────────────

    fn make_evt(major: u32, minor: u32) -> Vec<u8> {
        let mut buf = vec![0u8; 20];
        buf[0..4].copy_from_slice(&48u32.to_le_bytes()); // HeaderSize
        buf[4..8].copy_from_slice(b"LfLe"); // Signature
        buf[8..12].copy_from_slice(&major.to_le_bytes());
        buf[12..16].copy_from_slice(&minor.to_le_bytes());
        buf
    }

    #[test]
    fn evt_version_1_1_accepted() {
        assert!(validate_evt(&make_evt(1, 1), 0));
    }

    #[test]
    fn evt_wrong_major_rejected() {
        assert!(!validate_evt(&make_evt(2, 1), 0));
    }

    #[test]
    fn evt_wrong_minor_rejected() {
        assert!(!validate_evt(&make_evt(1, 2), 0));
    }

    #[test]
    fn evt_too_short_passes() {
        assert!(validate_evt(b"0\x00\x00\x00LfLe", 0));
    }

    // ── PEM ───────────────────────────────────────────────────────────────────

    fn make_pem(sep: u8, label_start: u8) -> Vec<u8> {
        let mut buf = b"-----BEGIN  CERTIFICATE-----\n".to_vec();
        buf[10] = sep;
        buf[11] = label_start;
        buf
    }

    #[test]
    fn pem_certificate_accepted() {
        assert!(validate_pem(&make_pem(b' ', b'C'), 0));
    }

    #[test]
    fn pem_private_key_accepted() {
        assert!(validate_pem(&make_pem(b' ', b'P'), 0));
    }

    #[test]
    fn pem_no_space_rejected() {
        assert!(!validate_pem(&make_pem(b'-', b'C'), 0));
    }

    #[test]
    fn pem_lowercase_label_rejected() {
        assert!(!validate_pem(&make_pem(b' ', b'c'), 0));
    }

    #[test]
    fn pem_too_short_passes() {
        assert!(validate_pem(b"-----BEGIN", 0));
    }

    // ── PAR2 ─────────────────────────────────────────────────────────────────

    fn make_par2(pkt_len: u64) -> Vec<u8> {
        let mut buf = vec![0u8; 64];
        buf[0..8].copy_from_slice(b"PAR2\0PKT");
        buf[8..16].copy_from_slice(&pkt_len.to_le_bytes());
        buf
    }

    #[test]
    fn par2_valid_packet_length() {
        assert!(validate_par2(&make_par2(128), 0));
    }

    #[test]
    fn par2_minimum_valid_length() {
        assert!(validate_par2(&make_par2(64), 0));
    }

    #[test]
    fn par2_packet_length_too_small_rejected() {
        assert!(!validate_par2(&make_par2(16), 0));
    }

    #[test]
    fn par2_zero_length_rejected() {
        assert!(!validate_par2(&make_par2(0), 0));
    }

    #[test]
    fn par2_too_short_passes() {
        assert!(validate_par2(b"PAR2\0PKT", 0));
    }

    // ── WAV ───────────────────────────────────────────────────────────────────

    fn make_wav(chunk_size: u32) -> Vec<u8> {
        let mut buf = b"RIFF\x00\x00\x00\x00WAVE".to_vec();
        buf[4..8].copy_from_slice(&chunk_size.to_le_bytes());
        buf
    }

    #[test]
    fn wav_valid_chunk_size_accepted() {
        assert!(validate_wav(&make_wav(36), 0));
    }

    #[test]
    fn wav_large_chunk_size_accepted() {
        assert!(validate_wav(&make_wav(0x0400_0000), 0));
    }

    #[test]
    fn wav_chunk_size_too_small_rejected() {
        assert!(!validate_wav(&make_wav(8), 0));
    }

    #[test]
    fn wav_zero_chunk_size_rejected() {
        assert!(!validate_wav(&make_wav(0), 0));
    }

    #[test]
    fn wav_too_short_passes() {
        assert!(validate_wav(b"RIFF\x00", 0));
    }

    // ── AVI ───────────────────────────────────────────────────────────────────

    fn make_avi(chunk_size: u32) -> Vec<u8> {
        let mut buf = b"RIFF\x00\x00\x00\x00AVI ".to_vec();
        buf[4..8].copy_from_slice(&chunk_size.to_le_bytes());
        buf
    }

    #[test]
    fn avi_valid_chunk_size_accepted() {
        assert!(validate_avi(&make_avi(512), 0));
    }

    #[test]
    fn avi_minimum_chunk_size_accepted() {
        assert!(validate_avi(&make_avi(12), 0));
    }

    #[test]
    fn avi_chunk_size_too_small_rejected() {
        assert!(!validate_avi(&make_avi(4), 0));
    }

    #[test]
    fn avi_too_short_passes() {
        assert!(validate_avi(b"RIFF\x00", 0));
    }

    // ── PYC ───────────────────────────────────────────────────────────────────

    fn make_pyc(flags: u32) -> Vec<u8> {
        let mut buf = b"\x33\x0d\x0d\x0a\x00\x00\x00\x00".to_vec();
        buf[4..8].copy_from_slice(&flags.to_le_bytes());
        buf
    }

    #[test]
    fn pyc_flags_zero_accepted() {
        assert!(validate_pyc(&make_pyc(0), 0));
    }

    #[test]
    fn pyc_flags_one_accepted() {
        assert!(validate_pyc(&make_pyc(1), 0));
    }

    #[test]
    fn pyc_flags_three_accepted() {
        assert!(validate_pyc(&make_pyc(3), 0));
    }

    #[test]
    fn pyc_flags_four_rejected() {
        assert!(!validate_pyc(&make_pyc(4), 0));
    }

    #[test]
    fn pyc_flags_high_rejected() {
        assert!(!validate_pyc(&make_pyc(0xDEAD_BEEF), 0));
    }

    #[test]
    fn pyc_too_short_passes() {
        assert!(validate_pyc(b"\x33\x0d\x0d\x0a", 0));
    }

    // ── DPX ───────────────────────────────────────────────────────────────────

    fn make_dpx_be(ver: &[u8; 4]) -> Vec<u8> {
        let mut buf = vec![0u8; 16];
        buf[0..4].copy_from_slice(b"SDPX");
        buf[8..12].copy_from_slice(ver);
        buf
    }

    #[test]
    fn dpx_v1_accepted() {
        assert!(validate_dpx(&make_dpx_be(b"V1.0"), 0));
    }

    #[test]
    fn dpx_v2_accepted() {
        assert!(validate_dpx(&make_dpx_be(b"V2.0"), 0));
    }

    #[test]
    fn dpx_garbage_version_rejected() {
        assert!(!validate_dpx(&make_dpx_be(b"XYZ!"), 0));
    }

    #[test]
    fn dpx_lowercase_v_rejected() {
        assert!(!validate_dpx(&make_dpx_be(b"v1.0"), 0));
    }

    #[test]
    fn dpx_too_short_passes() {
        assert!(validate_dpx(b"SDPX\x00\x00\x00\x00", 0));
    }

    // ── EXR ───────────────────────────────────────────────────────────────────

    fn make_exr(ver: u8, flags: u8) -> Vec<u8> {
        vec![0x76, 0x2F, 0x31, 0x01, ver, flags, 0x00, 0x00]
    }

    #[test]
    fn exr_version2_no_flags_accepted() {
        assert!(validate_exr(&make_exr(2, 0x00), 0));
    }

    #[test]
    fn exr_version2_tile_flag_accepted() {
        // bit 8 of the version u32 = byte @5 bit 0 = 0x01
        assert!(validate_exr(&make_exr(2, 0x01), 0));
    }

    #[test]
    fn exr_version2_all_known_flags_accepted() {
        // bits 8–11 set: byte @5 = 0x0F
        assert!(validate_exr(&make_exr(2, 0x0F), 0));
    }

    #[test]
    fn exr_wrong_version_rejected() {
        assert!(!validate_exr(&make_exr(1, 0x00), 0));
    }

    #[test]
    fn exr_reserved_flag_bits_rejected() {
        // upper nibble of byte @5 is reserved
        assert!(!validate_exr(&make_exr(2, 0xF0), 0));
    }

    #[test]
    fn exr_too_short_passes() {
        assert!(validate_exr(&[0x76, 0x2F, 0x31, 0x01], 0));
    }
}

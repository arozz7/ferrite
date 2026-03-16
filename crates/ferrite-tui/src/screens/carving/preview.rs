//! Hit Preview Panel — reads raw bytes from the device and renders a metadata
//! summary + optional halfblock image for the currently selected carve hit.

use ferrite_blockdev::{AlignedBuffer, BlockDevice};
use ferrite_carver::CarveHit;

// ── Terminal colour capability ─────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum ColorCap {
    TrueColor,
    Color256,
    Basic,
}

impl ColorCap {
    pub(crate) fn detect() -> Self {
        if let Ok(ct) = std::env::var("COLORTERM") {
            if ct == "truecolor" || ct == "24bit" {
                return Self::TrueColor;
            }
        }
        if std::env::var("WT_SESSION").is_ok() {
            return Self::TrueColor;
        }
        if let Ok(tp) = std::env::var("TERM_PROGRAM") {
            if matches!(tp.as_str(), "iTerm.app" | "Hyper" | "WezTerm" | "vscode") {
                return Self::TrueColor;
            }
        }
        if let Ok(term) = std::env::var("TERM") {
            if term.contains("256color") || term.contains("truecolor") {
                return Self::Color256;
            }
        }
        Self::Basic
    }
}

// ── File metadata ──────────────────────────────────────────────────────────────

pub(crate) struct FileMetadata {
    pub format: String,
    pub dimensions: Option<(u32, u32)>,
    pub extra: Vec<(&'static str, String)>,
}

pub(crate) struct HitPreview {
    pub metadata: FileMetadata,
    pub image: Option<image::DynamicImage>,
}

// ── Public entry point ─────────────────────────────────────────────────────────

/// Read up to 64 KiB from the device at `hit.byte_offset` and build a preview.
/// Returns `None` if the device cannot be read or no parser matches.
pub(crate) fn read_preview(device: &dyn BlockDevice, hit: &CarveHit) -> Option<HitPreview> {
    let ss = device.sector_size() as usize;
    let read_size = (65536usize).div_ceil(ss) * ss;
    let offset = hit.byte_offset;
    let available = device.size().saturating_sub(offset);
    if available == 0 {
        return None;
    }
    let read_size = read_size.min(available as usize);
    let read_size = read_size.max(ss);

    let mut buf = AlignedBuffer::new(read_size, ss);
    let n = device.read_at(offset, &mut buf).ok()?;
    if n == 0 {
        return None;
    }
    let data = &buf.as_slice()[..n];
    let ext = hit.signature.extension.as_str();

    parse_by_extension(ext, data)
}

fn parse_by_extension(ext: &str, data: &[u8]) -> Option<HitPreview> {
    match ext {
        "jpg" | "jpeg" => parse_jpeg(data),
        "png" => parse_png(data),
        "bmp" => parse_bmp(data),
        "gif" => parse_gif(data),
        "mp3" => parse_mp3_id3(data),
        "flac" => parse_flac(data),
        "zip" => super::preview_more::parse_zip(data),
        "pdf" => super::preview_more::parse_pdf(data),
        "db" => super::preview_more::parse_sqlite(data),
        "exe" | "dll" => super::preview_more::parse_pe(data),
        _ => Some(HitPreview {
            metadata: FileMetadata {
                format: ext.to_uppercase(),
                dimensions: None,
                extra: vec![],
            },
            image: None,
        }),
    }
}

// ── Format-specific parsers ────────────────────────────────────────────────────

fn parse_jpeg(data: &[u8]) -> Option<HitPreview> {
    let mut width = 0u32;
    let mut height = 0u32;
    let mut date = None::<String>;

    // Scan for SOF markers (0xFF 0xC0..0xC3, 0xC5..0xC7, 0xC9..0xCB, 0xCD..0xCF)
    let mut i = 2usize; // skip SOI marker
    while i + 4 < data.len() {
        if data[i] != 0xFF {
            break;
        }
        let marker = data[i + 1];
        if i + 4 >= data.len() {
            break;
        }
        let seg_len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
        if seg_len < 2 {
            break;
        }

        // SOF markers contain image dimensions
        if matches!(marker, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF)
            && i + 8 < data.len()
        {
            height = u16::from_be_bytes([data[i + 5], data[i + 6]]) as u32;
            width = u16::from_be_bytes([data[i + 7], data[i + 8]]) as u32;
        }

        // Scan EXIF APP1 for date
        if marker == 0xE1 && i + 10 < data.len() && data[i + 4..].starts_with(b"Exif\x00\x00") {
            date = extract_exif_date(&data[i + 4..]);
        }

        i += 2 + seg_len;
    }

    let mut extra = vec![];
    if let Some(d) = date {
        extra.push(("Date", d));
    }

    // Try to decode image for halfblock rendering
    let image = try_decode_image(data);

    Some(HitPreview {
        metadata: FileMetadata {
            format: "JPEG".to_string(),
            dimensions: if width > 0 && height > 0 {
                Some((width, height))
            } else {
                None
            },
            extra,
        },
        image,
    })
}

fn extract_exif_date(exif_data: &[u8]) -> Option<String> {
    // Scan for ASCII date string pattern YYYY:MM:DD HH:MM:SS
    if exif_data.len() < 19 {
        return None;
    }
    for i in 0..exif_data.len().saturating_sub(19) {
        let chunk = &exif_data[i..i + 19];
        // Check for YYYY:MM:DD HH:MM:SS pattern
        if chunk[4] == b':'
            && chunk[7] == b':'
            && chunk[10] == b' '
            && chunk[13] == b':'
            && chunk[16] == b':'
            && chunk[..4].iter().all(|b| b.is_ascii_digit())
        {
            if let Ok(s) = std::str::from_utf8(chunk) {
                return Some(s.to_string());
            }
        }
    }
    None
}

fn parse_png(data: &[u8]) -> Option<HitPreview> {
    // PNG signature = 8 bytes, then IHDR chunk: 4-byte length, "IHDR", width(4), height(4), ...
    if data.len() < 26 || !data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(HitPreview {
            metadata: FileMetadata {
                format: "PNG".into(),
                dimensions: None,
                extra: vec![],
            },
            image: None,
        });
    }
    // IHDR starts at offset 8: [length=13][IHDR][width][height][bitdepth][colortype]...
    let width = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
    let height = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
    let bit_depth = data[24];
    let color_type = data[25];
    let color_desc = match color_type {
        0 => "Grayscale",
        2 => "RGB",
        3 => "Indexed",
        4 => "Grayscale+Alpha",
        6 => "RGBA",
        _ => "Unknown",
    };

    let image = try_decode_image(data);

    Some(HitPreview {
        metadata: FileMetadata {
            format: "PNG".into(),
            dimensions: Some((width, height)),
            extra: vec![
                ("Bit depth", bit_depth.to_string()),
                ("Color", color_desc.to_string()),
            ],
        },
        image,
    })
}

fn parse_bmp(data: &[u8]) -> Option<HitPreview> {
    if data.len() < 30 || !data.starts_with(b"BM") {
        return Some(HitPreview {
            metadata: FileMetadata {
                format: "BMP".into(),
                dimensions: None,
                extra: vec![],
            },
            image: None,
        });
    }
    // BITMAPFILEHEADER (14 bytes) + BITMAPINFOHEADER starts at byte 14
    // width at offset 18, height at offset 22 (both i32 LE)
    let width = u32::from_le_bytes([data[18], data[19], data[20], data[21]]);
    let height_raw = i32::from_le_bytes([data[22], data[23], data[24], data[25]]);
    let height = height_raw.unsigned_abs();
    let bit_count = u16::from_le_bytes([data[28], data[29]]);

    let image = try_decode_image(data);

    Some(HitPreview {
        metadata: FileMetadata {
            format: "BMP".into(),
            dimensions: Some((width, height)),
            extra: vec![("Bits/pixel", bit_count.to_string())],
        },
        image,
    })
}

fn parse_gif(data: &[u8]) -> Option<HitPreview> {
    // GIF87a or GIF89a: 6-byte signature, then logical screen width(2 LE), height(2 LE)
    if data.len() < 10 || (!data.starts_with(b"GIF87a") && !data.starts_with(b"GIF89a")) {
        return Some(HitPreview {
            metadata: FileMetadata {
                format: "GIF".into(),
                dimensions: None,
                extra: vec![],
            },
            image: None,
        });
    }
    let width = u16::from_le_bytes([data[6], data[7]]) as u32;
    let height = u16::from_le_bytes([data[8], data[9]]) as u32;
    let version = if data.starts_with(b"GIF89a") {
        "GIF89a"
    } else {
        "GIF87a"
    };

    Some(HitPreview {
        metadata: FileMetadata {
            format: "GIF".into(),
            dimensions: Some((width, height)),
            extra: vec![("Version", version.to_string())],
        },
        image: try_decode_image(data),
    })
}

fn parse_mp3_id3(data: &[u8]) -> Option<HitPreview> {
    // ID3v2 header: "ID3" + version(1) + revision(1) + flags(1) + size(4 syncsafe)
    if data.len() < 10 || !data.starts_with(b"ID3") {
        return Some(HitPreview {
            metadata: FileMetadata {
                format: "MP3".into(),
                dimensions: None,
                extra: vec![],
            },
            image: None,
        });
    }
    let version = data[3];
    let revision = data[4];
    let size = syncsafe_to_u32([data[6], data[7], data[8], data[9]]);

    let mut extra = vec![
        ("Version", format!("ID3v2.{version}.{revision}")),
        ("Tag size", format!("{size} B")),
    ];

    // Scan for TIT2 (title) and TPE1 (artist) frames (ID3v2.3+)
    if version >= 3 {
        let tag_data_end = (10 + size as usize).min(data.len());
        let tag_data = &data[10..tag_data_end];
        if let Some(title) = id3_text_frame(tag_data, b"TIT2") {
            extra.push(("Title", title));
        }
        if let Some(artist) = id3_text_frame(tag_data, b"TPE1") {
            extra.push(("Artist", artist));
        }
        if let Some(album) = id3_text_frame(tag_data, b"TALB") {
            extra.push(("Album", album));
        }
    }

    Some(HitPreview {
        metadata: FileMetadata {
            format: "MP3".into(),
            dimensions: None,
            extra,
        },
        image: None,
    })
}

fn syncsafe_to_u32(bytes: [u8; 4]) -> u32 {
    ((bytes[0] as u32) << 21)
        | ((bytes[1] as u32) << 14)
        | ((bytes[2] as u32) << 7)
        | (bytes[3] as u32)
}

fn id3_text_frame(tag_data: &[u8], frame_id: &[u8; 4]) -> Option<String> {
    let mut i = 0;
    while i + 10 < tag_data.len() {
        let fid = &tag_data[i..i + 4];
        let fsize = u32::from_be_bytes([
            tag_data[i + 4],
            tag_data[i + 5],
            tag_data[i + 6],
            tag_data[i + 7],
        ]) as usize;
        if fid == frame_id && fsize > 1 && i + 10 + fsize <= tag_data.len() {
            let content = &tag_data[i + 10..i + 10 + fsize];
            // First byte is encoding; skip it
            let text = match content[0] {
                1 | 2 => {
                    // UTF-16: skip BOM if present
                    let utf16_bytes = &content[1..];
                    let chars: Vec<u16> = utf16_bytes
                        .chunks_exact(2)
                        .map(|c| u16::from_le_bytes([c[0], c[1]]))
                        .take_while(|&c| c != 0)
                        .collect();
                    String::from_utf16_lossy(&chars).to_string()
                }
                _ => {
                    // UTF-8 or Latin-1
                    String::from_utf8_lossy(&content[1..])
                        .trim_end_matches('\0')
                        .to_string()
                }
            };
            if !text.is_empty() {
                return Some(text);
            }
        }
        if fsize == 0 {
            break;
        }
        i += 10 + fsize;
    }
    None
}

fn parse_flac(data: &[u8]) -> Option<HitPreview> {
    // fLaC marker + STREAMINFO metadata block
    if data.len() < 42 || !data.starts_with(b"fLaC") {
        return Some(HitPreview {
            metadata: FileMetadata {
                format: "FLAC".into(),
                dimensions: None,
                extra: vec![],
            },
            image: None,
        });
    }
    // Block header at offset 4: [last_block(1bit) | block_type(7bits)][length(24bits)]
    let block_type = data[4] & 0x7F;
    if block_type != 0 {
        // First block must be STREAMINFO (type 0)
        return Some(HitPreview {
            metadata: FileMetadata {
                format: "FLAC".into(),
                dimensions: None,
                extra: vec![],
            },
            image: None,
        });
    }
    // STREAMINFO data starts at offset 8 (4-byte block header)
    let s = &data[8..]; // STREAMINFO is 34 bytes
    if s.len() < 34 {
        return Some(HitPreview {
            metadata: FileMetadata {
                format: "FLAC".into(),
                dimensions: None,
                extra: vec![],
            },
            image: None,
        });
    }
    // Bytes 10-17 of STREAMINFO: sample_rate(20b) | channels-1(3b) | bits_per_sample-1(5b) | total_samples(36b)
    let packed = u64::from_be_bytes([s[10], s[11], s[12], s[13], s[14], s[15], s[16], s[17]]);
    let sample_rate = (packed >> 44) as u32; // top 20 bits
    let channels = (((packed >> 41) & 0x7) as u8) + 1; // next 3 bits, 0-indexed
    let bits_per_sample = (((packed >> 36) & 0x1F) as u8) + 1; // next 5 bits, 0-indexed
    let total_samples = packed & 0x0FFFFFFFFF; // bottom 36 bits

    let duration_secs = if sample_rate > 0 {
        total_samples / sample_rate as u64
    } else {
        0
    };

    let extra = vec![
        ("Sample rate", format!("{sample_rate} Hz")),
        ("Channels", channels.to_string()),
        ("Bit depth", format!("{bits_per_sample} bit")),
        (
            "Duration",
            format!("{:02}:{:02}", duration_secs / 60, duration_secs % 60),
        ),
    ];

    Some(HitPreview {
        metadata: FileMetadata {
            format: "FLAC".into(),
            dimensions: None,
            extra,
        },
        image: None,
    })
}

// ── Image decoding ─────────────────────────────────────────────────────────────

fn try_decode_image(data: &[u8]) -> Option<image::DynamicImage> {
    use image::ImageReader;
    let cursor = std::io::Cursor::new(data);
    let reader = ImageReader::new(cursor).with_guessed_format().ok()?;
    reader.decode().ok()
}

// Re-export rendering from preview_more so callers use `preview::render_preview`.
pub(crate) use super::preview_more::render_preview;

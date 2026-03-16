//! ZIP, PDF, SQLite, PE parsers and preview rendering — split from `preview.rs`.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use super::helpers::fmt_bytes;
use super::preview::{ColorCap, FileMetadata, HitPreview};

pub(super) fn parse_zip(data: &[u8]) -> Option<HitPreview> {
    // Scan local file headers: PK\x03\x04
    let mut names = Vec::new();
    let mut i = 0usize;
    while i + 30 < data.len() && names.len() < 8 {
        if &data[i..i + 4] == b"PK\x03\x04" {
            let name_len = u16::from_le_bytes([data[i + 26], data[i + 27]]) as usize;
            let extra_len = u16::from_le_bytes([data[i + 28], data[i + 29]]) as usize;
            if i + 30 + name_len <= data.len() {
                if let Ok(name) = std::str::from_utf8(&data[i + 30..i + 30 + name_len]) {
                    if !name.is_empty() {
                        names.push(name.to_string());
                    }
                }
            }
            i += 30 + name_len + extra_len;
        } else {
            i += 1;
        }
    }

    const LABELS: [&str; 8] = [
        "File 1", "File 2", "File 3", "File 4", "File 5", "File 6", "File 7", "File 8",
    ];
    let extra = names
        .into_iter()
        .enumerate()
        .map(|(i, n)| (LABELS[i], n))
        .collect();

    Some(HitPreview {
        metadata: FileMetadata {
            format: "ZIP".into(),
            dimensions: None,
            extra,
        },
        image: None,
    })
}

pub(super) fn parse_pdf(data: &[u8]) -> Option<HitPreview> {
    // Scan for /Creator, /Author, /Title values in PDF dictionary
    let text = std::str::from_utf8(data).unwrap_or("");
    let mut extra = vec![];
    for (key, label) in &[
        ("/Title", "Title"),
        ("/Author", "Author"),
        ("/Creator", "Creator"),
    ] {
        if let Some(val) = extract_pdf_string(text, key) {
            extra.push((*label, val));
        }
    }
    Some(HitPreview {
        metadata: FileMetadata {
            format: "PDF".into(),
            dimensions: None,
            extra,
        },
        image: None,
    })
}

fn extract_pdf_string(text: &str, key: &str) -> Option<String> {
    let pos = text.find(key)?;
    let rest = &text[pos + key.len()..];
    let rest = rest.trim_start();
    if rest.starts_with('(') {
        let end = rest.find(')')?;
        Some(rest[1..end].to_string())
    } else if rest.starts_with('/') {
        let end = rest.find(|c: char| c.is_whitespace() || c == '>')?;
        Some(rest[1..end].to_string())
    } else {
        None
    }
}

pub(super) fn parse_sqlite(data: &[u8]) -> Option<HitPreview> {
    // SQLite header: "SQLite format 3\0" (16 bytes) + page_size(2 BE) + ...
    if data.len() < 100 || !data.starts_with(b"SQLite format 3\x00") {
        return Some(HitPreview {
            metadata: FileMetadata {
                format: "SQLite".into(),
                dimensions: None,
                extra: vec![],
            },
            image: None,
        });
    }
    let page_size_raw = u16::from_be_bytes([data[16], data[17]]);
    let page_size: u32 = if page_size_raw == 1 {
        65536
    } else {
        page_size_raw as u32
    };
    let page_count = u32::from_be_bytes([data[28], data[29], data[30], data[31]]);
    let size_bytes = page_size as u64 * page_count as u64;
    let text_encoding = match data[56] {
        1 => "UTF-8",
        2 => "UTF-16 LE",
        3 => "UTF-16 BE",
        _ => "Unknown",
    };

    let extra = vec![
        ("Page size", format!("{page_size} B")),
        ("Pages", page_count.to_string()),
        ("Est. size", fmt_bytes(size_bytes)),
        ("Encoding", text_encoding.to_string()),
    ];

    Some(HitPreview {
        metadata: FileMetadata {
            format: "SQLite DB".into(),
            dimensions: None,
            extra,
        },
        image: None,
    })
}

pub(super) fn parse_pe(data: &[u8]) -> Option<HitPreview> {
    // DOS header: "MZ" at offset 0, PE offset at 0x3C
    if data.len() < 64 || !data.starts_with(b"MZ") {
        return Some(HitPreview {
            metadata: FileMetadata {
                format: "PE".into(),
                dimensions: None,
                extra: vec![],
            },
            image: None,
        });
    }
    let pe_offset = u32::from_le_bytes([data[0x3C], data[0x3D], data[0x3E], data[0x3F]]) as usize;
    if pe_offset + 24 >= data.len() || &data[pe_offset..pe_offset + 4] != b"PE\x00\x00" {
        return Some(HitPreview {
            metadata: FileMetadata {
                format: "PE Executable".into(),
                dimensions: None,
                extra: vec![],
            },
            image: None,
        });
    }
    let machine = u16::from_le_bytes([data[pe_offset + 4], data[pe_offset + 5]]);
    let machine_str = match machine {
        0x014C => "x86 (32-bit)",
        0x8664 => "x86-64",
        0xAA64 => "ARM64",
        0x01C0 | 0x01C4 => "ARM",
        _ => "Unknown",
    };
    let num_sections = u16::from_le_bytes([data[pe_offset + 6], data[pe_offset + 7]]);
    let opt_header_size = u16::from_le_bytes([data[pe_offset + 20], data[pe_offset + 21]]) as usize;

    // Optional header at pe_offset + 24
    let subsystem_str = if opt_header_size >= 68 && pe_offset + 24 + 68 <= data.len() {
        let magic = u16::from_le_bytes([data[pe_offset + 24], data[pe_offset + 25]]);
        let sub_offset = if magic == 0x20B { 92 } else { 68 }; // PE32+ vs PE32
        if pe_offset + 24 + sub_offset + 2 <= data.len() {
            let sub = u16::from_le_bytes([
                data[pe_offset + 24 + sub_offset],
                data[pe_offset + 24 + sub_offset + 1],
            ]);
            match sub {
                1 => "Native",
                2 => "GUI",
                3 => "Console",
                9 => "WinCE GUI",
                10 => "EFI Application",
                _ => "Unknown",
            }
        } else {
            "Unknown"
        }
    } else {
        "Unknown"
    };

    let extra = vec![
        ("Architecture", machine_str.to_string()),
        ("Subsystem", subsystem_str.to_string()),
        ("Sections", num_sections.to_string()),
    ];

    Some(HitPreview {
        metadata: FileMetadata {
            format: "PE Executable".into(),
            dimensions: None,
            extra,
        },
        image: None,
    })
}

// ── Rendering ─────────────────────────────────────────────────────────────────

pub(crate) fn render_preview(
    frame: &mut Frame,
    area: Rect,
    preview: &HitPreview,
    color_cap: ColorCap,
) {
    let block = Block::default().borders(Borders::ALL).title(Span::styled(
        format!(" Preview — {} ", preview.metadata.format),
        Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
    ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 {
        return;
    }

    // If we have image data and enough color support, render halfblocks.
    if let Some(img) = &preview.image {
        if color_cap != ColorCap::Basic && inner.width >= 8 && inner.height >= 4 {
            // Split: top portion for halfblock image, bottom for metadata
            let img_height = (inner.height * 2 / 3).max(2);
            let meta_height = inner.height.saturating_sub(img_height);
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(img_height),
                    Constraint::Length(meta_height),
                ])
                .split(inner);
            render_halfblocks(frame, chunks[0], img);
            render_metadata_lines(frame, chunks[1], &preview.metadata);
            return;
        }
    }

    // Fallback: just show metadata text
    render_metadata_lines(frame, inner, &preview.metadata);
}

fn render_metadata_lines(frame: &mut Frame, area: Rect, meta: &FileMetadata) {
    let mut lines: Vec<Line> = Vec::new();

    // Dimensions line
    if let Some((w, h)) = meta.dimensions {
        lines.push(Line::from(vec![
            Span::styled("Dimensions: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{w} × {h}"), Style::default().fg(Color::White)),
        ]));
    }

    // Extra key-value pairs
    for (key, val) in &meta.extra {
        lines.push(Line::from(vec![
            Span::styled(format!("{key}: "), Style::default().fg(Color::DarkGray)),
            Span::styled(val.clone(), Style::default().fg(Color::White)),
        ]));
    }

    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            " No metadata available",
            Style::default().fg(Color::DarkGray),
        )));
    }

    frame.render_widget(Paragraph::new(Text::from(lines)), area);
}

fn render_halfblocks(frame: &mut Frame, area: Rect, img: &image::DynamicImage) {
    let target_w = area.width as u32;
    let target_h = (area.height as u32) * 2; // two pixel rows per terminal character row
    if target_w == 0 || target_h == 0 {
        return;
    }

    let resized = img.resize(target_w, target_h, image::imageops::FilterType::Nearest);
    let rgba = resized.to_rgba8();
    let img_w = rgba.width().min(target_w) as u16;
    let img_h = rgba.height();

    let mut lines: Vec<Line> = Vec::new();
    let char_rows = area.height;

    for char_row in 0..char_rows {
        let px_top = (char_row as u32) * 2;
        let px_bot = px_top + 1;
        let mut spans: Vec<Span> = Vec::with_capacity(img_w as usize);
        for col in 0..img_w {
            let top = if px_top < img_h {
                *rgba.get_pixel(col as u32, px_top)
            } else {
                image::Rgba([0, 0, 0, 255])
            };
            let bot = if px_bot < img_h {
                *rgba.get_pixel(col as u32, px_bot)
            } else {
                image::Rgba([0, 0, 0, 255])
            };
            let fg = Color::Rgb(top[0], top[1], top[2]);
            let bg = Color::Rgb(bot[0], bot[1], bot[2]);
            spans.push(Span::styled("▀", Style::default().fg(fg).bg(bg)));
        }
        lines.push(Line::from(spans));
    }

    frame.render_widget(Paragraph::new(Text::from(lines)), area);
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_zip_extracts_file_names() {
        // Minimal ZIP local file header for "hello.txt"
        let name = b"hello.txt";
        let mut data = vec![0u8; 30 + name.len()];
        data[0..4].copy_from_slice(b"PK\x03\x04");
        let name_len = (name.len() as u16).to_le_bytes();
        data[26] = name_len[0];
        data[27] = name_len[1];
        data[30..30 + name.len()].copy_from_slice(name);

        let preview = parse_zip(&data).unwrap();
        assert_eq!(preview.metadata.format, "ZIP");
        assert!(preview.metadata.extra.iter().any(|(_, v)| v == "hello.txt"));
    }

    #[test]
    fn parse_pdf_extracts_title_and_author() {
        let data = b"%PDF-1.4\n/Title (My Document)\n/Author (Alice)\n";
        let preview = parse_pdf(data).unwrap();
        assert_eq!(preview.metadata.format, "PDF");
        assert!(preview
            .metadata
            .extra
            .iter()
            .any(|(k, v)| *k == "Title" && v == "My Document"));
        assert!(preview
            .metadata
            .extra
            .iter()
            .any(|(k, v)| *k == "Author" && v == "Alice"));
    }

    #[test]
    fn parse_sqlite_extracts_page_info() {
        let mut data = vec![0u8; 100];
        data[..16].copy_from_slice(b"SQLite format 3\x00");
        data[16] = 0x10; // page_size high byte => 4096
        data[17] = 0x00;
        data[31] = 0x02; // page_count = 2
        data[56] = 1; // encoding = UTF-8

        let preview = parse_sqlite(&data).unwrap();
        assert_eq!(preview.metadata.format, "SQLite DB");
        assert!(preview
            .metadata
            .extra
            .iter()
            .any(|(k, _)| *k == "Page size"));
        assert!(preview
            .metadata
            .extra
            .iter()
            .any(|(k, v)| *k == "Encoding" && v == "UTF-8"));
    }

    #[test]
    fn parse_pe_detects_x86_64_architecture() {
        let mut data = vec![0u8; 256];
        data[0] = b'M';
        data[1] = b'Z';
        data[0x3C] = 64; // PE header at offset 64
        data[64..68].copy_from_slice(b"PE\x00\x00");
        data[68] = 0x64; // Machine = 0x8664 (x86-64) LE
        data[69] = 0x86;
        data[70] = 3; // num_sections = 3

        let preview = parse_pe(&data).unwrap();
        assert_eq!(preview.metadata.format, "PE Executable");
        assert!(preview
            .metadata
            .extra
            .iter()
            .any(|(k, v)| *k == "Architecture" && v == "x86-64"));
        assert!(preview
            .metadata
            .extra
            .iter()
            .any(|(k, v)| *k == "Sections" && v == "3"));
    }
}

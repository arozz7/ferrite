//! Screen 7 — Sector Hex Viewer: browse raw device sectors with a classic
//! hex-dump layout.  Supports byte-offset jumping, 16-sector page steps,
//! per-byte colour coding, and automatic sector-type annotation.

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyModifiers};
use ferrite_blockdev::{AlignedBuffer, BlockDevice};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(PartialEq)]
enum EditMode {
    None,
    /// Typing a decimal LBA number.
    Lba,
    /// Typing a byte offset — decimal or 0x-prefixed hex.
    Offset,
}

// ── State ─────────────────────────────────────────────────────────────────────

pub struct HexViewerState {
    pub device: Option<Arc<dyn BlockDevice>>,
    /// Sector currently displayed.
    pub current_lba: u64,
    /// Shared text input buffer (both LBA and offset modes).
    input: String,
    edit_mode: EditMode,
    /// Raw bytes of the current sector.
    pub data: Option<Vec<u8>>,
    /// Byte index within the current sector to highlight (set on byte-offset jump).
    highlight_byte: Option<usize>,
}

impl Default for HexViewerState {
    fn default() -> Self {
        Self::new()
    }
}

impl HexViewerState {
    pub fn new() -> Self {
        Self {
            device: None,
            current_lba: 0,
            input: String::new(),
            edit_mode: EditMode::None,
            data: None,
            highlight_byte: None,
        }
    }

    /// Returns `true` while a text-input field is active (so `q` won't quit).
    pub fn is_editing(&self) -> bool {
        self.edit_mode != EditMode::None
    }

    /// Attach a device and reset to LBA 0.
    pub fn set_device(&mut self, device: Arc<dyn BlockDevice>) {
        self.device = Some(device);
        self.current_lba = 0;
        self.input.clear();
        self.edit_mode = EditMode::None;
        self.data = None;
        self.highlight_byte = None;
        self.load_sector();
    }

    /// Jump to the sector containing `offset` and highlight the landing byte.
    /// Called by `app.rs` when deep-linking from the carving screen.
    pub fn jump_to_byte_offset(&mut self, offset: u64) {
        if let Some(device) = &self.device {
            let ss = device.sector_size() as u64;
            self.current_lba = offset / ss;
            self.highlight_byte = Some((offset % ss) as usize);
            self.load_sector();
        }
    }

    fn last_lba(&self) -> u64 {
        if let Some(device) = &self.device {
            let ss = device.sector_size() as u64;
            let size = device.size();
            if size == 0 {
                0
            } else {
                (size - 1) / ss
            }
        } else {
            0
        }
    }

    /// Read the sector at `current_lba` into `self.data`.
    pub fn load_sector(&mut self) {
        let device = match &self.device {
            Some(d) => Arc::clone(d),
            None => return,
        };
        let ss = device.sector_size() as usize;
        let offset = self.current_lba * device.sector_size() as u64;
        let mut buf = AlignedBuffer::new(ss, ss);
        match device.read_at(offset, &mut buf) {
            Ok(n) => self.data = Some(buf.as_slice()[..n].to_vec()),
            Err(_) => self.data = None,
        }
    }

    pub fn handle_key(&mut self, code: KeyCode, _modifiers: KeyModifiers) {
        match self.edit_mode {
            EditMode::Lba => {
                match code {
                    KeyCode::Enter => {
                        if let Ok(lba) = self.input.trim().parse::<u64>() {
                            self.current_lba = lba.min(self.last_lba());
                            self.highlight_byte = None;
                            self.load_sector();
                        }
                        self.edit_mode = EditMode::None;
                    }
                    KeyCode::Esc => self.edit_mode = EditMode::None,
                    KeyCode::Backspace => {
                        self.input.pop();
                    }
                    KeyCode::Char(c) if c.is_ascii_digit() => self.input.push(c),
                    _ => {}
                }
                return;
            }
            EditMode::Offset => {
                match code {
                    KeyCode::Enter => {
                        let raw = self.input.trim().to_string();
                        let parsed = if raw.starts_with("0x") || raw.starts_with("0X") {
                            u64::from_str_radix(&raw[2..], 16).ok()
                        } else {
                            raw.parse::<u64>().ok()
                        };
                        if let Some(offset) = parsed {
                            self.jump_to_byte_offset(offset);
                        }
                        self.edit_mode = EditMode::None;
                    }
                    KeyCode::Esc => self.edit_mode = EditMode::None,
                    KeyCode::Backspace => {
                        self.input.pop();
                    }
                    // Accept hex digits and the 'x'/'X' for a 0x prefix.
                    KeyCode::Char(c) if c.is_ascii_hexdigit() || c == 'x' || c == 'X' => {
                        self.input.push(c);
                    }
                    _ => {}
                }
                return;
            }
            EditMode::None => {}
        }

        match code {
            KeyCode::Up => {
                self.current_lba = self.current_lba.saturating_sub(1);
                self.highlight_byte = None;
                self.load_sector();
            }
            KeyCode::Down => {
                self.current_lba = (self.current_lba + 1).min(self.last_lba());
                self.highlight_byte = None;
                self.load_sector();
            }
            KeyCode::PageUp => {
                self.current_lba = self.current_lba.saturating_sub(16);
                self.highlight_byte = None;
                self.load_sector();
            }
            KeyCode::PageDown => {
                self.current_lba = (self.current_lba + 16).min(self.last_lba());
                self.highlight_byte = None;
                self.load_sector();
            }
            KeyCode::Home => {
                self.current_lba = 0;
                self.highlight_byte = None;
                self.load_sector();
            }
            KeyCode::End => {
                self.current_lba = self.last_lba();
                self.highlight_byte = None;
                self.load_sector();
            }
            KeyCode::Char('g') => {
                self.input.clear();
                self.edit_mode = EditMode::Lba;
            }
            KeyCode::Char('b') => {
                self.input.clear();
                self.edit_mode = EditMode::Offset;
            }
            _ => {}
        }
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        if self.device.is_some() && self.data.is_none() && self.edit_mode == EditMode::None {
            self.load_sector();
        }

        let outer = Block::default()
            .borders(Borders::ALL)
            .title(" Hex Viewer — ↑/↓: sector  PgUp/PgDn: ±16  Home/End  g: LBA  b: offset ");
        let inner = outer.inner(area);
        frame.render_widget(outer, area);

        if self.device.is_none() {
            frame.render_widget(Paragraph::new(" No device selected."), inner);
            return;
        }

        let device = self.device.as_ref().unwrap();
        let ss = device.sector_size() as u64;
        let total_sectors = if device.size() > 0 {
            device.size().div_ceil(ss)
        } else {
            0
        };
        let offset_base = self.current_lba * ss;

        let sector_type = self
            .data
            .as_deref()
            .and_then(|d| detect_sector_type(d, self.current_lba));

        let mut lines: Vec<Line> = Vec::new();

        // ── Header ────────────────────────────────────────────────────────────
        let header = format!(
            " LBA {}  /  {}    offset {offset_base:#010x}    sector {} B",
            self.current_lba,
            total_sectors.saturating_sub(1),
            ss,
        );
        let mut header_spans = vec![Span::styled(
            header,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )];
        if let Some(t) = sector_type {
            header_spans.push(Span::styled(
                format!("  [{t}]"),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        lines.push(Line::from(header_spans));

        // ── Input prompt ──────────────────────────────────────────────────────
        match self.edit_mode {
            EditMode::Lba => {
                lines.push(Line::from(Span::styled(
                    format!(" Jump to LBA: {}_", self.input),
                    Style::default().fg(Color::Yellow),
                )));
            }
            EditMode::Offset => {
                lines.push(Line::from(Span::styled(
                    format!(" Jump to byte offset (dec or 0x…): {}_", self.input),
                    Style::default().fg(Color::Yellow),
                )));
            }
            EditMode::None => {}
        }

        // ── Landing hint ──────────────────────────────────────────────────────
        if let Some(hb) = self.highlight_byte {
            lines.push(Line::from(Span::styled(
                format!(
                    " ► landed at byte +{hb:#x} within sector  (absolute {:#010x})",
                    offset_base + hb as u64,
                ),
                Style::default().fg(Color::Yellow),
            )));
        }

        // ── Hex dump ──────────────────────────────────────────────────────────
        if let Some(data) = &self.data {
            for row in 0..data.len().div_ceil(16) {
                let start = row * 16;
                let end = (start + 16).min(data.len());
                lines.push(build_hex_row(start, &data[start..end], self.highlight_byte));
            }
        } else {
            lines.push(Line::from(Span::styled(
                " (read error — sector unreadable)",
                Style::default().fg(Color::Red),
            )));
        }

        frame.render_widget(Paragraph::new(lines), inner);
    }
}

// ── Row builder ───────────────────────────────────────────────────────────────

/// Build a single hex-dump row as a coloured ratatui `Line`.
/// `start` = byte offset of this row within the sector.
/// `highlight` = byte offset within the sector to render highlighted.
fn build_hex_row(start: usize, chunk: &[u8], highlight: Option<usize>) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::with_capacity(2 + chunk.len() * 2 + 18);
    let count = chunk.len();

    // Offset column.
    spans.push(Span::styled(
        format!("{start:08X}  "),
        Style::default().fg(Color::DarkGray),
    ));

    // Hex columns — two groups of 8 separated by an extra space.
    for (i, &byte) in chunk.iter().enumerate() {
        if i == 8 {
            spans.push(Span::raw("  "));
        }
        let abs = start + i;
        let style = if highlight == Some(abs) {
            Style::default()
                .bg(Color::Yellow)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD)
        } else {
            byte_style(byte)
        };
        spans.push(Span::styled(format!("{byte:02X} "), style));
    }

    // Padding for the short last row.
    if count < 16 {
        let missing = 16 - count;
        let pad = missing * 3 + if count < 8 { 2 } else { 0 };
        spans.push(Span::raw(" ".repeat(pad)));
    }

    // ASCII column.
    spans.push(Span::styled(" │", Style::default().fg(Color::DarkGray)));
    for (i, &byte) in chunk.iter().enumerate() {
        let abs = start + i;
        let ch = if byte.is_ascii_graphic() || byte == b' ' {
            byte as char
        } else {
            '.'
        };
        let style = if highlight == Some(abs) {
            Style::default()
                .bg(Color::Yellow)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD)
        } else if byte == 0x00 {
            Style::default().fg(Color::DarkGray)
        } else if byte.is_ascii_graphic() {
            Style::default().fg(Color::Green)
        } else {
            Style::default().fg(Color::White)
        };
        spans.push(Span::styled(ch.to_string(), style));
    }
    for _ in count..16 {
        spans.push(Span::raw(" "));
    }
    spans.push(Span::styled("│", Style::default().fg(Color::DarkGray)));

    Line::from(spans)
}

/// Assign a display colour to a byte value.
fn byte_style(byte: u8) -> Style {
    if byte == 0x00 {
        Style::default().fg(Color::DarkGray) // null — dim
    } else if byte == 0xFF {
        Style::default().fg(Color::Red) // 0xFF — often erased flash
    } else if byte.is_ascii_graphic() || byte == b' ' {
        Style::default().fg(Color::Green) // printable ASCII
    } else {
        Style::default().fg(Color::White) // other binary
    }
}

// ── Sector annotation ─────────────────────────────────────────────────────────

/// Identify common on-disk structures from the first bytes of a sector.
/// Returns a short human-readable label, or `None` if unrecognised.
fn detect_sector_type(data: &[u8], lba: u64) -> Option<&'static str> {
    let len = data.len();
    if len < 4 {
        return None;
    }

    // MBR — only match at LBA 0 to avoid false positives on data sectors.
    if lba == 0 && len >= 512 && data[510] == 0x55 && data[511] == 0xAA {
        return if len > 450 && data[450] == 0xEE {
            Some("Protective MBR (GPT disk)")
        } else {
            Some("MBR — Master Boot Record")
        };
    }
    // GPT header.
    if len >= 8 && &data[0..8] == b"EFI PART" {
        return Some("GPT Header");
    }
    // NTFS Volume Boot Record.
    if len >= 11 && &data[3..11] == b"NTFS    " {
        return Some("NTFS Volume Boot Record");
    }
    // FAT32 Boot Sector.
    if len >= 90 && &data[82..90] == b"FAT32   " {
        return Some("FAT32 Boot Sector");
    }
    // FAT16 Boot Sector.
    if len >= 62 && &data[54..62] == b"FAT16   " {
        return Some("FAT16 Boot Sector");
    }
    // FAT12 Boot Sector.
    if len >= 62 && &data[54..62] == b"FAT12   " {
        return Some("FAT12 Boot Sector");
    }
    // ext2/3/4 Superblock — magic 0x53EF at offset 56 within the superblock.
    if len >= 58 && data[56] == 0x53 && data[57] == 0xEF {
        return Some("ext2/3/4 Superblock");
    }
    // SQLite Database.
    if len >= 16 && &data[0..16] == b"SQLite format 3\0" {
        return Some("SQLite Database");
    }
    // OLE2 Compound Document (DOC/XLS/PPT/PST/MDB).
    if len >= 8 && data[0..8] == [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1] {
        return Some("OLE2 Compound Doc (DOC/XLS/PPT/PST)");
    }
    // RIFF container (WAV/AVI) — checked before MZ to avoid misidentifying.
    if len >= 12 && &data[0..4] == b"RIFF" {
        return if &data[8..12] == b"WAVE" {
            Some("WAV Audio (RIFF)")
        } else if &data[8..12] == b"AVI " {
            Some("AVI Video (RIFF)")
        } else {
            Some("RIFF Container")
        };
    }
    // Windows PE / DOS Executable.
    if len >= 2 && &data[0..2] == b"MZ" {
        return Some("Windows PE / DOS Executable");
    }
    // ZIP / Office Open XML.
    if len >= 4 && &data[0..4] == b"PK\x03\x04" {
        return Some("ZIP / DOCX / XLSX / PPTX");
    }
    // RAR Archive.
    if len >= 6 && &data[0..6] == b"Rar!\x1a\x07" {
        return Some("RAR Archive");
    }
    // 7-Zip Archive.
    if len >= 6 && data[0..6] == [0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C] {
        return Some("7-Zip Archive");
    }
    // JPEG.
    if len >= 3 && data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF {
        return Some("JPEG Image");
    }
    // PNG.
    if len >= 8 && &data[0..8] == b"\x89PNG\r\n\x1a\n" {
        return Some("PNG Image");
    }
    // PDF.
    if len >= 4 && &data[0..4] == b"%PDF" {
        return Some("PDF Document");
    }
    // GIF.
    if len >= 6 && (&data[0..6] == b"GIF87a" || &data[0..6] == b"GIF89a") {
        return Some("GIF Image");
    }
    // OGG Media.
    if len >= 4 && &data[0..4] == b"OggS" {
        return Some("OGG Media Page");
    }
    // FLAC Audio.
    if len >= 4 && &data[0..4] == b"fLaC" {
        return Some("FLAC Audio");
    }
    // MP3 with ID3 tag.
    if len >= 3 && &data[0..3] == b"ID3" {
        return Some("MP3 Audio (ID3)");
    }
    // Matroska / MKV.
    if len >= 4 && data[0..4] == [0x1A, 0x45, 0xDF, 0xA3] {
        return Some("Matroska / MKV");
    }
    // Windows Event Log (EVTX): "ELFFILE\0"
    if len >= 7 && &data[0..7] == b"ELFFILE" {
        return Some("Windows Event Log (EVTX)");
    }
    // Outlook PST / OST.
    if len >= 4 && &data[0..4] == b"!BDN" {
        return Some("Outlook PST/OST");
    }

    None
}

#[cfg(test)]
#[path = "hex_viewer_tests.rs"]
mod tests;

//! Screen 7 — Sector Hex Viewer: browse raw device sectors with a classic
//! hex-dump layout.

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

// ── State ─────────────────────────────────────────────────────────────────────

pub struct HexViewerState {
    pub device: Option<Arc<dyn BlockDevice>>,
    /// Sector currently displayed.
    pub current_lba: u64,
    /// Text field for jump-to-LBA input.
    pub lba_input: String,
    /// Whether `lba_input` is active.
    pub editing: bool,
    /// Raw bytes of the current sector (up to 512 displayed).
    pub data: Option<Vec<u8>>,
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
            lba_input: String::new(),
            editing: false,
            data: None,
        }
    }

    /// Attach a device and reset to LBA 0.
    pub fn set_device(&mut self, device: Arc<dyn BlockDevice>) {
        self.device = Some(device);
        self.current_lba = 0;
        self.lba_input.clear();
        self.editing = false;
        self.data = None;
        self.load_sector();
    }

    /// Read `sector_size` bytes from `current_lba * sector_size` into `self.data`.
    pub fn load_sector(&mut self) {
        let device = match &self.device {
            Some(d) => Arc::clone(d),
            None => return,
        };
        let ss = device.sector_size() as usize;
        let offset = self.current_lba * device.sector_size() as u64;
        let mut buf = AlignedBuffer::new(ss, ss);
        match device.read_at(offset, &mut buf) {
            Ok(n) => {
                self.data = Some(buf.as_slice()[..n].to_vec());
            }
            Err(_) => {
                self.data = None;
            }
        }
    }

    pub fn handle_key(&mut self, code: KeyCode, _modifiers: KeyModifiers) {
        if self.editing {
            match code {
                KeyCode::Enter => {
                    if let Ok(lba) = self.lba_input.trim().parse::<u64>() {
                        self.current_lba = lba;
                        self.load_sector();
                    }
                    self.editing = false;
                }
                KeyCode::Esc => {
                    self.editing = false;
                }
                KeyCode::Backspace => {
                    self.lba_input.pop();
                }
                KeyCode::Char(c) if c.is_ascii_digit() => {
                    self.lba_input.push(c);
                }
                _ => {}
            }
            return;
        }

        match code {
            KeyCode::Up => {
                self.current_lba = self.current_lba.saturating_sub(1);
                self.load_sector();
            }
            KeyCode::Down => {
                self.current_lba = self.current_lba.saturating_add(1);
                self.load_sector();
            }
            KeyCode::Char('g') => {
                self.lba_input.clear();
                self.editing = true;
            }
            _ => {}
        }
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        // Auto-load on first render if device is set but no data yet.
        if self.device.is_some() && self.data.is_none() && !self.editing {
            self.load_sector();
        }

        let outer = Block::default()
            .borders(Borders::ALL)
            .title(" Hex Viewer — ↑/↓: sector  g: jump to LBA ");
        let inner = outer.inner(area);
        frame.render_widget(outer, area);

        if self.device.is_none() {
            frame.render_widget(Paragraph::new(" No device selected."), inner);
            return;
        }

        let device = self.device.as_ref().unwrap();
        let offset_base = self.current_lba * device.sector_size() as u64;
        let header_text = format!(" LBA: {}  Offset: {offset_base:#x}", self.current_lba);

        let mut lines: Vec<Line> = Vec::new();
        lines.push(Line::from(Span::styled(
            header_text,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )));

        if self.editing {
            lines.push(Line::from(Span::styled(
                format!(" Jump to LBA: {}_", self.lba_input),
                Style::default().fg(Color::Yellow),
            )));
        }

        if let Some(data) = &self.data {
            // Display up to 512 bytes in classic hex dump format: 16 bytes per row.
            let display_len = data.len().min(512);
            for row in 0..display_len.div_ceil(16) {
                let start = row * 16;
                let end = (start + 16).min(display_len);
                let chunk = &data[start..end];

                // Offset column: 8-char hex offset within sector.
                let mut line_str = format!("{:08X}  ", start);

                // Hex columns: two groups of 8, separated by extra space.
                for (i, &byte) in chunk.iter().enumerate() {
                    if i == 8 {
                        line_str.push(' ');
                    }
                    line_str.push_str(&format!("{byte:02X} "));
                }
                // Pad short last row.
                let count = chunk.len();
                if count < 16 {
                    let missing = 16 - count;
                    // Each missing byte = 3 chars; extra space if we're in first half
                    let pad = missing * 3 + if count < 8 { 1 } else { 0 };
                    for _ in 0..pad {
                        line_str.push(' ');
                    }
                }

                // ASCII column.
                line_str.push(' ');
                line_str.push('|');
                for &byte in chunk {
                    if byte.is_ascii_graphic() || byte == b' ' {
                        line_str.push(byte as char);
                    } else {
                        line_str.push('.');
                    }
                }
                // Pad short last row ASCII.
                for _ in count..16 {
                    line_str.push(' ');
                }
                line_str.push('|');

                lines.push(Line::from(Span::raw(line_str)));
            }
        } else {
            lines.push(Line::from(" (read error or empty sector)"));
        }

        frame.render_widget(Paragraph::new(lines), inner);
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
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
        assert!(s.editing);
    }
}

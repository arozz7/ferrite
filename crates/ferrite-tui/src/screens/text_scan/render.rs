//! Render methods for [`TextScanState`].

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, List, ListItem, ListState, Paragraph},
    Frame,
};

use ferrite_textcarver::TextKind;

use super::{ScanStatus, TextScanState};

// ── Color palette ─────────────────────────────────────────────────────────────

fn kind_color(kind: TextKind) -> Color {
    match kind {
        TextKind::Php => Color::Magenta,
        TextKind::Script => Color::Cyan,
        TextKind::Json => Color::Yellow,
        TextKind::Yaml => Color::Blue,
        TextKind::Markup => Color::Green,
        TextKind::Sql => Color::Red,
        TextKind::CSource => Color::LightBlue,
        TextKind::Markdown => Color::LightGreen,
        TextKind::Generic => Color::Gray,
    }
}

fn quality_color(pct: u8) -> Color {
    if pct >= 90 {
        Color::Green
    } else if pct >= 70 {
        Color::Yellow
    } else {
        Color::Red
    }
}

// ── Centered popup ────────────────────────────────────────────────────────────

fn centered_rect(pct_w: u16, pct_h: u16, r: Rect) -> Rect {
    let vpad = (100 - pct_h) / 2;
    let hpad = (100 - pct_w) / 2;
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(vpad),
            Constraint::Percentage(pct_h),
            Constraint::Percentage(vpad),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(hpad),
            Constraint::Percentage(pct_w),
            Constraint::Percentage(hpad),
        ])
        .split(vert[1])[1]
}

// ── Main render ───────────────────────────────────────────────────────────────

impl TextScanState {
    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        let status_label = match self.status {
            ScanStatus::Idle => " idle ",
            ScanStatus::Running => " scanning… ",
            ScanStatus::Done => " done ",
            ScanStatus::Cancelled => " cancelled ",
            ScanStatus::Error => " error ",
        };

        let filter_label = match self.filter_kind {
            None => "all".to_string(),
            Some(k) => k.label().to_string(),
        };

        let title = format!(
            " Text Scan [{status_label}] — filter: {filter_label}  s:scan  c:cancel  e:export  o:output  0-8:filter "
        );

        let outer = Block::default().borders(Borders::ALL).title(title);
        let inner = outer.inner(area);
        frame.render_widget(outer, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // output dir bar
                Constraint::Length(1), // progress / block count bar
                Constraint::Min(0),    // block list
                Constraint::Length(1), // filter hints / export status
            ])
            .split(inner);

        self.render_output_bar(frame, rows[0]);
        self.render_progress_bar(frame, rows[1]);
        self.render_block_list(frame, rows[2]);
        self.render_status_bar(frame, rows[3]);

        if self.show_consent {
            self.render_consent_dialog(frame, area);
        }
    }

    fn render_output_bar(&self, frame: &mut Frame, area: Rect) {
        let (text, style) = if self.editing_dir {
            (
                format!(" Output dir: {}\u{2588}", self.output_dir),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        } else if self.output_dir.is_empty() {
            (
                " Output dir: ./ferrite_text/  (o to set)".to_string(),
                Style::default().fg(Color::DarkGray),
            )
        } else {
            (
                format!(" Output dir: {}  (o to edit)", self.output_dir),
                Style::default().fg(Color::Green),
            )
        };
        frame.render_widget(Paragraph::new(Line::from(Span::styled(text, style))), area);
    }

    fn render_progress_bar(&self, frame: &mut Frame, area: Rect) {
        match (&self.progress, self.status) {
            (Some(p), ScanStatus::Running) if p.bytes_total > 0 => {
                let ratio = (p.bytes_done as f64 / p.bytes_total as f64).min(1.0);
                let elapsed = self
                    .scan_start
                    .map(|t| t.elapsed().as_secs_f64())
                    .unwrap_or(0.0);
                let label = format!(
                    " {} blocks  {}/{} MiB  {:.0}s",
                    p.blocks_found,
                    p.bytes_done / (1024 * 1024),
                    p.bytes_total / (1024 * 1024),
                    elapsed,
                );
                frame.render_widget(
                    Gauge::default()
                        .gauge_style(Style::default().fg(Color::Cyan).bg(Color::DarkGray))
                        .ratio(ratio)
                        .label(label),
                    area,
                );
            }
            _ => {
                let (count, color) = if self.blocks.is_empty() {
                    (" No blocks found yet".to_string(), Color::DarkGray)
                } else {
                    (format!(" {} total blocks", self.blocks.len()), Color::White)
                };
                frame.render_widget(
                    Paragraph::new(Line::from(Span::styled(count, Style::default().fg(color)))),
                    area,
                );
            }
        }
    }

    fn render_block_list(&mut self, frame: &mut Frame, area: Rect) {
        let focused_style = Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD);
        let normal_style = Style::default().fg(Color::White);

        let items: Vec<ListItem> = self
            .filtered
            .iter()
            .enumerate()
            .map(|(display_idx, &block_idx)| {
                let block = &self.blocks[block_idx];
                let sel = display_idx == self.block_sel;
                let kind_style = Style::default()
                    .fg(kind_color(block.kind))
                    .add_modifier(if sel {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    });
                let qual_style = Style::default()
                    .fg(quality_color(block.quality))
                    .add_modifier(if sel {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    });
                let row_style = if sel { focused_style } else { normal_style };

                let size_str = format_size(block.length);
                let line = Line::from(vec![
                    Span::styled(format!("{:08X}  ", block.byte_offset), row_style),
                    Span::styled(format!("{:>8}  ", size_str), row_style),
                    Span::styled(format!("{:<6}  ", block.extension), kind_style),
                    Span::styled(format!("{:>3}%  ", block.quality), qual_style),
                    Span::styled(block.preview.clone(), row_style),
                ]);
                ListItem::new(line)
            })
            .collect();

        if items.is_empty() {
            let msg = match self.status {
                ScanStatus::Idle => " Press s to start scanning for text blocks.",
                ScanStatus::Running => " Scanning…",
                ScanStatus::Done => " No text blocks found.",
                ScanStatus::Cancelled => " Scan cancelled.",
                ScanStatus::Error => " Scan error — see title bar.",
            };
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    msg,
                    Style::default().fg(Color::DarkGray),
                ))),
                area,
            );
            return;
        }

        // Update page size for PgUp/PgDn.
        self.blocks_page_size = area.height.saturating_sub(1) as usize;

        let mut list_state = ListState::default();
        list_state.select(Some(
            self.block_sel.min(self.filtered.len().saturating_sub(1)),
        ));

        frame.render_stateful_widget(
            List::new(items)
                .highlight_style(Style::default().add_modifier(Modifier::BOLD))
                .highlight_symbol("> "),
            area,
            &mut list_state,
        );
    }

    fn render_status_bar(&self, frame: &mut Frame, area: Rect) {
        let text = if let Some(ref status) = self.export_status {
            Span::styled(format!(" {status}"), Style::default().fg(Color::Green))
        } else if self.status == ScanStatus::Error && !self.error_msg.is_empty() {
            Span::styled(
                format!(" Error: {}", self.error_msg),
                Style::default().fg(Color::Red),
            )
        } else {
            Span::styled(
                " 0:all  1:php  2:script  3:json  4:yaml  5:markup  6:sql  7:csrc  8:md",
                Style::default().fg(Color::DarkGray),
            )
        };
        frame.render_widget(Paragraph::new(Line::from(text)), area);
    }

    fn render_consent_dialog(&self, frame: &mut Frame, area: Rect) {
        let popup_area = centered_rect(70, 60, area);
        frame.render_widget(Clear, popup_area);

        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Text Block Scanner — Consent ")
            .title_style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
            .border_style(Style::default().fg(Color::Yellow));

        let inner = block.inner(popup_area);
        frame.render_widget(block, popup_area);

        let lines = vec![
            Line::from(""),
            Line::from(Span::styled(
                "  This scanner identifies contiguous text regions in",
                Style::default().fg(Color::White),
            )),
            Line::from(Span::styled(
                "  the raw device stream.  Please note:",
                Style::default().fg(Color::White),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "  • Results are variable quality — some blocks may be",
                Style::default().fg(Color::Yellow),
            )),
            Line::from(Span::styled(
                "    partial files or merged fragments.",
                Style::default().fg(Color::Yellow),
            )),
            Line::from(Span::styled(
                "  • Binary files with long ASCII strings (EXE, SQLite)",
                Style::default().fg(Color::Yellow),
            )),
            Line::from(Span::styled(
                "    may produce false-positive hits.",
                Style::default().fg(Color::Yellow),
            )),
            Line::from(Span::styled(
                "  • No data is written until you press 'e' to export.",
                Style::default().fg(Color::White),
            )),
            Line::from(Span::styled(
                "  • Only UTF-8 / ASCII text is recognised.",
                Style::default().fg(Color::White),
            )),
            Line::from(Span::styled(
                "  • Prefer Tab 4 (Files) or Tab 7 (Quick Recover)",
                Style::default().fg(Color::White),
            )),
            Line::from(Span::styled(
                "    when filesystem metadata is available.",
                Style::default().fg(Color::White),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "  Press  y / Enter  to start scanning",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                "  Press  any other key  to cancel",
                Style::default().fg(Color::DarkGray),
            )),
        ];

        frame.render_widget(Paragraph::new(lines), inner);
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn format_size(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    if bytes >= MIB {
        format!("{:.1}M", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.0}K", bytes as f64 / KIB as f64)
    } else {
        format!("{bytes}B")
    }
}

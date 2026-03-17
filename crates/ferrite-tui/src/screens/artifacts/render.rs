//! Render methods for [`ArtifactsState`].

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, List, ListItem, ListState, Paragraph},
    Frame,
};

use ferrite_artifact::ArtifactKind;

use super::{ArtifactsState, ScanStatus};

// ── Color palette ─────────────────────────────────────────────────────────────

fn kind_color(kind: ArtifactKind) -> Color {
    match kind {
        ArtifactKind::Email => Color::Cyan,
        ArtifactKind::Url => Color::Green,
        ArtifactKind::CreditCard => Color::Red,
        ArtifactKind::Iban => Color::Yellow,
        ArtifactKind::WindowsPath => Color::Blue,
        ArtifactKind::Ssn => Color::Magenta,
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

impl ArtifactsState {
    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        let status_label = match self.status {
            ScanStatus::Idle => " idle ",
            ScanStatus::Running => " scanning… ",
            ScanStatus::Done => " done ",
            ScanStatus::Error => " error ",
        };

        let filter_label = match self.filter_kind {
            None => "all".to_string(),
            Some(k) => k.label().to_string(),
        };

        let title = format!(
            " Artifacts [{status_label}] — filter: {filter_label}  s:scan  c:cancel  e:export  o:output  0-6:filter "
        );

        let outer = Block::default().borders(Borders::ALL).title(title);
        let inner = outer.inner(area);
        frame.render_widget(outer, area);

        // Layout: output dir + progress bar + hit list + status line.
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // output dir bar
                Constraint::Length(1), // progress / hit count bar
                Constraint::Min(0),    // hit list
                Constraint::Length(1), // export status / filter hint
            ])
            .split(inner);

        self.render_output_bar(frame, rows[0]);
        self.render_progress_bar(frame, rows[1]);
        self.render_hit_list(frame, rows[2]);
        self.render_status_bar(frame, rows[3]);

        // Consent dialog overlay.
        if self.show_consent {
            self.render_consent_dialog(frame, area);
        }
    }

    fn render_output_bar(&self, frame: &mut Frame, area: Rect) {
        let (text, style) = if self.editing_dir {
            (
                format!(" Output dir: {}\u{2588}", self.output_dir),
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            )
        } else if self.output_dir.is_empty() {
            (
                " Output dir: .\\  (o to set — CSV written to current dir)".to_string(),
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
                    " {} hits  {}/{} MiB  {:.0}s",
                    p.hits_found,
                    p.bytes_done / (1024 * 1024),
                    p.bytes_total / (1024 * 1024),
                    elapsed,
                );
                frame.render_widget(
                    Gauge::default()
                        .gauge_style(Style::default().fg(Color::Green).bg(Color::DarkGray))
                        .ratio(ratio)
                        .label(label),
                    area,
                );
            }
            _ => {
                let (count, color) = if self.hits.is_empty() {
                    (" No hits yet".to_string(), Color::DarkGray)
                } else {
                    (
                        format!(" {} total hits", self.hits.len()),
                        Color::White,
                    )
                };
                frame.render_widget(
                    Paragraph::new(Line::from(Span::styled(count, Style::default().fg(color)))),
                    area,
                );
            }
        }
    }

    fn render_hit_list(&mut self, frame: &mut Frame, area: Rect) {
        let focused_style = Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD);
        let normal_style = Style::default().fg(Color::White);

        let items: Vec<ListItem> = self
            .filtered
            .iter()
            .enumerate()
            .map(|(display_idx, &hit_idx)| {
                let hit = &self.hits[hit_idx];
                let sel = display_idx == self.hit_sel;
                let kind_style = Style::default()
                    .fg(kind_color(hit.kind))
                    .add_modifier(if sel { Modifier::BOLD } else { Modifier::empty() });
                let row_style = if sel { focused_style } else { normal_style };

                let line = Line::from(vec![
                    Span::styled(format!("{:>12x}  ", hit.byte_offset), row_style),
                    Span::styled(hit.kind.short_label(), kind_style),
                    Span::styled(format!("  {}", hit.value), row_style),
                ]);
                ListItem::new(line)
            })
            .collect();

        if items.is_empty() {
            let msg = match self.status {
                ScanStatus::Idle => " Press s to start scanning for artifacts.",
                ScanStatus::Running => " Scanning…",
                ScanStatus::Done => " No artifacts found.",
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
        self.hits_page_size = area.height.saturating_sub(1) as usize;

        let mut list_state = ListState::default();
        list_state.select(Some(self.hit_sel));
        frame.render_stateful_widget(List::new(items), area, &mut list_state);
    }

    fn render_status_bar(&self, frame: &mut Frame, area: Rect) {
        let text = if let Some(ref msg) = self.export_status {
            Span::styled(format!(" {msg}"), Style::default().fg(Color::Green))
        } else if self.status == ScanStatus::Error && !self.error_msg.is_empty() {
            Span::styled(
                format!(" Error: {}", self.error_msg),
                Style::default().fg(Color::Red),
            )
        } else {
            Span::styled(
                " 0:all  1:email  2:url  3:CC  4:IBAN  5:path  6:SSN",
                Style::default().fg(Color::DarkGray),
            )
        };
        frame.render_widget(Paragraph::new(Line::from(text)), area);
    }

    // ── Consent dialog ────────────────────────────────────────────────────────

    pub(super) fn render_consent_dialog(&self, frame: &mut Frame, area: Rect) {
        let popup = centered_rect(60, 40, area);
        frame.render_widget(Clear, popup);

        let block = Block::default()
            .borders(Borders::ALL)
            .title(" ⚠  Privacy Notice ")
            .border_style(Style::default().fg(Color::Yellow));
        let inner = block.inner(popup);
        frame.render_widget(block, popup);

        let lines = vec![
            Line::from(""),
            Line::from(Span::styled(
                "  This scanner will search the device for:",
                Style::default().fg(Color::White),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "    • Email addresses",
                Style::default().fg(Color::Cyan),
            )),
            Line::from(Span::styled(
                "    • URLs",
                Style::default().fg(Color::Green),
            )),
            Line::from(Span::styled(
                "    • Credit card numbers (masked — last 4 digits only)",
                Style::default().fg(Color::Red),
            )),
            Line::from(Span::styled(
                "    • IBANs, Windows paths, US SSNs",
                Style::default().fg(Color::Yellow),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "  Results are stored in memory and optionally",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(Span::styled(
                "  exported to CSV.  No data leaves this machine.",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "  Proceed?   y = yes   any other key = cancel",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )),
        ];

        frame.render_widget(Paragraph::new(lines), inner);
    }
}

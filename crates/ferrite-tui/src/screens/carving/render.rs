//! Render methods for [`CarvingState`] — split from `mod.rs` to keep file sizes under limit.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, List, ListItem, ListState, Paragraph},
    Frame,
};

use super::{
    fmt_bytes, preview, CarveFocus, CarveStatus, CarvingState, ExtractProgress, ExtractionSummary,
    HitStatus, ScanRangeField,
};

impl CarvingState {
    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        let status_label = match &self.status {
            CarveStatus::Idle => " idle ",
            CarveStatus::Running if self.backpressure_paused => " scan paused — queue full ",
            CarveStatus::Running => " scanning… ",
            CarveStatus::Pausing => " pausing… ",
            CarveStatus::Paused => " PAUSED ",
            CarveStatus::Done if self.meta_index_building => " done · building filename index… ",
            CarveStatus::Done => " done ",
            CarveStatus::Error(e) => {
                let msg = format!(" Carving Error: {e}");
                frame.render_widget(
                    Paragraph::new(msg)
                        .style(Style::default().fg(Color::Red))
                        .block(Block::default().borders(Borders::ALL).title(" Carving ")),
                    area,
                );
                return;
            }
        };

        let title = format!(
            " Carving [{status_label}] — Space: toggle  s: scan  e: extract  v: preview  ←/→: switch panel "
        );
        let outer = Block::default().borders(Borders::ALL).title(title);
        let inner = outer.inner(area);
        frame.render_widget(outer, area);

        // Split vertically: output dir + scan range + disk/auto-extract + FS index + main panels.
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(0),
            ])
            .split(inner);

        self.render_output_dir_bar(frame, rows[0]);
        self.render_scan_range_bar(frame, rows[1]);
        self.render_disk_auto_bar(frame, rows[2]);
        self.render_fs_index_bar(frame, rows[3]);

        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(rows[4]);

        self.render_sig_panel(frame, cols[0]);
        self.render_hits_panel(frame, cols[1]);
    }

    fn render_output_dir_bar(&self, frame: &mut Frame, area: Rect) {
        let (dir_text, dir_style) = if self.editing_dir {
            (
                format!(" Output Dir: {}█", self.output_dir),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        } else if self.output_dir.is_empty() {
            (
                " Output Dir: carved\\  (o to set — files go to current dir/carved/)".to_string(),
                Style::default().fg(Color::DarkGray),
            )
        } else {
            (
                format!(" Output Dir: {}  (o to edit)", self.output_dir),
                Style::default().fg(Color::Green),
            )
        };

        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(dir_text, dir_style))),
            area,
        );
    }

    fn render_scan_range_bar(&mut self, frame: &mut Frame, area: Rect) {
        let start_active = self.scan_range_field == ScanRangeField::Start;
        let end_active = self.scan_range_field == ScanRangeField::End;
        let label_style = Style::default().fg(Color::DarkGray);
        let active_style = Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD);
        let inactive_style = Style::default().fg(Color::DarkGray);
        let start_display = if self.scan_start_lba_str.is_empty() {
            format!("[{:>10}]", "0")
        } else if start_active {
            format!("[{:>10}\u{2588}]", self.scan_start_lba_str)
        } else {
            format!("[{:>10}]", self.scan_start_lba_str)
        };
        let end_display = if self.scan_end_lba_str.is_empty() {
            format!("[{:>10}]", "")
        } else if end_active {
            format!("[{:>10}\u{2588}]", self.scan_end_lba_str)
        } else {
            format!("[{:>10}]", self.scan_end_lba_str)
        };
        let spans = vec![
            Span::styled(" Range  From LBA: ", label_style),
            Span::styled(
                start_display,
                if start_active {
                    active_style
                } else {
                    inactive_style
                },
            ),
            Span::styled("  To LBA: ", label_style),
            Span::styled(
                end_display,
                if end_active {
                    active_style
                } else {
                    inactive_style
                },
            ),
            Span::styled("  (empty = full device)   [: from  ]: to", label_style),
        ];
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    fn render_sig_panel(&self, frame: &mut Frame, area: Rect) {
        let focused = self.focus == CarveFocus::Signatures;
        let title_style = if focused {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(" Signatures (Space=toggle) ", title_style));

        let items: Vec<ListItem> = self
            .sig_list
            .iter()
            .map(|e| {
                let check = if e.enabled { "[✓]" } else { "[ ]" };
                let style = if e.enabled {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default().fg(Color::DarkGray)
                };
                ListItem::new(format!("{check} {}", e.sig.name)).style(style)
            })
            .collect();

        let list = List::new(items)
            .block(block)
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

        let mut ls =
            ListState::default().with_selected(if focused { Some(self.sig_sel) } else { None });
        frame.render_stateful_widget(list, area, &mut ls);
    }

    fn render_hits_panel(&mut self, frame: &mut Frame, area: Rect) {
        let focused = self.focus == CarveFocus::Hits;
        let title_style = if focused {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let hit_count = self.hits.len();
        let total_count = self.total_hits_found;
        let hits_label = if total_count > hit_count {
            format!("{hit_count} of {total_count} total")
        } else {
            format!("{hit_count}")
        };
        let sel_count = self.hits.iter().filter(|e| e.selected).count();
        let done_count = self
            .hits
            .iter()
            .filter(|e| matches!(e.status, HitStatus::Ok { .. } | HitStatus::Truncated { .. }))
            .count();
        let auto_str = if self.auto_extract {
            " [AUTO-EXTRACT]"
        } else {
            ""
        };
        let title_str = if self.extract_progress.is_some() {
            format!(" Hits ({hits_label}){auto_str}  {done_count} extracted — p: pause  c: cancel ")
        } else if sel_count > 0 {
            format!(" Hits ({hits_label}){auto_str}  {sel_count} selected — Space: toggle  a: all  e: extract  E: extract selected  PgUp/Dn: page  Home/End: jump ")
        } else {
            format!(
                " Hits ({hits_label}){auto_str} — Space: select  a: all  E: extract selected  x: auto-extract  PgUp/Dn: page  Home/End: jump "
            )
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(title_str, title_style));

        // Scanning with no hits yet → show full progress bar filling the panel.
        if matches!(
            self.status,
            CarveStatus::Running | CarveStatus::Pausing | CarveStatus::Paused
        ) && self.hits.is_empty()
        {
            let inner = block.inner(area);
            frame.render_widget(block, area);
            self.render_progress(frame, inner);
            return;
        }

        // Empty, not scanning.
        if self.hits.is_empty() {
            let msg = match &self.status {
                CarveStatus::Idle => " Enable signatures and press s to scan.",
                _ => " No hits found.",
            };
            frame.render_widget(Paragraph::new(msg).block(block), area);
            return;
        }

        // Render border, then split inner area.
        let inner = block.inner(area);
        frame.render_widget(block, area);

        // While scanning WITH hits present, show a compact progress row.
        let after_scan = if matches!(
            self.status,
            CarveStatus::Running | CarveStatus::Pausing | CarveStatus::Paused
        ) {
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Min(0)])
                .split(inner);
            self.render_compact_scan_progress(frame, rows[0]);
            rows[1]
        } else {
            inner
        };

        // Extraction progress / summary bar.
        let after_extract = if let Some(ep) = &self.extract_progress {
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(4), Constraint::Min(0)])
                .split(after_scan);
            self.render_extract_progress(frame, rows[0], ep);
            rows[1]
        } else if let Some(summary) = &self.extract_summary {
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(3), Constraint::Min(0)])
                .split(after_scan);
            self.render_extraction_summary(frame, rows[0], summary);
            rows[1]
        } else {
            after_scan
        };

        // Preview panel split (when enabled and hits present).
        let list_area = if self.show_preview {
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
                .split(after_extract);
            if let Some(p) = &self.current_preview {
                preview::render_preview(frame, rows[0], p, self.color_cap);
            } else {
                let msg = if self.preview_loading {
                    " Loading preview…"
                } else {
                    " No preview available."
                };
                frame.render_widget(
                    Paragraph::new(msg)
                        .style(Style::default().fg(Color::DarkGray))
                        .block(Block::default().borders(Borders::ALL).title(" Preview ")),
                    rows[0],
                );
            }
            rows[1]
        } else {
            after_extract
        };

        // Update page size so PgUp/PgDn cover exactly the visible rows.
        self.hits_page_size = (list_area.height as usize).max(1);

        let items: Vec<ListItem> = self
            .hits
            .iter()
            .map(|entry| {
                let check = if entry.selected {
                    Span::styled(
                        "[✓] ",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    Span::raw("[ ] ")
                };
                let status_span = match &entry.status {
                    HitStatus::Unextracted => Span::raw(""),
                    HitStatus::Queued => {
                        Span::styled(" [queued]", Style::default().fg(Color::DarkGray))
                    }
                    HitStatus::Extracting => {
                        Span::styled(" [extracting…]", Style::default().fg(Color::Yellow))
                    }
                    HitStatus::Ok { bytes } => Span::styled(
                        format!(" [OK {}]", fmt_bytes(*bytes)),
                        Style::default().fg(Color::Green),
                    ),
                    HitStatus::Truncated { bytes } => Span::styled(
                        format!(" [TRUNC {}]", fmt_bytes(*bytes)),
                        Style::default().fg(Color::Red),
                    ),
                };
                let orig_name = self
                    .meta_index
                    .as_ref()
                    .and_then(|idx| idx.lookup(entry.hit.byte_offset))
                    .map(|m| format!(" ({})", m.name));
                let label = format!(
                    "{} @ {:#x}{}",
                    entry.hit.signature.name,
                    entry.hit.byte_offset,
                    orig_name.as_deref().unwrap_or("")
                );
                ListItem::new(Line::from(vec![check, Span::raw(label), status_span]))
            })
            .collect();

        let list =
            List::new(items).highlight_style(Style::default().add_modifier(Modifier::REVERSED));

        let mut ls =
            ListState::default().with_selected(if focused { Some(self.hit_sel) } else { None });
        frame.render_stateful_widget(list, list_area, &mut ls);
    }

    fn render_extract_progress(&self, frame: &mut Frame, area: Rect, ep: &ExtractProgress) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Length(1)])
            .split(area);

        let ratio = if ep.total > 0 {
            (ep.done as f64 / ep.total as f64).clamp(0.0, 1.0)
        } else {
            0.0
        };

        // Spinner driven by elapsed time for a live "pulse" indicator.
        const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        let spin = SPINNER[(ep.start.elapsed().as_millis() / 100) as usize % SPINNER.len()];

        let cancelling = self
            .extract_cancel
            .load(std::sync::atomic::Ordering::Relaxed);
        let paused = self
            .extract_pause
            .load(std::sync::atomic::Ordering::Relaxed);
        let label = if cancelling {
            format!("Cancelling… {}/{}", ep.done, ep.total)
        } else if paused {
            format!("⏸ PAUSED  {}/{}", ep.done, ep.total)
        } else if ep.last_name.is_empty() {
            format!("{spin} Starting…  0/{}", ep.total)
        } else {
            format!("{spin} {}/{} — {}", ep.done, ep.total, ep.last_name)
        };

        let gauge_color = if cancelling {
            Color::Red
        } else if paused {
            Color::Yellow
        } else {
            Color::Cyan
        };
        let bar_title = if cancelling {
            " Extracting [cancelling…] "
        } else if paused {
            " Extracting [PAUSED — p to resume] "
        } else {
            " Extracting (p: pause  c: cancel) "
        };
        let gauge = Gauge::default()
            .block(Block::default().borders(Borders::ALL).title(bar_title))
            .ratio(ratio)
            .label(label)
            .gauge_style(Style::default().fg(gauge_color));
        frame.render_widget(gauge, chunks[0]);

        // Stats line: bytes written, rate, elapsed, ETA.
        let elapsed = ep.start.elapsed().as_secs_f64();
        let rate_bps = if elapsed > 0.0 && ep.done > 0 {
            ep.total_bytes as f64 / elapsed
        } else {
            0.0
        };
        let rate_str = if rate_bps > 0.0 {
            format!("{:.1} MB/s", rate_bps / (1024.0 * 1024.0))
        } else {
            "—".to_string()
        };
        let elapsed_secs = elapsed as u64;
        let elapsed_str = format!(
            "{:02}:{:02}:{:02}",
            elapsed_secs / 3600,
            (elapsed_secs % 3600) / 60,
            elapsed_secs % 60,
        );
        let eta_str = if ep.done > 0 && ep.done < ep.total {
            let secs_per_file = elapsed / ep.done as f64;
            let eta_secs = ((ep.total - ep.done) as f64 * secs_per_file) as u64;
            if eta_secs >= 3600 {
                format!("ETA {:02}h{:02}m", eta_secs / 3600, (eta_secs % 3600) / 60)
            } else {
                format!("ETA {:02}m{:02}s", eta_secs / 60, eta_secs % 60)
            }
        } else {
            String::new()
        };
        let stats = format!(
            " {} written   {}   Elapsed {}   {}",
            fmt_bytes(ep.total_bytes),
            rate_str,
            elapsed_str,
            eta_str,
        );
        frame.render_widget(
            Paragraph::new(stats).style(Style::default().fg(Color::DarkGray)),
            chunks[1],
        );
    }

    fn render_extraction_summary(&self, frame: &mut Frame, area: Rect, s: &ExtractionSummary) {
        let elapsed = s.elapsed_secs;
        let rate_str = if elapsed > 0.0 && s.total_bytes > 0 {
            let bps = s.total_bytes as f64 / elapsed;
            format!("{:.1} MB/s avg", bps / (1024.0 * 1024.0))
        } else {
            String::new()
        };
        let elapsed_secs = elapsed as u64;
        let elapsed_str = format!(
            "{:02}:{:02}:{:02}",
            elapsed_secs / 3600,
            (elapsed_secs % 3600) / 60,
            elapsed_secs % 60,
        );

        let ok_span = Span::styled(
            format!("  ✓ {} extracted", s.succeeded),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        );
        let trunc_span = if s.truncated > 0 {
            Span::styled(
                format!("   ⚠ {} truncated", s.truncated),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::raw("")
        };
        let fail_span = if s.failed > 0 {
            Span::styled(
                format!("   ✗ {} failed", s.failed),
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            )
        } else {
            Span::raw("")
        };
        let meta_span = Span::styled(
            format!(
                "   │  {}  │  {}{}",
                fmt_bytes(s.total_bytes),
                elapsed_str,
                if rate_str.is_empty() {
                    String::new()
                } else {
                    format!("  {rate_str}")
                }
            ),
            Style::default().fg(Color::DarkGray),
        );
        let dismiss_span = Span::styled("   (d to dismiss)", Style::default().fg(Color::DarkGray));

        let title_style = Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD);
        // In auto-extract mode the summary accumulates across all batches;
        // label it accordingly so the running total is not mistaken for a
        // one-shot result.
        let title = if self.auto_extract {
            " Auto-Extract Session Total "
        } else {
            " Extraction Complete "
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(title, title_style))
            .border_style(Style::default().fg(Color::Green));

        let inner = block.inner(area);
        frame.render_widget(block, area);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                ok_span,
                trunc_span,
                fail_span,
                meta_span,
                dismiss_span,
            ])),
            inner,
        );
    }
}

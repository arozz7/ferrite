//! Progress-related render methods for [`CarvingState`] — split from `render.rs`.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph},
    Frame,
};

use super::{fmt_bytes, CarveStatus, CarvingState};

impl CarvingState {
    pub(super) fn render_disk_auto_bar(&self, frame: &mut Frame, area: Rect) {
        let auto_str = if self.auto_extract {
            Span::styled(
                "  x: auto-extract [ON] ",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled(
                "  x: auto-extract [off] ",
                Style::default().fg(Color::DarkGray),
            )
        };

        let disk_span = match self.disk_avail_bytes {
            None => Span::styled(" Disk: —", Style::default().fg(Color::DarkGray)),
            Some(avail) => {
                const LOW_THRESHOLD: u64 = 10 * 1024 * 1024 * 1024; // 10 GiB
                if avail < LOW_THRESHOLD {
                    Span::styled(
                        format!(" ⚠ Disk free: {} (LOW)", fmt_bytes(avail)),
                        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                    )
                } else {
                    Span::styled(
                        format!(" Disk free: {}", fmt_bytes(avail)),
                        Style::default().fg(Color::DarkGray),
                    )
                }
            }
        };

        frame.render_widget(Paragraph::new(Line::from(vec![disk_span, auto_str])), area);
    }

    pub(super) fn render_compact_scan_progress(&self, frame: &mut Frame, area: Rect) {
        let paused = self.status == CarveStatus::Paused;
        let (frac, hits_found) = if let Some(p) = &self.scan_progress {
            let window = p.scan_end.saturating_sub(p.scan_start);
            let scanned_in_window = p.bytes_scanned.saturating_sub(p.scan_start);
            let f = if window > 0 {
                (scanned_in_window as f64 / window as f64).clamp(0.0, 1.0)
            } else if p.device_size > 0 {
                (p.bytes_scanned as f64 / p.device_size as f64).clamp(0.0, 1.0)
            } else {
                0.0
            };
            (f, p.hits_found)
        } else {
            (0.0, 0)
        };

        let rate_str = if let (Some(p), Some(start)) = (&self.scan_progress, &self.scan_start) {
            let wall_elapsed = start.elapsed().as_secs_f64();
            let paused_secs = self.paused_elapsed.as_secs_f64()
                + self.paused_since.map_or(0.0, |t| t.elapsed().as_secs_f64());
            let active_secs = (wall_elapsed - paused_secs).max(0.001);
            if paused {
                "paused".to_string()
            } else {
                let bps = p.bytes_scanned as f64 / active_secs;
                if bps > 0.0 {
                    format!("{:.1} MB/s", bps / (1024.0 * 1024.0))
                } else {
                    "—".to_string()
                }
            }
        } else {
            "—".to_string()
        };

        let status_pfx = if paused {
            "⏸ "
        } else if self.backpressure_paused {
            "⏳ "
        } else {
            ""
        };
        let queue_hint = if self.backpressure_paused {
            format!(
                "   queue: {} (waiting for extraction)",
                self.auto_extract_queue.len()
            )
        } else {
            String::new()
        };
        let line = format!(
            " {status_pfx}Scan {:.1}%   {} hits found   {rate_str}{queue_hint}",
            frac * 100.0,
            hits_found,
        );
        let style = if paused {
            Style::default().fg(Color::Yellow)
        } else if self.backpressure_paused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        frame.render_widget(Paragraph::new(line).style(style), area);
    }

    pub(super) fn render_progress(&self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // progress bar
                Constraint::Length(1), // stats line
                Constraint::Min(0),    // padding
            ])
            .split(area);

        let (ratio, bar_label) = if let Some(p) = &self.scan_progress {
            let window = p.scan_end.saturating_sub(p.scan_start);
            let scanned_in_window = p.bytes_scanned.saturating_sub(p.scan_start);
            let frac = if window > 0 {
                (scanned_in_window as f64 / window as f64).clamp(0.0, 1.0)
            } else if p.device_size > 0 {
                (p.bytes_scanned as f64 / p.device_size as f64).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let window_str = if window > 0 && window != p.device_size {
                format!(
                    " (window {} / {})",
                    fmt_bytes(scanned_in_window),
                    fmt_bytes(window)
                )
            } else {
                format!(" / {}", fmt_bytes(p.device_size))
            };
            let label = format!(
                "{:.1}%  —  {}{}  —  {} hits",
                frac * 100.0,
                fmt_bytes(p.bytes_scanned),
                window_str,
                p.hits_found,
            );
            (frac, label)
        } else {
            (0.0, "Starting\u{2026}".to_string())
        };

        let paused = self.status == CarveStatus::Paused;
        let gauge_color = if paused { Color::Yellow } else { Color::Green };
        let bar_title = if paused {
            " Progress  [PAUSED — p to resume] "
        } else {
            " Progress "
        };
        let gauge = Gauge::default()
            .block(Block::default().borders(Borders::ALL).title(bar_title))
            .ratio(ratio)
            .label(bar_label)
            .gauge_style(Style::default().fg(gauge_color));
        frame.render_widget(gauge, chunks[0]);

        // Rate + ETA stats line.
        if let (Some(p), Some(start)) = (&self.scan_progress, &self.scan_start) {
            let wall_elapsed = start.elapsed().as_secs_f64();
            // Subtract cumulative paused time so rate and ETA reflect active scan
            // time only.  When the scan is currently paused, also include the
            // duration of the current pause interval.
            let paused_secs = self.paused_elapsed.as_secs_f64()
                + self.paused_since.map_or(0.0, |t| t.elapsed().as_secs_f64());
            let active_secs = (wall_elapsed - paused_secs).max(0.001);
            let is_paused = self.paused_since.is_some();
            let rate_bps = if active_secs > 0.0 {
                p.bytes_scanned as f64 / active_secs
            } else {
                0.0
            };
            let rate_str = if is_paused {
                "— (paused)".to_string()
            } else if rate_bps > 0.0 {
                format!("{:.1} MB/s", rate_bps / (1024.0 * 1024.0))
            } else {
                "—".to_string()
            };
            let eta_str = if is_paused {
                String::new()
            } else if rate_bps > 0.0 && p.scan_end > p.bytes_scanned {
                let remaining = (p.scan_end.saturating_sub(p.bytes_scanned)) as f64 / rate_bps;
                let secs = remaining as u64;
                if secs >= 3600 {
                    format!("ETA {:02}h{:02}m", secs / 3600, (secs % 3600) / 60)
                } else if secs >= 60 {
                    format!("ETA {:02}m{:02}s", secs / 60, secs % 60)
                } else {
                    format!("ETA {secs}s")
                }
            } else {
                String::new()
            };
            let elapsed_secs = active_secs as u64;
            let elapsed_str = format!(
                "Elapsed {:02}:{:02}:{:02}",
                elapsed_secs / 3600,
                (elapsed_secs % 3600) / 60,
                elapsed_secs % 60,
            );
            let stats = format!(" {rate_str}   {elapsed_str}   {eta_str}");
            frame.render_widget(
                Paragraph::new(stats).style(Style::default().fg(Color::DarkGray)),
                chunks[1],
            );
        }
    }
}

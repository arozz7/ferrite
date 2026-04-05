//! Progress-related render methods for [`CarvingState`] — split from `render.rs`.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph},
    Frame,
};

use ferrite_core::ThermalEvent;

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

        let skip_trunc_span = if self.skip_truncated {
            Span::styled(
                " t: skip-trunc [ON]",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled(" t: skip-trunc [off]", Style::default().fg(Color::DarkGray))
        };

        let skip_corrupt_span = if self.skip_corrupt {
            Span::styled(
                " C: skip-corrupt [ON]",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled(
                " C: skip-corrupt [off]",
                Style::default().fg(Color::DarkGray),
            )
        };

        frame.render_widget(
            Paragraph::new(Line::from(vec![
                disk_span,
                auto_str,
                skip_trunc_span,
                skip_corrupt_span,
            ])),
            area,
        );
    }

    /// Dedicated row showing whether original filenames and folder paths are
    /// available for carved files.  Lives on its own line so it is never
    /// obscured by the disk / auto-extract status.
    pub(super) fn render_fs_index_bar(&self, frame: &mut Frame, area: Rect) {
        let label = Span::styled(" Folder structure: ", Style::default().fg(Color::DarkGray));

        let status = if self.meta_index_building {
            Span::styled(
                "⌛ building filesystem index…",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        } else if let Some(idx) = &self.meta_index {
            if idx.is_empty() {
                Span::styled(
                    "✗ No filesystem metadata found — files will be named by byte offset",
                    Style::default().fg(Color::DarkGray),
                )
            } else {
                Span::styled(
                    format!(
                        "✓ {} paths indexed — extracted files will use original names & folder structure",
                        idx.len()
                    ),
                    Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
                )
            }
        } else {
            Span::styled(
                "○ Not scanned yet — original folder structure unavailable",
                Style::default().fg(Color::DarkGray),
            )
        };

        frame.render_widget(Paragraph::new(Line::from(vec![label, status])), area);
    }

    pub(super) fn render_compact_scan_progress(&self, frame: &mut Frame, area: Rect) {
        let paused = self.status == CarveStatus::Paused;
        let (frac, hits_found) = if let Some(p) = &self.scan_progress {
            // Show progress relative to the full configured window (not just the
            // remaining portion after a resume), so a resumed scan starts at the
            // percentage already covered rather than jumping back to 0%.
            let window = p.scan_end.saturating_sub(self.scan_window_start);
            let covered = p.bytes_scanned.saturating_sub(self.scan_window_start);
            let f = if window > 0 {
                (covered as f64 / window as f64).clamp(0.0, 1.0)
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
                // Use bytes scanned in this session (not the absolute offset) so
                // the rate isn't inflated when resuming from a large offset.
                let bytes_this_session = p.bytes_scanned.saturating_sub(p.scan_start) as f64;
                let bps = bytes_this_session / active_secs;
                if bps > 0.0 {
                    format!("{:.1} MB/s", bps / (1024.0 * 1024.0))
                } else {
                    "—".to_string()
                }
            }
        } else {
            "—".to_string()
        };

        let thermal_active = matches!(
            self.thermal_event,
            Some(ThermalEvent::Paused) | Some(ThermalEvent::SpeedThrottle)
        );
        let status_pfx = if thermal_active {
            "🌡 "
        } else if paused {
            "⏸ "
        } else if self.backpressure_paused {
            "⏳ "
        } else {
            ""
        };
        let queue_hint = if self.backpressure_paused {
            let pending = self.hits.len().saturating_sub(self.next_auto_extract_idx);
            format!("   queue: {pending} (waiting for extraction)")
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

    /// Overall extraction progress gauge — shown when the scan is complete and
    /// there are hits to (or being) extracted.  Mirrors the scan progress bar
    /// in shape: a 3-row gauge + 1-row stats line.
    ///
    /// Provides: ratio, files written, rate (hits/sec), elapsed time, and ETA.
    pub(super) fn render_extraction_overview(&self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // gauge
                Constraint::Length(1), // stats line
                Constraint::Min(0),    // padding
            ])
            .split(area);

        let total = self.total_hits_scanned.max(1);
        let processed = self.hits_extracted_count.min(total);
        let ratio = (processed as f64 / total as f64).clamp(0.0, 1.0);
        let remaining = total.saturating_sub(processed);
        let done = processed == total;

        let gauge_label = format!(
            "{processed} / {total} processed  ({:.1}%)   {} files written",
            ratio * 100.0,
            self.files_written_count,
        );

        let gauge_color = if done { Color::Green } else { Color::Cyan };
        let bar_title = if done {
            " Extraction  [COMPLETE] "
        } else {
            " Extraction Progress "
        };
        let gauge = Gauge::default()
            .block(Block::default().borders(Borders::ALL).title(bar_title))
            .ratio(ratio)
            .label(gauge_label)
            .gauge_style(Style::default().fg(gauge_color));
        frame.render_widget(gauge, chunks[0]);

        // Rate, elapsed and ETA — only meaningful once extraction has started.
        if let Some(start) = self.extraction_start {
            let elapsed_secs = start.elapsed().as_secs_f64().max(0.001);
            let rate = processed as f64 / elapsed_secs; // hits/sec

            let rate_str = if rate >= 1.0 {
                format!("{:.1} hits/s", rate)
            } else if rate > 0.0 {
                format!("{:.2} hits/s", rate)
            } else {
                "—".to_string()
            };

            let elapsed_u = elapsed_secs as u64;
            let elapsed_str = format!(
                "Elapsed {:02}:{:02}:{:02}",
                elapsed_u / 3600,
                (elapsed_u % 3600) / 60,
                elapsed_u % 60,
            );

            let eta_str = if done {
                String::new()
            } else if rate > 0.0 {
                let eta_secs = (remaining as f64 / rate) as u64;
                if eta_secs >= 3600 {
                    format!("ETA ~{:02}h{:02}m", eta_secs / 3600, (eta_secs % 3600) / 60)
                } else if eta_secs >= 60 {
                    format!("ETA ~{:02}m{:02}s", eta_secs / 60, eta_secs % 60)
                } else {
                    format!("ETA ~{eta_secs}s")
                }
            } else {
                "ETA —".to_string()
            };

            let stats = format!(" {rate_str}   {elapsed_str}   {eta_str}");
            frame.render_widget(
                Paragraph::new(stats).style(Style::default().fg(Color::DarkGray)),
                chunks[1],
            );
        } else if !done {
            frame.render_widget(
                Paragraph::new(" Not started — press E to extract or x for auto-extract")
                    .style(Style::default().fg(Color::DarkGray)),
                chunks[1],
            );
        }
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
            let window = p.scan_end.saturating_sub(self.scan_window_start);
            let covered = p.bytes_scanned.saturating_sub(self.scan_window_start);
            let frac = if window > 0 {
                (covered as f64 / window as f64).clamp(0.0, 1.0)
            } else if p.device_size > 0 {
                (p.bytes_scanned as f64 / p.device_size as f64).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let window_str = if window > 0 && window != p.device_size {
                format!(" (window {} / {})", fmt_bytes(covered), fmt_bytes(window))
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
        let thermal_paused = matches!(
            self.thermal_event,
            Some(ThermalEvent::Paused) | Some(ThermalEvent::SpeedThrottle)
        );
        let gauge_color = if paused {
            Color::Yellow
        } else if thermal_paused {
            Color::LightYellow
        } else {
            Color::Green
        };
        let bar_title = if thermal_paused {
            " Progress  [⏸ THERMAL PAUSE — cooling down] "
        } else if paused {
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
            // Use bytes scanned in this session so the rate isn't inflated
            // when resuming from a large byte offset.
            let bytes_this_session = p.bytes_scanned.saturating_sub(p.scan_start) as f64;
            let rate_bps = if active_secs > 0.0 {
                bytes_this_session / active_secs
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

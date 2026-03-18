//! Render methods for [`ImagingState`] — split from `mod.rs` to keep file
//! sizes under the project hard limit (600 lines).

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Gauge, Paragraph},
    Frame,
};

use ferrite_imaging::mapfile::BlockStatus;

use super::{EditField, ImagingState, ImagingStatus};

impl ImagingState {
    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        let outer = Block::default()
            .borders(Borders::ALL)
            .title(" Imaging Engine ");
        let inner = outer.inner(area);
        frame.render_widget(outer, area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(12), // config fields + hint + resume line
                Constraint::Length(3),  // progress bar
                Constraint::Length(6),  // sector map
                Constraint::Min(0),     // stats / messages
            ])
            .split(inner);

        // ── Config fields ────────────────────────────────────────────────────
        let editing_dest = self.edit_field == Some(EditField::Dest);
        let editing_map = self.edit_field == Some(EditField::Mapfile);

        let dest_style = if editing_dest {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else if self.dest_path.is_empty() {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::Green)
        };
        let map_style = if editing_map {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };

        let source_label = self
            .device
            .as_ref()
            .map(|d| {
                let info = d.device_info();
                let size_gib = info.size_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
                match (&info.model, &info.serial) {
                    (Some(m), Some(s)) => {
                        let _ = s;
                        format!("{} — {} ({:.1} GiB)", info.path, m, size_gib)
                    }
                    (Some(m), None) => format!("{} — {} ({:.1} GiB)", info.path, m, size_gib),
                    _ => format!("{} ({:.1} GiB)", info.path, size_gib),
                }
            })
            .unwrap_or_else(|| "—".into());

        let config_text = vec![
            Line::from(format!(" Source  : {source_label}")),
            Line::from(vec![
                Span::raw(" Dest    : "),
                Span::styled(
                    if editing_dest {
                        format!("{}█", self.dest_path)
                    } else if self.dest_path.is_empty() {
                        "(not set — press d)  e.g. D:\\recovery\\disk.img".into()
                    } else {
                        self.dest_path.clone()
                    },
                    dest_style,
                ),
            ]),
            Line::from(vec![
                Span::raw(" Mapfile : "),
                Span::styled(
                    if editing_map {
                        format!("{}█", self.mapfile_path)
                    } else if self.mapfile_path.is_empty() {
                        "(none — progress won't be saved)".into()
                    } else {
                        self.mapfile_path.clone()
                    },
                    map_style,
                ),
            ]),
            Line::from(vec![
                Span::raw(" Resume  : "),
                if self.imaging_resumed {
                    Span::styled(
                        "YES — continuing from saved mapfile",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    Span::styled("NO — fresh start", Style::default().fg(Color::DarkGray))
                },
            ]),
            Line::from(vec![
                Span::raw(" Start   : "),
                Span::styled(
                    if self.edit_field == Some(EditField::StartLba) {
                        format!("{}█", self.start_lba_str)
                    } else if self.start_lba_str.is_empty() {
                        "(beginning)".into()
                    } else {
                        format!("LBA {}", self.start_lba_str)
                    },
                    if self.edit_field == Some(EditField::StartLba) {
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    },
                ),
            ]),
            Line::from(vec![
                Span::raw(" End     : "),
                Span::styled(
                    if self.edit_field == Some(EditField::EndLba) {
                        format!("{}█", self.end_lba_str)
                    } else if self.end_lba_str.is_empty() {
                        "(end of device)".into()
                    } else {
                        format!("LBA {}", self.end_lba_str)
                    },
                    if self.edit_field == Some(EditField::EndLba) {
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    },
                ),
            ]),
            Line::from(vec![
                Span::raw(" BlockSz : "),
                Span::styled(
                    if self.edit_field == Some(EditField::BlockSize) {
                        format!("{}█ KiB", self.block_size_str)
                    } else if self.block_size_str.is_empty() {
                        "(default 512 KiB)".into()
                    } else {
                        format!("{} KiB", self.block_size_str)
                    },
                    if self.edit_field == Some(EditField::BlockSize) {
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    },
                ),
            ]),
            Line::from(vec![
                Span::raw(" Reverse : "),
                Span::styled(
                    if self.reverse { "YES" } else { "NO" }.to_string(),
                    if self.reverse {
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    },
                ),
                Span::raw("  (r to toggle)"),
            ]),
            Line::from(Span::styled(
                " Dest is the output image file path, e.g. D:\\recovery\\disk.img  \
                 Mapfile saves progress so imaging can resume after interruption.",
                Style::default().fg(Color::DarkGray),
            )),
        ];
        frame.render_widget(
            Paragraph::new(config_text).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Configuration "),
            ),
            chunks[0],
        );

        // ── Drive mismatch confirmation prompt ───────────────────────────────
        if let ImagingStatus::ConfirmDriveMismatch {
            sidecar_serial,
            sidecar_model,
            sidecar_size,
            current_serial,
            current_model,
            current_size,
        } = &self.status
        {
            let fmt = |n: u64| -> String {
                const GIB: u64 = 1 << 30;
                const MIB: u64 = 1 << 20;
                if n >= GIB {
                    format!("{:.1} GiB", n as f64 / GIB as f64)
                } else if n >= MIB {
                    format!("{:.1} MiB", n as f64 / MIB as f64)
                } else {
                    format!("{n} B")
                }
            };
            let text = vec![
                Line::from(Span::styled(
                    " ⚠  Drive mismatch — this image was created from a different drive.",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::styled(" Expected : ", Style::default().fg(Color::DarkGray)),
                    Span::raw(format!(
                        "{} {}  ({})",
                        sidecar_model,
                        sidecar_serial,
                        fmt(*sidecar_size)
                    )),
                ]),
                Line::from(vec![
                    Span::styled(" Connected: ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        format!(
                            "{} {}  ({})",
                            current_model,
                            current_serial,
                            fmt(*current_size)
                        ),
                        Style::default().fg(Color::Red),
                    ),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    " Continuing will write into the existing image with the wrong source drive.",
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::styled(
                        " y",
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" — proceed anyway     "),
                    Span::styled(
                        " n / Esc",
                        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" — cancel"),
                ]),
            ];
            frame.render_widget(
                Paragraph::new(text).block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" ⚠ Drive Mismatch ")
                        .style(Style::default().fg(Color::Yellow)),
                ),
                chunks[1],
            );
            return;
        }

        // ── Progress bar ─────────────────────────────────────────────────────
        let ratio = self
            .latest
            .as_ref()
            .map(|u| u.fraction_done())
            .unwrap_or(0.0)
            .clamp(0.0, 1.0);

        let phase_label = self.latest.as_ref().map(|u| {
            use ferrite_imaging::ImagingPhase;
            match u.phase {
                ImagingPhase::Copy => "Copy",
                ImagingPhase::Trim => "Trim",
                ImagingPhase::Sweep => "Sweep",
                ImagingPhase::Scrape => "Scrape",
                ImagingPhase::Retry { attempt, max } => {
                    let _ = (attempt, max);
                    "Retry"
                }
                ImagingPhase::Complete => "Complete",
            }
        });

        let bar_label = match &self.status {
            ImagingStatus::Idle => "Not started — press s to start".into(),
            ImagingStatus::Running => {
                let phase = phase_label.unwrap_or("Copy");
                format!("{phase} — {:.1}%", ratio * 100.0)
            }
            ImagingStatus::Complete => "Complete ✓".into(),
            ImagingStatus::Cancelled => "Cancelled".into(),
            ImagingStatus::Error(e) => format!("Error: {e}"),
            ImagingStatus::ConfirmDriveMismatch { .. } => String::new(), // handled above
        };

        // Detect low read rate (> 0 but < 5 MB/s) for amber indicator.
        let is_low_rate = self
            .latest
            .as_ref()
            .map(|u| u.read_rate_bps > 0 && u.read_rate_bps < 5 * 1024 * 1024)
            .unwrap_or(false);

        let bar_style = match &self.status {
            ImagingStatus::Running if self.user_paused || self.thermal_paused => {
                Style::default().fg(Color::Yellow)
            }
            ImagingStatus::Running if is_low_rate => Style::default().fg(Color::Yellow),
            ImagingStatus::Running => Style::default().fg(Color::Green),
            ImagingStatus::Complete => Style::default().fg(Color::Green),
            ImagingStatus::Error(_) => Style::default().fg(Color::Red),
            _ => Style::default().fg(Color::DarkGray),
        };

        let bar_title = if self.thermal_paused {
            " Progress [⚠ THERMAL PAUSE] "
        } else if self.user_paused {
            " Progress [⏸ PAUSED — p to resume] "
        } else if is_low_rate {
            " Progress [⚠ LOW RATE] "
        } else {
            " Progress "
        };
        let gauge = Gauge::default()
            .block(Block::default().borders(Borders::ALL).title(bar_title))
            .ratio(ratio)
            .label(bar_label)
            .gauge_style(bar_style);
        frame.render_widget(gauge, chunks[1]);

        // ── Sector map ────────────────────────────────────────────────────────
        self.render_sector_map(frame, chunks[2]);

        // ── Stats ─────────────────────────────────────────────────────────────
        // ── Write-blocker status line ─────────────────────────────────────────
        let wb_line: Option<Line> =
            if self.status == ImagingStatus::Running || self.write_blocked.is_some() {
                match self.write_blocked {
                    None => Some(Line::from(Span::styled(
                        " Write-blocker: checking…",
                        Style::default().fg(Color::DarkGray),
                    ))),
                    Some(true) => Some(Line::from(Span::styled(
                        " Write-blocker: OK",
                        Style::default().fg(Color::Green),
                    ))),
                    Some(false) => Some(Line::from(Span::styled(
                        " Write-blocker: WARNING — not blocked!",
                        Style::default().fg(Color::Red),
                    ))),
                }
            } else {
                None
            };

        if let Some(u) = &self.latest {
            let elapsed = u.elapsed.as_secs();
            let rate_mbps = u.read_rate_bps as f64 / (1024.0 * 1024.0);

            let eta_str = if u.read_rate_bps > 0 && u.bytes_finished < u.device_size {
                let remaining = u.device_size - u.bytes_finished;
                let eta_secs = (remaining as f64 / u.read_rate_bps as f64) as u64;
                if eta_secs >= 3600 {
                    format!(
                        "  ETA {:02}h{:02}m",
                        eta_secs / 3600,
                        (eta_secs % 3600) / 60
                    )
                } else if eta_secs >= 60 {
                    format!("  ETA {:02}m{:02}s", eta_secs / 60, eta_secs % 60)
                } else {
                    format!("  ETA {eta_secs}s")
                }
            } else {
                String::new()
            };

            let temp_str = match (self.current_temp, self.thermal_paused) {
                (Some(t), true) => format!("  Temp: {t}°C ⚠ PAUSED (>55°C)"),
                (Some(t), false) => format!("  Temp: {t}°C"),
                (None, _) => String::new(),
            };

            // Rate line — amber with warning suffix when below 5 MB/s.
            let rate_line = if u.read_rate_bps == 0 {
                Line::from(format!(" Rate: —{eta_str}{temp_str}"))
            } else if is_low_rate {
                Line::from(vec![
                    Span::styled(
                        format!(" Rate: {rate_mbps:.1} MB/s ⚠ SLOW"),
                        Style::default().fg(Color::Yellow),
                    ),
                    Span::raw(format!("{eta_str}{temp_str}")),
                ])
            } else {
                Line::from(format!(" Rate: {rate_mbps:.1} MB/s{eta_str}{temp_str}"))
            };

            let stats = format!(
                " Finished: {}  Bad: {}  Non-tried: {}  Elapsed: {:02}:{:02}:{:02}",
                fmt_bytes(u.bytes_finished),
                fmt_bytes(u.bytes_bad),
                fmt_bytes(u.bytes_non_tried),
                elapsed / 3600,
                (elapsed % 3600) / 60,
                elapsed % 60,
            );
            let hash_line: Option<Line> = self.image_sha256.as_ref().map(|hash| {
                if self.imaging_resumed {
                    Line::from(vec![
                        Span::styled(" SHA-256: ", Style::default().fg(Color::Yellow)),
                        Span::styled(hash.as_str(), Style::default().fg(Color::Yellow)),
                        Span::styled(
                            "  ⚠ resumed — hash covers new data only",
                            Style::default().fg(Color::Yellow),
                        ),
                    ])
                } else {
                    Line::from(vec![
                        Span::styled(" SHA-256: ", Style::default().fg(Color::Green)),
                        Span::raw(hash.as_str()),
                    ])
                }
            });
            let mut text = Text::from(stats);
            text.push_line(rate_line);
            if let Some(hl) = hash_line {
                text.push_line(hl);
            }
            if let Some(wbl) = wb_line {
                text.push_line(wbl);
            }
            frame.render_widget(
                Paragraph::new(text)
                    .block(Block::default().borders(Borders::ALL).title(" Statistics ")),
                chunks[3],
            );
        } else {
            let base_msg = " Press s to start imaging, d to set destination path.";
            if let Some(wbl) = wb_line {
                let mut text = Text::from(base_msg);
                text.push_line(wbl);
                frame.render_widget(
                    Paragraph::new(text)
                        .block(Block::default().borders(Borders::ALL).title(" Statistics ")),
                    chunks[3],
                );
            } else {
                frame.render_widget(
                    Paragraph::new(base_msg)
                        .block(Block::default().borders(Borders::ALL).title(" Statistics ")),
                    chunks[3],
                );
            }
        }
    }

    fn render_sector_map(&self, frame: &mut Frame, area: Rect) {
        let legend = if area.width >= 90 {
            " Sector Map  \u{2588}\u{2588} Finished  \u{2591}\u{2591} Non-tried  \u{2592}\u{2592} Non-trim/scrape  \u{2588}\u{2588} Bad  \u{25b6} Current "
        } else if area.width >= 60 {
            " Sector Map  \u{2588}\u{2588} OK  \u{2591}\u{2591} Pending  \u{2592}\u{2592} Warn  \u{2588}\u{2588} Bad  \u{25b6} Pos "
        } else {
            " Sector Map "
        };
        let block = Block::default().borders(Borders::ALL).title(legend);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if inner.width == 0 || inner.height == 0 || self.sector_map.is_empty() {
            let msg = if self.status == ImagingStatus::Idle {
                " Start imaging to see the sector map."
            } else {
                " Waiting for sector map data…"
            };
            frame.render_widget(
                Paragraph::new(msg).style(Style::default().fg(Color::DarkGray)),
                inner,
            );
            return;
        }

        let device_size = self.latest.as_ref().map(|u| u.device_size).unwrap_or(0);
        if device_size == 0 {
            return;
        }

        let current_offset = self.latest.as_ref().map(|u| u.current_offset).unwrap_or(0);

        let total_cells = (inner.width as usize) * (inner.height as usize);
        if total_cells == 0 {
            return;
        }
        let bytes_per_cell = (device_size as usize).div_ceil(total_cells);

        // For each cell, find the dominant block status at that byte range.
        let mut cells: Vec<(char, Color)> = Vec::with_capacity(total_cells);

        for cell_idx in 0..total_cells {
            let cell_start = cell_idx as u64 * bytes_per_cell as u64;
            let cell_end = (cell_start + bytes_per_cell as u64).min(device_size);

            // Current position marker
            if current_offset >= cell_start && current_offset < cell_end {
                cells.push(('▶', Color::Cyan));
                continue;
            }

            // Find the dominant status in this cell range
            let mut counts = [0u64; 5]; // NonTried, NonTrimmed, NonScraped, BadSector, Finished
            for block in &self.sector_map {
                let block_end = block.pos + block.size;
                if block.pos >= cell_end || block_end <= cell_start {
                    continue;
                }
                let overlap_start = block.pos.max(cell_start);
                let overlap_end = block_end.min(cell_end);
                let overlap = overlap_end - overlap_start;
                match block.status {
                    BlockStatus::NonTried => counts[0] += overlap,
                    BlockStatus::NonTrimmed => counts[1] += overlap,
                    BlockStatus::NonScraped => counts[2] += overlap,
                    BlockStatus::BadSector => counts[3] += overlap,
                    BlockStatus::Finished => counts[4] += overlap,
                }
            }

            // Priority: BadSector > NonTrimmed > NonScraped > Finished > NonTried
            let (ch, color) = if counts[3] > 0 {
                ('█', Color::Red)
            } else if counts[1] > 0 || counts[2] > 0 {
                ('▒', Color::Yellow)
            } else if counts[4] > counts[0] {
                ('█', Color::Green)
            } else {
                ('░', Color::DarkGray)
            };
            cells.push((ch, color));
        }

        // Build lines
        let mut lines: Vec<Line> = Vec::new();
        let width = inner.width as usize;
        for row_start in (0..cells.len()).step_by(width) {
            let row_end = (row_start + width).min(cells.len());
            let spans: Vec<Span> = cells[row_start..row_end]
                .iter()
                .map(|(ch, color)| Span::styled(ch.to_string(), Style::default().fg(*color)))
                .collect();
            lines.push(Line::from(spans));
        }

        frame.render_widget(Paragraph::new(Text::from(lines)), inner);
    }
}

fn fmt_bytes(n: u64) -> String {
    const GIB: u64 = 1024 * 1024 * 1024;
    const MIB: u64 = 1024 * 1024;
    if n >= GIB {
        format!("{:.1}GiB", n as f64 / GIB as f64)
    } else if n >= MIB {
        format!("{:.1}MiB", n as f64 / MIB as f64)
    } else {
        format!("{n}B")
    }
}

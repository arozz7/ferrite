//! Screen 2 — Health Dashboard: S.M.A.R.T. summary and attribute table.

use std::sync::mpsc::{self, Receiver};

use crossterm::event::{KeyCode, KeyModifiers};
use ferrite_smart::{query_and_assess, CountThresholds, HealthVerdict, SmartData, SmartThresholds};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
    Frame,
};

// ── Types ─────────────────────────────────────────────────────────────────────

enum HealthMsg {
    Data(Box<SmartData>, HealthVerdict),
    Error(String),
}

enum HealthStatus {
    Idle,
    Loading,
    Loaded,
    Error(String),
}

// ── State ─────────────────────────────────────────────────────────────────────

pub struct HealthState {
    device_path: Option<String>,
    data: Option<SmartData>,
    verdict: Option<HealthVerdict>,
    attr_selected: usize,
    status: HealthStatus,
    rx: Option<Receiver<HealthMsg>>,
    /// Most recently received S.M.A.R.T. data — exposed for report generation.
    pub last_smart_data: Option<SmartData>,
}

impl Default for HealthState {
    fn default() -> Self {
        Self::new()
    }
}

impl HealthState {
    pub fn new() -> Self {
        Self {
            device_path: None,
            data: None,
            verdict: None,
            attr_selected: 0,
            status: HealthStatus::Idle,
            rx: None,
            last_smart_data: None,
        }
    }

    /// Called when the user selects a new device.
    pub fn set_device(&mut self, path: String) {
        self.device_path = Some(path);
        self.data = None;
        self.verdict = None;
        self.attr_selected = 0;
        self.status = HealthStatus::Idle;
        self.rx = None;
    }

    /// Drain the background S.M.A.R.T. channel.
    pub fn tick(&mut self) {
        let rx = match &self.rx {
            Some(r) => r,
            None => return,
        };
        match rx.try_recv() {
            Ok(HealthMsg::Data(data, verdict)) => {
                let smart = *data;
                self.last_smart_data = Some(smart.clone());
                self.data = Some(smart);
                self.verdict = Some(verdict);
                self.status = HealthStatus::Loaded;
                self.rx = None;
            }
            Ok(HealthMsg::Error(e)) => {
                self.status = HealthStatus::Error(e);
                self.rx = None;
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                self.rx = None;
            }
        }
    }

    pub fn handle_key(&mut self, code: KeyCode, _modifiers: KeyModifiers) {
        match code {
            KeyCode::Char('r') => self.start_query(),
            KeyCode::Up => {
                if self.attr_selected > 0 {
                    self.attr_selected -= 1;
                }
            }
            KeyCode::Down => {
                let max = self
                    .data
                    .as_ref()
                    .map(|d| d.attributes.len().saturating_sub(1))
                    .unwrap_or(0);
                if self.attr_selected < max {
                    self.attr_selected += 1;
                }
            }
            _ => {}
        }
    }

    fn start_query(&mut self) {
        let path = match &self.device_path {
            Some(p) => p.clone(),
            None => return,
        };
        self.status = HealthStatus::Loading;
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        std::thread::spawn(move || {
            let thresholds = SmartThresholds::default_config();
            match query_and_assess(&path, None, &thresholds) {
                Ok((data, verdict)) => {
                    let _ = tx.send(HealthMsg::Data(Box::new(data), verdict));
                }
                Err(e) => {
                    let _ = tx.send(HealthMsg::Error(e.to_string()));
                }
            }
        });
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        // Auto-query on first render once a device is set.
        if matches!(self.status, HealthStatus::Idle) && self.device_path.is_some() {
            self.start_query();
        }

        let outer = Block::default()
            .borders(Borders::ALL)
            .title(" Health Dashboard — press r to refresh ");

        match &self.status {
            HealthStatus::Idle => {
                frame.render_widget(Paragraph::new(" No device selected.").block(outer), area);
            }
            HealthStatus::Loading => {
                frame.render_widget(
                    Paragraph::new(" Querying S.M.A.R.T. data…").block(outer),
                    area,
                );
            }
            HealthStatus::Error(e) => {
                let install_hint = if cfg!(target_os = "windows") {
                    " • Run Ferrite as Administrator (right-click → Run as administrator)\n \
                     \n \
                     If smartctl is not installed:\n \
                     • winget install smartmontools\n \
                     • Or download from https://www.smartmontools.org/wiki/Download\n \
                     • Ensure C:\\Program Files\\smartmontools\\bin is on your PATH."
                } else {
                    " Install via your package manager:\n \
                     \n \
                       Debian/Ubuntu:  sudo apt install smartmontools\n \
                       Fedora/RHEL:    sudo dnf install smartmontools\n \
                       Arch:           sudo pacman -S smartmontools\n \
                     \n \
                     You may also need to run Ferrite with sudo."
                };
                let msg = format!(
                    " Error: {e}\n\n smartctl must be installed and on PATH.\n\n{install_hint}\n\n Press r to retry."
                );
                frame.render_widget(
                    Paragraph::new(msg)
                        .style(Style::default().fg(Color::Red))
                        .block(outer),
                    area,
                );
            }
            HealthStatus::Loaded => {
                render_health_loaded(
                    frame,
                    area,
                    outer,
                    self.data.as_ref().unwrap(),
                    self.verdict.as_ref().unwrap(),
                    self.attr_selected,
                );
            }
        }
    }
}

// ── Rendering helpers ─────────────────────────────────────────────────────────

fn render_health_loaded(
    frame: &mut Frame,
    area: Rect,
    outer: Block,
    data: &SmartData,
    verdict: &HealthVerdict,
    attr_sel: usize,
) {
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let thresholds = SmartThresholds::default_config();
    let reasons = verdict.reasons();

    // Summary height: base lines + 2 borders + one per reason, capped at half
    // the available height so the attribute table always gets some space.
    let base_lines: u16 = 8; // verdict + model + serial + fw + size/rotation + temp/hours + smart + separator
    let summary_height = (base_lines + 2 + reasons.len() as u16).min(area.height / 2 + 4);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(summary_height), Constraint::Min(0)])
        .split(inner);

    // ── Summary panel ────────────────────────────────────────────────────────
    let (verdict_style, verdict_label) = match verdict {
        HealthVerdict::Healthy => (Style::default().fg(Color::Green), "✓ HEALTHY"),
        HealthVerdict::Warning { .. } => (Style::default().fg(Color::Yellow), "⚠ WARNING"),
        HealthVerdict::Critical { .. } => (
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            "✗ CRITICAL",
        ),
    };

    // Capacity string: bytes → GiB / TiB
    let capacity_str = data.capacity_bytes.map(fmt_capacity).unwrap_or("—".into());

    // Rotation: Some(0) = SSD, Some(n) = n RPM, None = Unknown
    let rotation_str = match data.rotation_rate {
        Some(0) => "SSD".into(),
        Some(rpm) => format!("{rpm} RPM"),
        None => "—".into(),
    };

    // Bad sector count from ATA error log
    let bad_lba_note = if !data.bad_sector_lbas.is_empty() {
        format!(
            "  |  {} bad-sector LBAs in error log",
            data.bad_sector_lbas.len()
        )
    } else {
        String::new()
    };

    let mut summary_lines = vec![
        Line::from(vec![
            Span::raw(" Verdict : "),
            Span::styled(verdict_label, verdict_style.add_modifier(Modifier::BOLD)),
        ]),
        Line::from(format!(
            " Model   : {}",
            data.model.as_deref().unwrap_or("—")
        )),
        Line::from(format!(
            " Serial  : {}",
            data.serial.as_deref().unwrap_or("—")
        )),
        Line::from(format!(
            " Firmware: {}",
            data.firmware.as_deref().unwrap_or("—")
        )),
        Line::from(format!(" Size    : {capacity_str}  |  {rotation_str}")),
        Line::from(format!(
            " Temp    : {}   Power-on: {}{}",
            data.temperature_celsius
                .map(|t| format!("{t}°C"))
                .unwrap_or("—".into()),
            data.power_on_hours
                .map(|h| format!("{h} h"))
                .unwrap_or("—".into()),
            bad_lba_note,
        )),
        Line::from(format!(
            " SMART   : {}",
            if data.smart_passed {
                "PASSED"
            } else {
                "FAILED"
            }
        )),
    ];

    // USB/bridge note — when rotation_rate is unknown the SMART bridge may
    // report unreliable attribute values.
    if data.rotation_rate.is_none() {
        summary_lines.push(Line::from(Span::styled(
            "  \u{26a0} rotation rate unknown — USB bridge may report unreliable attributes",
            Style::default().fg(Color::DarkGray),
        )));
    }

    // Reasons — colour-coded by verdict level
    if !reasons.is_empty() {
        let reason_style = match verdict {
            HealthVerdict::Critical { .. } => Style::default().fg(Color::Red),
            HealthVerdict::Warning { .. } => Style::default().fg(Color::Yellow),
            HealthVerdict::Healthy => Style::default(),
        };
        for r in reasons {
            summary_lines.push(Line::from(Span::styled(format!("  • {r}"), reason_style)));
        }
    }

    frame.render_widget(
        Paragraph::new(summary_lines)
            .block(Block::default().borders(Borders::ALL).title(" Summary ")),
        chunks[0],
    );

    // ── Attributes table ─────────────────────────────────────────────────────
    if data.attributes.is_empty() {
        frame.render_widget(
            Paragraph::new(" No SMART attributes (NVMe device or smartctl version mismatch)")
                .block(Block::default().borders(Borders::ALL).title(" Attributes ")),
            chunks[1],
        );
        return;
    }

    // Column: Pf = prefailure indicator
    let header = Row::new([
        Cell::from("ID"),
        Cell::from("Attribute Name"),
        Cell::from("Pf"),
        Cell::from("Val"),
        Cell::from("Wst"),
        Cell::from("Thr"),
        Cell::from("Raw"),
        Cell::from("Status"),
    ])
    .style(
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    );

    let rows: Vec<Row> = data
        .attributes
        .iter()
        .map(|a| {
            // Determine row severity colour.
            let row_color = attr_row_color(a, &thresholds);

            // Prefailure column: "!" = predictive failure attr, "·" = informational
            let (pf_text, pf_style) = if a.prefailure {
                ("!", Style::default().fg(Color::Yellow))
            } else {
                ("·", Style::default().fg(Color::DarkGray))
            };

            // Status column: show why this attribute is flagged.
            let (status_text, status_style) = attr_status_cell(a, &thresholds);

            let base_style = row_color
                .map(|c| Style::default().fg(c))
                .unwrap_or_else(|| {
                    if a.prefailure {
                        Style::default()
                    } else {
                        Style::default().fg(Color::DarkGray)
                    }
                });

            Row::new([
                Cell::from(format!("{:>3}", a.id)).style(base_style),
                Cell::from(a.name.clone()).style(base_style),
                Cell::from(pf_text).style(pf_style),
                Cell::from(format!("{:>3}", a.value)).style(base_style),
                Cell::from(format!("{:>3}", a.worst)).style(base_style),
                Cell::from(format!("{:>3}", a.thresh)).style(base_style),
                Cell::from(format!("{}", a.raw_value)).style(base_style),
                Cell::from(status_text).style(status_style),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(4),  // ID
        Constraint::Min(24),    // Name
        Constraint::Length(3),  // Pf
        Constraint::Length(4),  // Val
        Constraint::Length(4),  // Wst
        Constraint::Length(4),  // Thr
        Constraint::Length(12), // Raw
        Constraint::Length(12), // Status
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Attributes  Pf=prefailure  Val/Wst/Thr=normalised(higher=better)  (↑/↓) "),
        )
        .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut ts = TableState::default().with_selected(Some(attr_sel));
    frame.render_stateful_widget(table, chunks[1], &mut ts);
}

// ── Attribute helpers ─────────────────────────────────────────────────────────

/// Row foreground colour based on severity:
/// - Red  : drive's own `value ≤ thresh` failure bit, OR our critical threshold crossed
/// - Yellow: our warning threshold crossed
/// - None  : healthy (caller applies default / dimming for informational attrs)
fn attr_row_color(a: &ferrite_smart::SmartAttribute, t: &SmartThresholds) -> Option<Color> {
    // Drive's own normalised failure threshold (manufacturer-set).
    if a.thresh > 0 && a.value <= a.thresh {
        return Some(Color::Red);
    }
    // Our custom raw-value thresholds for key attributes.
    match a.id {
        5 => count_color(a.raw_value, &t.reallocated_sectors),
        197 => count_color(a.raw_value, &t.pending_sectors),
        198 => count_color(a.raw_value, &t.uncorrectable_sectors),
        3 => {
            if a.raw_value >= t.spin_up_time_ms.critical_ms {
                Some(Color::Red)
            } else if a.raw_value >= t.spin_up_time_ms.warning_ms {
                Some(Color::Yellow)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn count_color(raw: u64, t: &CountThresholds) -> Option<Color> {
    if raw >= t.critical_count {
        Some(Color::Red)
    } else if raw >= t.warning_count {
        Some(Color::Yellow)
    } else {
        None
    }
}

/// Text and style for the Status column, explaining *why* the row is flagged.
fn attr_status_cell(a: &ferrite_smart::SmartAttribute, t: &SmartThresholds) -> (String, Style) {
    // 1. Drive's own threshold failure (highest priority).
    if a.thresh > 0 && a.value <= a.thresh {
        return (
            "val≤thr!".into(),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        );
    }
    // 2. Previously or currently failed per smartctl.
    if !a.when_failed.is_empty() {
        return (a.when_failed.clone(), Style::default().fg(Color::Red));
    }
    // 3. Our raw-value thresholds.
    let color = match a.id {
        5 => count_color(a.raw_value, &t.reallocated_sectors),
        197 => count_color(a.raw_value, &t.pending_sectors),
        198 => count_color(a.raw_value, &t.uncorrectable_sectors),
        3 => {
            if a.raw_value >= t.spin_up_time_ms.critical_ms {
                Some(Color::Red)
            } else if a.raw_value >= t.spin_up_time_ms.warning_ms {
                Some(Color::Yellow)
            } else {
                None
            }
        }
        _ => None,
    };
    match color {
        Some(Color::Red) => (
            "critical".into(),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Some(Color::Yellow) => ("warning".into(), Style::default().fg(Color::Yellow)),
        _ => (String::new(), Style::default()),
    }
}

/// Format capacity bytes as a human-readable string (GiB or TiB).
fn fmt_capacity(bytes: u64) -> String {
    const TIB: u64 = 1_099_511_627_776;
    const GIB: u64 = 1_073_741_824;
    if bytes >= TIB {
        format!("{:.2} TiB", bytes as f64 / TIB as f64)
    } else {
        format!("{:.1} GiB", bytes as f64 / GIB as f64)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_device_resets_state() {
        let mut s = HealthState::new();
        s.attr_selected = 5;
        s.set_device("/dev/sda".into());
        assert_eq!(s.attr_selected, 0);
        assert!(s.data.is_none());
    }

    #[test]
    fn attr_scroll_does_not_underflow() {
        let mut s = HealthState::new();
        s.handle_key(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(s.attr_selected, 0);
    }
}

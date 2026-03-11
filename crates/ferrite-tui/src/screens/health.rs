//! Screen 2 — Health Dashboard: S.M.A.R.T. summary and attribute table.

use std::sync::mpsc::{self, Receiver};

use crossterm::event::{KeyCode, KeyModifiers};
use ferrite_smart::{query_and_assess, HealthVerdict, SmartData, SmartThresholds};
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
                self.data = Some(*data);
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
                let msg = format!(
                    " Error: {e}\n\n smartctl must be installed and on PATH.\n Press r to retry."
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

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(7), Constraint::Min(0)])
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

    let reasons: Vec<Line> = match verdict {
        HealthVerdict::Healthy => vec![],
        HealthVerdict::Warning { reasons } | HealthVerdict::Critical { reasons } => reasons
            .iter()
            .map(|r| Line::from(format!("  • {r}")))
            .collect(),
    };

    let mut summary_lines = vec![
        Line::from(vec![
            Span::raw(" Verdict: "),
            Span::styled(verdict_label, verdict_style.add_modifier(Modifier::BOLD)),
        ]),
        Line::from(format!(" Model: {}", data.model.as_deref().unwrap_or("—"))),
        Line::from(format!(
            " Serial: {}",
            data.serial.as_deref().unwrap_or("—")
        )),
        Line::from(format!(
            " Temp: {}   Power-on hours: {}",
            data.temperature_celsius
                .map(|t| format!("{t}°C"))
                .unwrap_or("—".into()),
            data.power_on_hours
                .map(|h| format!("{h} h"))
                .unwrap_or("—".into()),
        )),
        Line::from(format!(
            " SMART self-test: {}",
            if data.smart_passed {
                "PASSED"
            } else {
                "FAILED"
            }
        )),
    ];
    summary_lines.extend(reasons);

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

    let header = Row::new([
        Cell::from("ID"),
        Cell::from("Name"),
        Cell::from("Val"),
        Cell::from("Wst"),
        Cell::from("Thr"),
        Cell::from("Raw"),
        Cell::from("Failed?"),
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
            let fail_style = if !a.when_failed.is_empty() {
                Style::default().fg(Color::Red)
            } else {
                Style::default()
            };
            Row::new([
                Cell::from(format!("{:>3}", a.id)),
                Cell::from(a.name.clone()),
                Cell::from(format!("{:>3}", a.value)),
                Cell::from(format!("{:>3}", a.worst)),
                Cell::from(format!("{:>3}", a.thresh)),
                Cell::from(format!("{}", a.raw_value)),
                Cell::from(a.when_failed.clone()).style(fail_style),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(4),
        Constraint::Min(28),
        Constraint::Length(4),
        Constraint::Length(4),
        Constraint::Length(4),
        Constraint::Length(12),
        Constraint::Length(10),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Attributes (↑/↓ to scroll) "),
        )
        .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut ts = TableState::default().with_selected(Some(attr_sel));
    frame.render_stateful_widget(table, chunks[1], &mut ts);
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

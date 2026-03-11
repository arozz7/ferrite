//! Screen 4 — Partition Analysis: read MBR/GPT tables and optionally scan for
//! lost partitions using filesystem-signature detection.

use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyModifiers};
use ferrite_blockdev::BlockDevice;
use ferrite_partition::{
    read_partition_table, scan, PartitionTable, PartitionTableKind, ScanOptions,
};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
    Frame,
};

// ── Types ─────────────────────────────────────────────────────────────────────

enum PartitionMsg {
    Table(PartitionTable),
    Error(String),
}

enum PartitionStatus {
    Idle,
    Reading,
    Scanning,
    Done,
    Error(String),
}

// ── State ─────────────────────────────────────────────────────────────────────

pub struct PartitionState {
    device: Option<Arc<dyn BlockDevice>>,
    table: Option<PartitionTable>,
    selected: usize,
    status: PartitionStatus,
    rx: Option<Receiver<PartitionMsg>>,
}

impl Default for PartitionState {
    fn default() -> Self {
        Self::new()
    }
}

impl PartitionState {
    pub fn new() -> Self {
        Self {
            device: None,
            table: None,
            selected: 0,
            status: PartitionStatus::Idle,
            rx: None,
        }
    }

    pub fn set_device(&mut self, device: Arc<dyn BlockDevice>) {
        self.device = Some(device);
        self.table = None;
        self.selected = 0;
        self.status = PartitionStatus::Idle;
        self.rx = None;
    }

    /// Drain the background partition channel.
    pub fn tick(&mut self) {
        let rx = match &self.rx {
            Some(r) => r,
            None => return,
        };
        match rx.try_recv() {
            Ok(PartitionMsg::Table(tbl)) => {
                self.table = Some(tbl);
                self.selected = 0;
                self.status = PartitionStatus::Done;
                self.rx = None;
            }
            Ok(PartitionMsg::Error(e)) => {
                self.status = PartitionStatus::Error(e);
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
            KeyCode::Up => {
                if self.selected > 0 {
                    self.selected -= 1;
                }
            }
            KeyCode::Down => {
                let max = self
                    .table
                    .as_ref()
                    .map(|t| t.entries.len().saturating_sub(1))
                    .unwrap_or(0);
                if self.selected < max {
                    self.selected += 1;
                }
            }
            KeyCode::Char('r') => self.start_read(),
            KeyCode::Char('s') => self.start_scan(),
            _ => {}
        }
    }

    fn start_read(&mut self) {
        let device = match &self.device {
            Some(d) => Arc::clone(d),
            None => return,
        };
        self.status = PartitionStatus::Reading;
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        std::thread::spawn(move || match read_partition_table(device.as_ref()) {
            Ok(tbl) => {
                let _ = tx.send(PartitionMsg::Table(tbl));
            }
            Err(e) => {
                let _ = tx.send(PartitionMsg::Error(e.to_string()));
            }
        });
    }

    fn start_scan(&mut self) {
        let device = match &self.device {
            Some(d) => Arc::clone(d),
            None => return,
        };
        self.status = PartitionStatus::Scanning;
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        std::thread::spawn(move || {
            let opts = ScanOptions::default();
            match scan(device.as_ref(), &opts) {
                Ok(hits) => {
                    let disk_lba = device.size() / device.sector_size() as u64;
                    let tbl =
                        ferrite_partition::from_scan_hits(&hits, disk_lba, device.sector_size());
                    let _ = tx.send(PartitionMsg::Table(tbl));
                }
                Err(e) => {
                    let _ = tx.send(PartitionMsg::Error(e.to_string()));
                }
            }
        });
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        // Auto-read on first render once a device is set.
        if matches!(self.status, PartitionStatus::Idle) && self.device.is_some() {
            self.start_read();
        }

        let title = match &self.status {
            PartitionStatus::Reading => " Partition Analysis — reading… ",
            PartitionStatus::Scanning => " Partition Analysis — scanning… ",
            _ => " Partition Analysis — r: read  s: scan ",
        };
        let outer = Block::default().borders(Borders::ALL).title(title);

        match &self.status {
            PartitionStatus::Idle => {
                frame.render_widget(Paragraph::new(" No device selected.").block(outer), area);
            }
            PartitionStatus::Reading | PartitionStatus::Scanning => {
                frame.render_widget(Paragraph::new(" Working…").block(outer), area);
            }
            PartitionStatus::Error(e) => {
                frame.render_widget(
                    Paragraph::new(format!(" Error: {e}\n Press r to retry."))
                        .style(Style::default().fg(Color::Red))
                        .block(outer),
                    area,
                );
            }
            PartitionStatus::Done => {
                let table_ref = self.table.as_ref().unwrap();
                render_partition_table(frame, area, outer, table_ref, self.selected);
            }
        }
    }
}

// ── Rendering helpers ─────────────────────────────────────────────────────────

fn render_partition_table(
    frame: &mut Frame,
    area: Rect,
    outer: Block,
    tbl: &PartitionTable,
    selected: usize,
) {
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let kind_str = match tbl.kind {
        PartitionTableKind::Mbr => "MBR",
        PartitionTableKind::Gpt => "GPT",
        PartitionTableKind::Recovered => "Recovered (signature scan)",
    };
    let summary = format!(
        " Table type: {kind_str}  Sector size: {} B  Partitions: {}",
        tbl.sector_size,
        tbl.entries.len()
    );

    // Split: 1 row summary + rest table.
    use ratatui::layout::{Constraint, Direction, Layout};
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(0)])
        .split(inner);

    frame.render_widget(Paragraph::new(summary), chunks[0]);

    if tbl.entries.is_empty() {
        frame.render_widget(
            Paragraph::new(" No partitions found. Try s to scan for lost partitions."),
            chunks[1],
        );
        return;
    }

    let header = Row::new([
        Cell::from(" #"),
        Cell::from("Type"),
        Cell::from("Start LBA"),
        Cell::from("End LBA"),
        Cell::from("Size"),
        Cell::from("Name / Info"),
    ])
    .style(
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    );

    let ss = tbl.sector_size;
    let rows: Vec<Row> = tbl
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let type_str = part_type_label(e);
            let size_bytes = e.size_bytes(ss);
            let name = e.name.as_deref().unwrap_or("—");
            Row::new([
                Cell::from(format!("{:>2}", i + 1)),
                Cell::from(type_str),
                Cell::from(format!("{}", e.start_lba)),
                Cell::from(format!("{}", e.end_lba)),
                Cell::from(fmt_bytes(size_bytes)),
                Cell::from(name.to_string()),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(3),
        Constraint::Length(10),
        Constraint::Length(12),
        Constraint::Length(12),
        Constraint::Length(10),
        Constraint::Min(16),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut ts = TableState::default().with_selected(Some(selected));
    frame.render_stateful_widget(table, chunks[1], &mut ts);
}

fn part_type_label(e: &ferrite_partition::PartitionEntry) -> String {
    use ferrite_partition::PartitionKind;
    match &e.kind {
        PartitionKind::Mbr { partition_type } => format!("MBR {partition_type:#04x}"),
        PartitionKind::Gpt { .. } => "GPT".into(),
        PartitionKind::Recovered { fs_type } => format!("{fs_type:?}"),
    }
}

fn fmt_bytes(n: u64) -> String {
    const GIB: u64 = 1024 * 1024 * 1024;
    const MIB: u64 = 1024 * 1024;
    if n >= GIB {
        format!("{:.1} GiB", n as f64 / GIB as f64)
    } else if n >= MIB {
        format!("{:.1} MiB", n as f64 / MIB as f64)
    } else {
        format!("{n} B")
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_does_not_underflow() {
        let mut s = PartitionState::new();
        s.handle_key(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(s.selected, 0);
    }
}

//! Screen 4 — Partition Analysis: read MBR/GPT tables and optionally scan for
//! lost partitions using filesystem-signature detection.

use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyModifiers};
use ferrite_blockdev::{BlockDevice, FileBlockDevice};
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
    /// Partition table was parsed but yielded no entries; auto-triggered scan.
    AutoScanning,
    Scanning,
    Done,
    Error(String),
}

// ── State ─────────────────────────────────────────────────────────────────────

pub struct PartitionState {
    device: Option<Arc<dyn BlockDevice>>,
    /// The most recently parsed partition table — exposed for report generation.
    pub table: Option<PartitionTable>,
    selected: usize,
    status: PartitionStatus,
    rx: Option<Receiver<PartitionMsg>>,
    /// Most recent export result message (success path or error).
    pub export_status: Option<String>,
    /// `true` while the Imaging tab is actively running on the same device.
    imaging_active: bool,
    /// Path to the partial image file when imaging is in progress and the file
    /// has already been written to.  `None` when imaging is not running or the
    /// file does not yet exist.
    fallback_image_path: Option<String>,
    /// `true` when the last `start_read()` used the partial image fallback
    /// instead of reading the physical device.
    used_image_fallback: bool,
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
            export_status: None,
            imaging_active: false,
            fallback_image_path: None,
            used_image_fallback: false,
        }
    }

    /// Called by the app each tick to propagate the current imaging state.
    ///
    /// `active` — whether imaging is actively running.
    /// `path`   — path to the partial image file when it already has data.
    pub fn set_imaging_context(&mut self, active: bool, path: Option<String>) {
        self.imaging_active = active;
        self.fallback_image_path = path;
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
                let is_read_result = matches!(self.status, PartitionStatus::Reading);
                let is_empty = tbl.entries.is_empty();
                self.table = Some(tbl);
                self.selected = 0;
                self.rx = None;
                // Auto-scan when the MBR/GPT parse returned no partitions.
                if is_read_result && is_empty {
                    self.status = PartitionStatus::AutoScanning;
                    self.start_scan();
                } else {
                    self.status = PartitionStatus::Done;
                }
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
            KeyCode::Char('w') => self.export_partition_table(),
            _ => {}
        }
    }

    fn export_partition_table(&mut self) {
        let device = match &self.device {
            Some(d) => Arc::clone(d),
            None => return,
        };
        let table = match &self.table {
            Some(t) => t.clone(),
            None => return,
        };
        let sector_size = device.sector_size() as u64;

        match table.kind {
            PartitionTableKind::Recovered => {
                let mut lines = vec!["Partition Table: Recovered (signature scan)".to_string()];
                lines.push(format!(
                    "Sector size: {} B  Partitions: {}",
                    table.sector_size,
                    table.entries.len()
                ));
                for e in &table.entries {
                    lines.push(format!(
                        "  #{}: start={} end={} size={} LBA",
                        e.index, e.start_lba, e.end_lba, e.size_lba
                    ));
                }
                match std::fs::write("ferrite-partition.txt", lines.join("\n")) {
                    Ok(_) => {
                        self.export_status = Some("Exported to ferrite-partition.txt".into());
                    }
                    Err(e) => {
                        self.export_status = Some(format!("Export failed: {e}"));
                    }
                }
            }
            PartitionTableKind::Mbr | PartitionTableKind::Gpt => {
                let sectors: u64 = if table.kind == PartitionTableKind::Mbr {
                    1
                } else {
                    34
                };
                let byte_count = (sectors * sector_size) as usize;
                let mut buf =
                    ferrite_blockdev::AlignedBuffer::new(byte_count, sector_size as usize);
                match device.read_at(0, &mut buf) {
                    Ok(n) => match std::fs::write("ferrite-partition.bin", &buf.as_slice()[..n]) {
                        Ok(_) => {
                            self.export_status = Some("Exported to ferrite-partition.bin".into());
                        }
                        Err(e) => {
                            self.export_status = Some(format!("Export failed: {e}"));
                        }
                    },
                    Err(e) => {
                        self.export_status = Some(format!("Read failed: {e}"));
                    }
                }
            }
        }
    }

    fn start_read(&mut self) {
        // Prefer reading from the partial image file when imaging is active and
        // the file already exists — avoids I/O contention with the imager.
        let device: Arc<dyn BlockDevice> = if let Some(ref path) = self.fallback_image_path {
            match FileBlockDevice::open(path) {
                Ok(fbd) if fbd.size() > 0 => {
                    self.used_image_fallback = true;
                    Arc::new(fbd)
                }
                _ => {
                    self.used_image_fallback = false;
                    match &self.device {
                        Some(d) => Arc::clone(d),
                        None => return,
                    }
                }
            }
        } else {
            self.used_image_fallback = false;
            match &self.device {
                Some(d) => Arc::clone(d),
                None => return,
            }
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

        let title: String = match &self.status {
            PartitionStatus::Reading => " Partition Analysis \u{2014} reading\u{2026} ".to_string(),
            PartitionStatus::AutoScanning => {
                " Partition Analysis \u{2014} no table found, scanning\u{2026} ".to_string()
            }
            PartitionStatus::Scanning => {
                " Partition Analysis \u{2014} scanning\u{2026} ".to_string()
            }
            PartitionStatus::Done if self.used_image_fallback => {
                " Partition Analysis (reading from partial image file) \u{2014} r: read  s: scan  w: export ".to_string()
            }
            PartitionStatus::Done => {
                " Partition Analysis \u{2014} r: read  s: scan  w: export ".to_string()
            }
            _ => " Partition Analysis \u{2014} r: read  s: scan ".to_string(),
        };
        let outer = Block::default().borders(Borders::ALL).title(title);

        match &self.status {
            PartitionStatus::Idle => {
                frame.render_widget(Paragraph::new(" No device selected.").block(outer), area);
            }
            PartitionStatus::Reading
            | PartitionStatus::AutoScanning
            | PartitionStatus::Scanning => {
                frame.render_widget(Paragraph::new(" Working\u{2026}").block(outer), area);
            }
            PartitionStatus::Error(e) => {
                let msg = format!(" Error: {e}\n Press r to retry.");
                frame.render_widget(
                    Paragraph::new(msg)
                        .style(Style::default().fg(Color::Red))
                        .block(outer),
                    area,
                );
            }
            PartitionStatus::Done => {
                let show_contention = self.imaging_active && !self.used_image_fallback;
                let table_ref = self.table.as_ref().unwrap();
                render_partition_table(
                    frame,
                    area,
                    outer,
                    table_ref,
                    self.selected,
                    self.export_status.as_deref(),
                    show_contention,
                );
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
    export_status: Option<&str>,
    show_contention_warning: bool,
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

    // Split: summary row + optional advisory rows + table + optional export status line.
    use ratatui::layout::{Constraint, Direction, Layout};
    let contention_height: u16 = if show_contention_warning { 1 } else { 0 };
    let note_height: u16 = if tbl.note.is_some() { 1 } else { 0 };
    let status_height: u16 = if export_status.is_some() { 1 } else { 0 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),                 // summary
            Constraint::Length(contention_height), // contention advisory
            Constraint::Length(note_height),       // table.note
            Constraint::Min(0),                    // partition table
            Constraint::Length(status_height),     // export status
        ])
        .split(inner);

    frame.render_widget(Paragraph::new(summary), chunks[0]);

    if show_contention_warning {
        frame.render_widget(
            Paragraph::new(
                " \u{26a0} Imaging in progress \u{2014} I/O contention possible. Using image file is recommended.",
            )
            .style(Style::default().fg(Color::Yellow)),
            chunks[1],
        );
    }

    if let Some(note) = &tbl.note {
        frame.render_widget(
            Paragraph::new(format!(" Note: {note}")).style(Style::default().fg(Color::Yellow)),
            chunks[2],
        );
    }

    if let Some(msg) = export_status {
        frame.render_widget(
            Paragraph::new(format!(" {msg}")).style(Style::default().fg(Color::Green)),
            chunks[4],
        );
    }

    if tbl.entries.is_empty() {
        frame.render_widget(
            Paragraph::new(" No partitions found. Try s to scan for lost partitions."),
            chunks[3],
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
            let name = e.name.as_deref().unwrap_or("\u{2014}");
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
    frame.render_stateful_widget(table, chunks[3], &mut ts);
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

    #[test]
    fn w_key_when_no_device_does_nothing() {
        let mut s = PartitionState::new();
        s.handle_key(KeyCode::Char('w'), KeyModifiers::NONE);
        // Should not panic; no device means early return.
        assert!(s.export_status.is_none());
    }

    #[test]
    fn w_key_when_no_table_does_nothing() {
        use ferrite_blockdev::MockBlockDevice;
        use std::sync::Arc;
        let dev: Arc<dyn BlockDevice> = Arc::new(MockBlockDevice::zeroed(4096, 512));
        let mut s = PartitionState::new();
        s.device = Some(dev);
        s.handle_key(KeyCode::Char('w'), KeyModifiers::NONE);
        // No table → early return, no panic, no export status.
        assert!(s.export_status.is_none());
    }

    #[test]
    fn auto_scan_triggered_when_read_returns_empty_table() {
        use ferrite_blockdev::MockBlockDevice;
        use ferrite_partition::{PartitionTable, PartitionTableKind};
        use std::sync::mpsc;
        use std::sync::Arc;

        let dev: Arc<dyn BlockDevice> = Arc::new(MockBlockDevice::zeroed(4096, 512));
        let mut s = PartitionState::new();
        s.device = Some(dev);

        // Simulate the background thread sending an empty table as the read result.
        let (tx, rx) = mpsc::channel::<PartitionMsg>();
        s.rx = Some(rx);
        s.status = PartitionStatus::Reading;

        let empty_table = PartitionTable {
            kind: PartitionTableKind::Mbr,
            sector_size: 512,
            disk_size_lba: 8,
            entries: vec![],
            note: None,
        };
        tx.send(PartitionMsg::Table(empty_table)).unwrap();

        // First tick processes the empty table and starts auto-scan.
        s.tick();
        // Status should now be AutoScanning (scan thread was spawned internally).
        assert!(matches!(
            s.status,
            PartitionStatus::AutoScanning | PartitionStatus::Scanning
        ));
    }

    #[test]
    fn no_auto_scan_when_read_returns_non_empty_table() {
        use ferrite_blockdev::MockBlockDevice;
        use ferrite_partition::{
            PartitionEntry, PartitionKind, PartitionTable, PartitionTableKind,
        };
        use std::sync::mpsc;
        use std::sync::Arc;

        let dev: Arc<dyn BlockDevice> = Arc::new(MockBlockDevice::zeroed(4096, 512));
        let mut s = PartitionState::new();
        s.device = Some(dev);

        let (tx, rx) = mpsc::channel::<PartitionMsg>();
        s.rx = Some(rx);
        s.status = PartitionStatus::Reading;

        let non_empty_table = PartitionTable {
            kind: PartitionTableKind::Mbr,
            sector_size: 512,
            disk_size_lba: 8,
            entries: vec![PartitionEntry {
                index: 0,
                start_lba: 2,
                end_lba: 7,
                size_lba: 6,
                name: None,
                kind: PartitionKind::Mbr {
                    partition_type: 0x07,
                },
                bootable: false,
            }],
            note: None,
        };
        tx.send(PartitionMsg::Table(non_empty_table)).unwrap();
        s.tick();
        // Should go straight to Done, not AutoScanning.
        assert!(matches!(s.status, PartitionStatus::Done));
    }

    #[test]
    fn set_imaging_context_updates_fields() {
        let mut s = PartitionState::new();
        assert!(!s.imaging_active);
        assert!(s.fallback_image_path.is_none());

        s.set_imaging_context(true, Some("/tmp/test.img".to_string()));
        assert!(s.imaging_active);
        assert_eq!(s.fallback_image_path.as_deref(), Some("/tmp/test.img"));

        s.set_imaging_context(false, None);
        assert!(!s.imaging_active);
        assert!(s.fallback_image_path.is_none());
    }

    #[test]
    fn fallback_not_used_when_path_is_none() {
        use ferrite_blockdev::MockBlockDevice;
        use std::sync::Arc;
        let dev: Arc<dyn BlockDevice> = Arc::new(MockBlockDevice::zeroed(4096, 512));
        let mut s = PartitionState::new();
        s.device = Some(dev);
        // imaging_active but no path set
        s.set_imaging_context(true, None);
        assert!(!s.used_image_fallback);
        // After start_read with no fallback path, used_image_fallback must remain false.
        s.start_read();
        assert!(!s.used_image_fallback);
    }
}

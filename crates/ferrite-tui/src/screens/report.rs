//! Recovery report generation — aggregates results from all screens into a
//! plain-text report string.

use ferrite_core::types::DeviceInfo;
use ferrite_partition::{PartitionTable, PartitionTableKind};
use ferrite_smart::SmartData;

/// Generate a plain-text recovery report string.
///
/// The caller is responsible for writing the string to disk.
pub fn generate_report(
    device_info: &DeviceInfo,
    smart: Option<&SmartData>,
    imaging_dest: &str,
    imaging_mapfile: &str,
    partition_table: Option<&PartitionTable>,
    carve_hit_count: usize,
) -> String {
    let mut out = String::new();

    out.push_str("=== Ferrite Recovery Report ===\n");
    out.push_str(&format!(
        "Generated: {}\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
    ));

    // ── Device ────────────────────────────────────────────────────────────────
    out.push_str("\n--- Device ---\n");
    out.push_str(&format!("Path:   {}\n", device_info.path));
    out.push_str(&format!(
        "Model:  {}\n",
        device_info.model.as_deref().unwrap_or("—")
    ));
    out.push_str(&format!(
        "Serial: {}\n",
        device_info.serial.as_deref().unwrap_or("—")
    ));
    out.push_str(&format!("Size:   {} bytes\n", device_info.size_bytes));

    // ── S.M.A.R.T. ───────────────────────────────────────────────────────────
    out.push_str("\n--- S.M.A.R.T. ---\n");
    match smart {
        None => {
            out.push_str("Not available\n");
        }
        Some(data) => {
            out.push_str(&format!(
                "Overall health: {}\n",
                if data.smart_passed {
                    "PASSED"
                } else {
                    "FAILED"
                }
            ));
            // Reallocated sectors (ATA attr 0x05 = 5)
            if let Some(attr) = data.attribute(5) {
                out.push_str(&format!("Reallocated sectors: {}\n", attr.raw_value));
            }
            // Pending sectors (ATA attr 0xC5 = 197)
            if let Some(attr) = data.attribute(197) {
                out.push_str(&format!("Pending sectors: {}\n", attr.raw_value));
            }
            if let Some(h) = data.power_on_hours {
                out.push_str(&format!("Power-on hours: {h}\n"));
            }
        }
    }

    // ── Imaging ───────────────────────────────────────────────────────────────
    out.push_str("\n--- Imaging ---\n");
    out.push_str(&format!(
        "Destination: {}\n",
        if imaging_dest.is_empty() {
            "not set"
        } else {
            imaging_dest
        }
    ));
    out.push_str(&format!(
        "Map file:    {}\n",
        if imaging_mapfile.is_empty() {
            "not set"
        } else {
            imaging_mapfile
        }
    ));

    // ── Partitions ────────────────────────────────────────────────────────────
    out.push_str("\n--- Partitions ---\n");
    match partition_table {
        None => {
            out.push_str("Not read\n");
        }
        Some(tbl) => {
            let kind_str = match tbl.kind {
                PartitionTableKind::Mbr => "MBR",
                PartitionTableKind::Gpt => "GPT",
                PartitionTableKind::Recovered => "Recovered (signature scan)",
            };
            out.push_str(&format!("Table type: {kind_str}\n"));
            out.push_str(&format!("Partitions: {}\n", tbl.entries.len()));
            for e in &tbl.entries {
                out.push_str(&format!(
                    "  #{}: start={} end={} size={} LBA\n",
                    e.index, e.start_lba, e.end_lba, e.size_lba
                ));
            }
        }
    }

    // ── Carving ───────────────────────────────────────────────────────────────
    out.push_str("\n--- Carving ---\n");
    out.push_str(&format!("Total hits: {carve_hit_count}\n"));

    out
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_device_info(path: &str) -> DeviceInfo {
        DeviceInfo {
            path: path.to_string(),
            model: Some("TestModel".to_string()),
            serial: Some("SN123".to_string()),
            size_bytes: 1_000_000_000,
            sector_size: 512,
            logical_sector_size: 512,
        }
    }

    #[test]
    fn report_contains_device_path() {
        let info = make_device_info("/dev/sda");
        let report = generate_report(&info, None, "", "", None, 0);
        assert!(
            report.contains("/dev/sda"),
            "report should contain device path"
        );
    }

    #[test]
    fn report_handles_no_smart_data() {
        let info = make_device_info("/dev/sdb");
        let report = generate_report(&info, None, "", "", None, 0);
        assert!(
            report.contains("Not available"),
            "report should say 'Not available' when smart is None"
        );
    }
}

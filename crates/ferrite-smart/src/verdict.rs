/// Drive health assessment — converts raw `SmartData` into a `HealthVerdict`.
use crate::thresholds::SmartThresholds;
use crate::types::{HealthVerdict, SmartData};

/// Assess drive health against `thresholds`, returning the worst applicable verdict.
///
/// Reasons are accumulated and the highest severity encountered wins.
pub fn assess_health(data: &SmartData, thresholds: &SmartThresholds) -> HealthVerdict {
    let mut warnings: Vec<String> = Vec::new();
    let mut criticals: Vec<String> = Vec::new();

    // ── Overall SMART self-assessment ─────────────────────────────────────────
    if !data.smart_passed {
        criticals.push("SMART overall health self-assessment FAILED".to_owned());
    }

    // ── Temperature ───────────────────────────────────────────────────────────
    if let Some(temp) = data.temperature_celsius {
        let t = &thresholds.temperature;
        if temp >= t.critical_c {
            criticals.push(format!(
                "Temperature {temp}°C ≥ critical threshold {}",
                t.critical_c
            ));
        } else if temp >= t.warning_c {
            warnings.push(format!(
                "Temperature {temp}°C ≥ warning threshold {}",
                t.warning_c
            ));
        }
    }

    // ── ATA attribute checks ──────────────────────────────────────────────────

    // ID 5 — Reallocated_Sector_Ct
    check_count_attribute(
        data,
        5,
        "Reallocated sectors",
        &thresholds.reallocated_sectors,
        &mut warnings,
        &mut criticals,
    );

    // ID 197 — Current_Pending_Sector
    check_count_attribute(
        data,
        197,
        "Pending sectors",
        &thresholds.pending_sectors,
        &mut warnings,
        &mut criticals,
    );

    // ID 198 — Offline_Uncorrectable
    check_count_attribute(
        data,
        198,
        "Uncorrectable sectors",
        &thresholds.uncorrectable_sectors,
        &mut warnings,
        &mut criticals,
    );

    // ID 3 — Spin_Up_Time (spinning HDDs only; skip for SSDs and USB bridges).
    // SSDs report rotation_rate == Some(0); USB bridge-connected drives often
    // report None or garbage values for this attribute — checking it causes
    // spurious CRITICAL verdicts on flash drives and external SSDs.
    let is_spinning_hdd = matches!(data.rotation_rate, Some(rpm) if rpm > 0);
    if is_spinning_hdd {
        if let Some(attr) = data.attribute(3) {
            let ms = attr.raw_value;
            let t = &thresholds.spin_up_time_ms;
            if ms >= t.critical_ms {
                criticals.push(format!(
                    "Spin-up time {ms}ms ≥ critical threshold {}",
                    t.critical_ms
                ));
            } else if ms >= t.warning_ms {
                warnings.push(format!(
                    "Spin-up time {ms}ms ≥ warning threshold {}",
                    t.warning_ms
                ));
            }
        }
    }

    // ── NVMe-specific checks ──────────────────────────────────────────────────

    if let Some(cw) = data.nvme_critical_warning {
        if cw > 0 {
            warnings.push(format!(
                "NVMe critical warning bitmask non-zero: 0x{cw:02x}"
            ));
        }
    }

    if let (Some(spare), Some(spare_thresh)) = (
        data.nvme_available_spare,
        data.nvme_available_spare_threshold,
    ) {
        if spare < spare_thresh {
            criticals.push(format!(
                "NVMe available spare {spare}% below threshold {spare_thresh}%"
            ));
        }
    }

    if let Some(media_errors) = data.nvme_media_errors {
        if media_errors > 0 {
            criticals.push(format!("NVMe media/data integrity errors: {media_errors}"));
        }
    }

    // ── When-failed attribute flags ───────────────────────────────────────────
    for attr in &data.attributes {
        if !attr.when_failed.is_empty() {
            warnings.push(format!(
                "Attribute {} ({}) flagged when_failed: {}",
                attr.id, attr.name, attr.when_failed
            ));
        }
    }

    // ── Assemble verdict ──────────────────────────────────────────────────────
    if !criticals.is_empty() {
        criticals.extend(warnings);
        HealthVerdict::Critical { reasons: criticals }
    } else if !warnings.is_empty() {
        HealthVerdict::Warning { reasons: warnings }
    } else {
        HealthVerdict::Healthy
    }
}

fn check_count_attribute(
    data: &SmartData,
    id: u8,
    label: &str,
    thresholds: &crate::thresholds::CountThresholds,
    warnings: &mut Vec<String>,
    criticals: &mut Vec<String>,
) {
    if let Some(attr) = data.attribute(id) {
        let count = attr.raw_value;
        if count >= thresholds.critical_count {
            criticals.push(format!(
                "{label} count {count} ≥ critical threshold {}",
                thresholds.critical_count
            ));
        } else if count >= thresholds.warning_count {
            warnings.push(format!(
                "{label} count {count} ≥ warning threshold {}",
                thresholds.warning_count
            ));
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;
    use crate::thresholds::SmartThresholds;

    fn thresholds() -> SmartThresholds {
        SmartThresholds::default_config()
    }

    fn make_data(json: &str) -> SmartData {
        parse("/dev/sda", json).unwrap()
    }

    #[test]
    fn healthy_drive() {
        let data = make_data(
            r#"{
            "smart_status": { "passed": true },
            "temperature": { "current": 35 },
            "ata_smart_attributes": { "table": [] }
        }"#,
        );
        assert_eq!(assess_health(&data, &thresholds()), HealthVerdict::Healthy);
    }

    #[test]
    fn smart_failed_is_critical() {
        let data = make_data(r#"{ "smart_status": { "passed": false } }"#);
        let verdict = assess_health(&data, &thresholds());
        assert!(matches!(verdict, HealthVerdict::Critical { .. }));
        assert!(verdict.blocks_imaging());
    }

    #[test]
    fn high_temperature_warning() {
        let data = make_data(
            r#"{
            "smart_status": { "passed": true },
            "temperature": { "current": 55 }
        }"#,
        );
        let verdict = assess_health(&data, &thresholds());
        assert!(matches!(verdict, HealthVerdict::Warning { .. }));
        assert!(verdict.reasons().iter().any(|r| r.contains("Temperature")));
    }

    #[test]
    fn critical_temperature() {
        let data = make_data(
            r#"{
            "smart_status": { "passed": true },
            "temperature": { "current": 65 }
        }"#,
        );
        let verdict = assess_health(&data, &thresholds());
        assert!(matches!(verdict, HealthVerdict::Critical { .. }));
    }

    #[test]
    fn reallocated_sectors_warning() {
        let data = make_data(
            r#"{
            "smart_status": { "passed": true },
            "ata_smart_attributes": {
                "table": [{
                    "id": 5, "name": "Reallocated_Sector_Ct",
                    "value": 100, "worst": 100, "thresh": 140,
                    "flags": { "prefailure": true },
                    "raw": { "value": 3 },
                    "when_failed": ""
                }]
            }
        }"#,
        );
        let verdict = assess_health(&data, &thresholds());
        assert!(matches!(verdict, HealthVerdict::Warning { .. }));
    }

    #[test]
    fn reallocated_sectors_critical() {
        let data = make_data(
            r#"{
            "smart_status": { "passed": true },
            "ata_smart_attributes": {
                "table": [{
                    "id": 5, "name": "Reallocated_Sector_Ct",
                    "value": 100, "worst": 100, "thresh": 140,
                    "flags": { "prefailure": true },
                    "raw": { "value": 75 },
                    "when_failed": ""
                }]
            }
        }"#,
        );
        let verdict = assess_health(&data, &thresholds());
        assert!(matches!(verdict, HealthVerdict::Critical { .. }));
    }

    #[test]
    fn pending_sectors_warning() {
        let data = make_data(
            r#"{
            "smart_status": { "passed": true },
            "ata_smart_attributes": {
                "table": [{
                    "id": 197, "name": "Current_Pending_Sector",
                    "value": 200, "worst": 200, "thresh": 0,
                    "flags": { "prefailure": false },
                    "raw": { "value": 1 },
                    "when_failed": ""
                }]
            }
        }"#,
        );
        assert!(matches!(
            assess_health(&data, &thresholds()),
            HealthVerdict::Warning { .. }
        ));
    }

    #[test]
    fn when_failed_attribute_is_warning() {
        let data = make_data(
            r#"{
            "smart_status": { "passed": true },
            "ata_smart_attributes": {
                "table": [{
                    "id": 12, "name": "Power_Cycle_Count",
                    "value": 100, "worst": 100, "thresh": 0,
                    "flags": { "prefailure": false },
                    "raw": { "value": 0 },
                    "when_failed": "past"
                }]
            }
        }"#,
        );
        let verdict = assess_health(&data, &thresholds());
        assert!(matches!(verdict, HealthVerdict::Warning { .. }));
        assert!(verdict.reasons().iter().any(|r| r.contains("when_failed")));
    }

    #[test]
    fn nvme_media_errors_critical() {
        let data = make_data(
            r#"{
            "smart_status": { "passed": true },
            "nvme_smart_health_information_log": {
                "critical_warning": 0,
                "media_errors": 5,
                "available_spare": 100,
                "available_spare_threshold": 10,
                "percentage_used": 0
            }
        }"#,
        );
        assert!(matches!(
            assess_health(&data, &thresholds()),
            HealthVerdict::Critical { .. }
        ));
    }

    #[test]
    fn nvme_spare_below_threshold_critical() {
        let data = make_data(
            r#"{
            "smart_status": { "passed": true },
            "nvme_smart_health_information_log": {
                "critical_warning": 0,
                "media_errors": 0,
                "available_spare": 5,
                "available_spare_threshold": 10,
                "percentage_used": 0
            }
        }"#,
        );
        assert!(matches!(
            assess_health(&data, &thresholds()),
            HealthVerdict::Critical { .. }
        ));
    }

    #[test]
    fn healthy_label_display() {
        assert_eq!(HealthVerdict::Healthy.label(), "HEALTHY");
        assert_eq!(HealthVerdict::Healthy.to_string(), "HEALTHY");
        assert!(!HealthVerdict::Healthy.blocks_imaging());
    }
}

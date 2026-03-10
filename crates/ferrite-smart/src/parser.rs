/// Serde deserialization of `smartctl --json -a` output.
///
/// Only the fields Ferrite needs are captured. Unknown fields are ignored via
/// `deny_unknown_fields = false` (the default).
use serde::Deserialize;

use crate::error::{Result, SmartError};
use crate::types::{SmartAttribute, SmartData};

// ── Top-level JSON structure ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct RawSmartctl {
    device: Option<RawDevice>,
    model_name: Option<String>,
    serial_number: Option<String>,
    firmware_version: Option<String>,
    user_capacity: Option<RawUserCapacity>,
    rotation_rate: Option<u32>,
    smart_status: Option<RawSmartStatus>,
    temperature: Option<RawTemperature>,
    power_on_time: Option<RawPowerOnTime>,
    ata_smart_attributes: Option<RawAtaAttributes>,
    nvme_smart_health_information_log: Option<RawNvmeLog>,
}

#[derive(Debug, Deserialize)]
struct RawDevice {
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawUserCapacity {
    bytes: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct RawSmartStatus {
    passed: bool,
}

#[derive(Debug, Deserialize)]
struct RawTemperature {
    current: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct RawPowerOnTime {
    hours: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct RawAtaAttributes {
    table: Vec<RawAtaAttribute>,
}

#[derive(Debug, Deserialize)]
struct RawAtaAttribute {
    id: u8,
    name: Option<String>,
    value: Option<u8>,
    worst: Option<u8>,
    thresh: Option<u8>,
    flags: Option<RawAtaFlags>,
    raw: Option<RawAtaRaw>,
    when_failed: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawAtaFlags {
    prefailure: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct RawAtaRaw {
    value: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct RawNvmeLog {
    critical_warning: Option<u8>,
    media_errors: Option<u64>,
    available_spare: Option<u8>,
    available_spare_threshold: Option<u8>,
    percentage_used: Option<u8>,
}

// ── Public parse entry point ──────────────────────────────────────────────────

/// Parse the JSON output of `smartctl --json -a` into a [`SmartData`].
pub fn parse(device_path: &str, json: &str) -> Result<SmartData> {
    let raw: RawSmartctl =
        serde_json::from_str(json).map_err(|e| SmartError::Parse(e.to_string()))?;

    let attributes = raw
        .ata_smart_attributes
        .map(|a| a.table.into_iter().map(convert_attribute).collect())
        .unwrap_or_default();

    let nvme = raw.nvme_smart_health_information_log;

    Ok(SmartData {
        device_path: raw
            .device
            .and_then(|d| d.name)
            .unwrap_or_else(|| device_path.to_owned()),
        model: raw.model_name,
        serial: raw.serial_number,
        firmware: raw.firmware_version,
        capacity_bytes: raw.user_capacity.and_then(|c| c.bytes),
        rotation_rate: raw.rotation_rate,
        smart_passed: raw.smart_status.map(|s| s.passed).unwrap_or(false),
        temperature_celsius: raw.temperature.and_then(|t| t.current),
        power_on_hours: raw.power_on_time.and_then(|p| p.hours),
        attributes,
        nvme_critical_warning: nvme.as_ref().and_then(|n| n.critical_warning),
        nvme_media_errors: nvme.as_ref().and_then(|n| n.media_errors),
        nvme_available_spare: nvme.as_ref().and_then(|n| n.available_spare),
        nvme_available_spare_threshold: nvme.as_ref().and_then(|n| n.available_spare_threshold),
        nvme_percentage_used: nvme.as_ref().and_then(|n| n.percentage_used),
    })
}

fn convert_attribute(raw: RawAtaAttribute) -> SmartAttribute {
    SmartAttribute {
        id: raw.id,
        name: raw.name.unwrap_or_default(),
        value: raw.value.unwrap_or(0),
        worst: raw.worst.unwrap_or(0),
        thresh: raw.thresh.unwrap_or(0),
        prefailure: raw.flags.and_then(|f| f.prefailure).unwrap_or(false),
        raw_value: raw.raw.and_then(|r| r.value).unwrap_or(0),
        when_failed: raw.when_failed.unwrap_or_default(),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const ATA_JSON: &str = r#"{
        "device": { "name": "/dev/sda", "type": "ata" },
        "model_name": "WDC WD10EZEX",
        "serial_number": "WD-ABC123",
        "firmware_version": "01.01A01",
        "user_capacity": { "blocks": 1953525168, "bytes": 1000204886016 },
        "rotation_rate": 7200,
        "smart_status": { "passed": true },
        "temperature": { "current": 38 },
        "power_on_time": { "hours": 4321 },
        "ata_smart_attributes": {
            "table": [
                {
                    "id": 5,
                    "name": "Reallocated_Sector_Ct",
                    "value": 200,
                    "worst": 200,
                    "thresh": 140,
                    "flags": { "prefailure": true },
                    "raw": { "value": 0 },
                    "when_failed": ""
                },
                {
                    "id": 197,
                    "name": "Current_Pending_Sector",
                    "value": 200,
                    "worst": 200,
                    "thresh": 0,
                    "flags": { "prefailure": false },
                    "raw": { "value": 3 },
                    "when_failed": ""
                }
            ]
        }
    }"#;

    const NVME_JSON: &str = r#"{
        "device": { "name": "/dev/nvme0", "type": "nvme" },
        "model_name": "Samsung SSD 980 PRO",
        "serial_number": "S5P2NX0T123456",
        "firmware_version": "2B2QGXA7",
        "user_capacity": { "bytes": 1000204886016 },
        "rotation_rate": 0,
        "smart_status": { "passed": true },
        "temperature": { "current": 42 },
        "power_on_time": { "hours": 1200 },
        "nvme_smart_health_information_log": {
            "critical_warning": 0,
            "temperature": 42,
            "available_spare": 100,
            "available_spare_threshold": 10,
            "percentage_used": 2,
            "media_errors": 0
        }
    }"#;

    #[test]
    fn parse_ata_drive() {
        let data = parse("/dev/sda", ATA_JSON).unwrap();
        assert_eq!(data.model.as_deref(), Some("WDC WD10EZEX"));
        assert_eq!(data.serial.as_deref(), Some("WD-ABC123"));
        assert_eq!(data.rotation_rate, Some(7200));
        assert!(data.smart_passed);
        assert_eq!(data.temperature_celsius, Some(38));
        assert_eq!(data.power_on_hours, Some(4321));
        assert_eq!(data.attributes.len(), 2);

        let attr5 = data.attribute(5).unwrap();
        assert_eq!(attr5.name, "Reallocated_Sector_Ct");
        assert!(attr5.prefailure);
        assert_eq!(attr5.raw_value, 0);

        let attr197 = data.attribute(197).unwrap();
        assert_eq!(attr197.raw_value, 3);
    }

    #[test]
    fn parse_nvme_drive() {
        let data = parse("/dev/nvme0", NVME_JSON).unwrap();
        assert_eq!(data.rotation_rate, Some(0));
        assert!(data.attributes.is_empty());
        assert_eq!(data.nvme_critical_warning, Some(0));
        assert_eq!(data.nvme_available_spare, Some(100));
        assert_eq!(data.nvme_available_spare_threshold, Some(10));
        assert_eq!(data.nvme_percentage_used, Some(2));
        assert_eq!(data.nvme_media_errors, Some(0));
    }

    #[test]
    fn parse_error_on_invalid_json() {
        let err = parse("/dev/sda", "not json").unwrap_err();
        assert!(matches!(err, crate::error::SmartError::Parse(_)));
    }

    #[test]
    fn missing_fields_use_defaults() {
        let data = parse("/dev/sda", "{}").unwrap();
        assert_eq!(data.device_path, "/dev/sda");
        assert!(!data.smart_passed);
        assert!(data.model.is_none());
        assert!(data.attributes.is_empty());
    }
}

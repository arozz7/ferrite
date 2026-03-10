/// TOML-backed S.M.A.R.T. threshold configuration.
use std::path::Path;

use serde::Deserialize;

use crate::error::{Result, SmartError};

/// Full set of health thresholds loaded from `config/smart_thresholds.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct SmartThresholds {
    pub temperature: TemperatureThresholds,
    pub reallocated_sectors: CountThresholds,
    pub pending_sectors: CountThresholds,
    pub uncorrectable_sectors: CountThresholds,
    pub spin_up_time_ms: SpinUpThresholds,
    /// Informational — not used for verdict logic.
    #[serde(default)]
    pub power_on_hours: PowerOnHoursNote,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TemperatureThresholds {
    pub warning_c: u32,
    pub critical_c: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CountThresholds {
    pub warning_count: u64,
    pub critical_count: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SpinUpThresholds {
    pub warning_ms: u64,
    pub critical_ms: u64,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PowerOnHoursNote {
    #[serde(default)]
    pub note: String,
}

impl SmartThresholds {
    /// Sensible defaults that match `config/smart_thresholds.toml`.
    pub fn default_config() -> Self {
        Self {
            temperature: TemperatureThresholds {
                warning_c: 50,
                critical_c: 60,
            },
            reallocated_sectors: CountThresholds {
                warning_count: 1,
                critical_count: 50,
            },
            pending_sectors: CountThresholds {
                warning_count: 1,
                critical_count: 10,
            },
            uncorrectable_sectors: CountThresholds {
                warning_count: 1,
                critical_count: 5,
            },
            spin_up_time_ms: SpinUpThresholds {
                warning_ms: 10_000,
                critical_ms: 20_000,
            },
            power_on_hours: PowerOnHoursNote::default(),
        }
    }

    /// Load thresholds from a TOML file.
    pub fn from_file(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|source| SmartError::ThresholdConfig {
            path: path.display().to_string(),
            source,
        })?;

        toml::from_str(&text).map_err(|source| SmartError::ThresholdParse {
            path: path.display().to_string(),
            source,
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_values() {
        let t = SmartThresholds::default_config();
        assert_eq!(t.temperature.warning_c, 50);
        assert_eq!(t.temperature.critical_c, 60);
        assert_eq!(t.reallocated_sectors.warning_count, 1);
        assert_eq!(t.pending_sectors.critical_count, 10);
        assert_eq!(t.spin_up_time_ms.critical_ms, 20_000);
    }

    #[test]
    fn parse_toml_inline() {
        let toml_str = r#"
            [temperature]
            warning_c  = 45
            critical_c = 55

            [reallocated_sectors]
            warning_count  = 2
            critical_count = 100

            [pending_sectors]
            warning_count  = 1
            critical_count = 20

            [uncorrectable_sectors]
            warning_count  = 1
            critical_count = 5

            [spin_up_time_ms]
            warning_ms  = 8000
            critical_ms = 15000
        "#;
        let t: SmartThresholds = toml::from_str(toml_str).unwrap();
        assert_eq!(t.temperature.warning_c, 45);
        assert_eq!(t.reallocated_sectors.warning_count, 2);
        assert_eq!(t.spin_up_time_ms.warning_ms, 8000);
    }

    #[test]
    fn from_file_not_found_returns_threshold_config_error() {
        let err = SmartThresholds::from_file(Path::new("/no/such/file.toml")).unwrap_err();
        assert!(matches!(
            err,
            crate::error::SmartError::ThresholdConfig { .. }
        ));
    }
}

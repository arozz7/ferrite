/// A single ATA S.M.A.R.T. attribute from the drive's attribute table.
#[derive(Debug, Clone)]
pub struct SmartAttribute {
    pub id: u8,
    pub name: String,
    /// Normalised value (higher = better for most attributes).
    pub value: u8,
    /// Worst normalised value ever recorded.
    pub worst: u8,
    /// Failure threshold — if `value` drops below this the drive has failed.
    pub thresh: u8,
    /// Whether this is a pre-failure attribute (vs. usage/informational).
    pub prefailure: bool,
    /// Raw counter value (sector counts, hours, etc.).
    pub raw_value: u64,
    /// Non-empty when the attribute is currently or previously failed.
    pub when_failed: String,
}

/// Health data collected from a single drive via `smartctl --json -a`.
#[derive(Debug, Clone)]
pub struct SmartData {
    /// Device path as passed to smartctl.
    pub device_path: String,
    pub model: Option<String>,
    pub serial: Option<String>,
    pub firmware: Option<String>,
    /// Total user-accessible bytes.
    pub capacity_bytes: Option<u64>,
    /// Rotation speed in RPM. `Some(0)` means SSD/NVMe, `None` means unknown.
    pub rotation_rate: Option<u32>,
    /// Overall SMART self-assessment (`PASSED` / `FAILED`).
    pub smart_passed: bool,
    /// Current temperature in °C.
    pub temperature_celsius: Option<u32>,
    /// Cumulative power-on hours.
    pub power_on_hours: Option<u64>,
    /// ATA attribute table. Empty for NVMe drives.
    pub attributes: Vec<SmartAttribute>,
    // ── NVMe-specific fields ───────────────────────────────────────────────
    /// Bitmask of NVMe critical warnings (0 = none).
    pub nvme_critical_warning: Option<u8>,
    /// NVMe media and data integrity errors.
    pub nvme_media_errors: Option<u64>,
    /// Available spare percentage (NVMe SSDs).
    pub nvme_available_spare: Option<u8>,
    /// Threshold below which available spare triggers a warning.
    pub nvme_available_spare_threshold: Option<u8>,
    /// Drive lifetime percentage used.
    pub nvme_percentage_used: Option<u8>,
}

impl SmartData {
    /// Convenience: look up an ATA attribute by its well-known ID.
    pub fn attribute(&self, id: u8) -> Option<&SmartAttribute> {
        self.attributes.iter().find(|a| a.id == id)
    }
}

/// Health assessment verdict for a drive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthVerdict {
    Healthy,
    Warning { reasons: Vec<String> },
    Critical { reasons: Vec<String> },
}

impl HealthVerdict {
    /// True if the verdict is severe enough to block an imaging run.
    pub fn blocks_imaging(&self) -> bool {
        matches!(self, Self::Critical { .. })
    }

    pub fn reasons(&self) -> &[String] {
        match self {
            Self::Healthy => &[],
            Self::Warning { reasons } | Self::Critical { reasons } => reasons,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Healthy => "HEALTHY",
            Self::Warning { .. } => "WARNING",
            Self::Critical { .. } => "CRITICAL",
        }
    }
}

impl std::fmt::Display for HealthVerdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.label())
    }
}

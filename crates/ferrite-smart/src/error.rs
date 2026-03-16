use thiserror::Error;

#[derive(Debug, Error)]
pub enum SmartError {
    #[error("smartctl not found — install smartmontools (https://www.smartmontools.org)")]
    SmartctlNotFound,

    #[error("Failed to execute smartctl: {0}")]
    Io(#[from] std::io::Error),

    #[error("smartctl exited with hard error code {code} (bits 0-3 set)")]
    SmartctlError { code: i32 },

    #[error("Failed to parse smartctl JSON: {0}")]
    Parse(String),

    #[error("Device does not support SMART or data unavailable")]
    NotSupported,

    #[error("Failed to load threshold config '{path}': {source}")]
    ThresholdConfig {
        path: String,
        source: std::io::Error,
    },

    #[error("Failed to parse threshold config '{path}': {source}")]
    ThresholdParse {
        path: String,
        source: toml::de::Error,
    },
}

pub type Result<T> = std::result::Result<T, SmartError>;

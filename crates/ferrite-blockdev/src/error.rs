use thiserror::Error;

#[derive(Debug, Error)]
pub enum BlockDeviceError {
    #[error("I/O error on '{device}' at offset {offset:#x}: {source}")]
    Io {
        device: String,
        offset: u64,
        #[source]
        source: std::io::Error,
    },

    #[error("Device '{0}' not found")]
    NotFound(String),

    #[error("Access denied to '{0}' — run as Administrator / root")]
    PermissionDenied(String),

    #[error("Simulated read error at offset {0:#x}")]
    Simulated(u64),

    #[error("Unsupported: {0}")]
    Unsupported(String),
}

pub type Result<T> = std::result::Result<T, BlockDeviceError>;

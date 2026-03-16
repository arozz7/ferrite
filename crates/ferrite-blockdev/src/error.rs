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

    #[error("Read timeout on '{device}' at offset {offset:#x}: sector did not respond within the timeout window")]
    Timeout { device: String, offset: u64 },
}

pub type Result<T> = std::result::Result<T, BlockDeviceError>;

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_error_display() {
        let e = BlockDeviceError::Timeout {
            device: "/dev/sda".to_string(),
            offset: 0x1000,
        };
        let msg = e.to_string();
        assert!(msg.contains("/dev/sda"), "device path missing from message");
        assert!(msg.contains("0x1000"), "offset missing from message");
    }
}

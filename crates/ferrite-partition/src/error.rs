use thiserror::Error;

#[derive(Debug, Error)]
pub enum PartitionError {
    #[error("Block device error: {0}")]
    BlockDevice(#[from] ferrite_blockdev::BlockDeviceError),

    #[error("Buffer too small: need {needed} bytes, got {got}")]
    BufferTooSmall { needed: usize, got: usize },

    #[error("Invalid {context} signature: expected {expected}, found {found:02X?}")]
    InvalidSignature {
        context: &'static str,
        expected: &'static str,
        found: Vec<u8>,
    },

    #[error("GPT CRC mismatch: expected {expected:#010x}, computed {computed:#010x}")]
    CrcMismatch { expected: u32, computed: u32 },

    #[error("Invalid GPT header: {0}")]
    InvalidGptHeader(String),
}

pub type Result<T> = std::result::Result<T, PartitionError>;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum FilesystemError {
    #[error("block device error: {0}")]
    BlockDevice(#[from] ferrite_blockdev::BlockDeviceError),

    #[error("unknown or unsupported filesystem")]
    UnknownFilesystem,

    #[error("invalid {context} structure: {reason}")]
    InvalidStructure {
        context: &'static str,
        reason: String,
    },

    #[error("entry not found: {0}")]
    NotFound(String),

    #[error("buffer underrun: need {needed} bytes, got {got}")]
    BufferTooSmall { needed: usize, got: usize },
}

pub type Result<T> = std::result::Result<T, FilesystemError>;

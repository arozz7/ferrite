use thiserror::Error;

#[derive(Debug, Error)]
pub enum CarveError {
    #[error("block device error: {0}")]
    BlockDevice(#[from] ferrite_blockdev::BlockDeviceError),

    #[error("invalid signature definition: {0}")]
    InvalidSignature(String),

    #[error("TOML parse error: {0}")]
    TomlParse(#[from] toml::de::Error),

    #[error("I/O error: {0}")]
    Io(String),
}

pub type Result<T> = std::result::Result<T, CarveError>;

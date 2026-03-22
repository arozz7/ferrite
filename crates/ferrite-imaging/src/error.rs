use std::path::PathBuf;

use ferrite_blockdev::BlockDeviceError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ImagingError {
    #[error("Device read error on '{device}' at offset {offset:#x}: {source}")]
    DeviceRead {
        device: String,
        offset: u64,
        #[source]
        source: BlockDeviceError,
    },

    #[error("Image write error at offset {offset:#x}: {source}")]
    ImageWrite {
        offset: u64,
        #[source]
        source: std::io::Error,
    },

    #[error("Mapfile parse error at line {line}: {message}")]
    MapfileParse { line: usize, message: String },

    #[error("Mapfile I/O error '{path}': {source}")]
    MapfileIo {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "Device size mismatch: mapfile covers {mapfile_bytes} bytes, device has {device_bytes} bytes"
    )]
    SizeMismatch {
        mapfile_bytes: u64,
        device_bytes: u64,
    },

    #[error("Imaging cancelled by caller")]
    Cancelled,

    #[error(
        "Output file '{path}' is locked by another imaging session. \
         Stop the other session before starting a new one."
    )]
    OutputLocked { path: PathBuf },
}

pub type Result<T> = std::result::Result<T, ImagingError>;

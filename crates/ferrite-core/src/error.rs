use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("I/O error on device '{device}' at offset {offset:#x}: {source}")]
    Io {
        device: String,
        offset: u64,
        #[source]
        source: std::io::Error,
    },

    #[error("Invalid configuration: {0}")]
    Config(String),

    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Unsupported operation: {0}")]
    Unsupported(String),
}

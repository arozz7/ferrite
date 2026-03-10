/// Global runtime configuration, loaded once at startup.
#[derive(Debug, Clone)]
pub struct Config {
    /// Default block size for imaging reads (bytes).
    pub imaging_block_size: u64,
    /// Maximum retries for bad-sector recovery.
    pub imaging_max_retries: u32,
    /// Path to smartctl binary (auto-detected if None).
    pub smartctl_path: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            imaging_block_size: 512 * 1024, // 512 KiB
            imaging_max_retries: 3,
            smartctl_path: None,
        }
    }
}

/// Returned by [`ProgressReporter::report`] — allows the caller to cancel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    Continue,
    Cancel,
}

/// Which imaging pass is currently running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImagingPhase {
    Copy,
    Trim,
    Sweep,
    Scrape,
    Retry { attempt: u32, max: u32 },
    Complete,
}

impl std::fmt::Display for ImagingPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Copy => write!(f, "Copy"),
            Self::Trim => write!(f, "Trim"),
            Self::Sweep => write!(f, "Sweep"),
            Self::Scrape => write!(f, "Scrape"),
            Self::Retry { attempt, max } => write!(f, "Retry {attempt}/{max}"),
            Self::Complete => write!(f, "Complete"),
        }
    }
}

/// A snapshot of imaging progress at one point in time.
#[derive(Debug, Clone)]
pub struct ProgressUpdate {
    pub phase: ImagingPhase,
    pub bytes_finished: u64,
    pub bytes_non_tried: u64,
    pub bytes_non_trimmed: u64,
    pub bytes_non_scraped: u64,
    pub bytes_bad: u64,
    pub device_size: u64,
    pub current_offset: u64,
    pub elapsed: std::time::Duration,
    /// Rolling read throughput over the last ~1 second, in bytes per second.
    /// Zero until at least one full second has elapsed.
    pub read_rate_bps: u64,
    /// Periodic snapshot of the mapfile block list for sector-map rendering.
    /// `None` on most ticks — only populated every 50 calls.
    pub map_snapshot: Option<Vec<crate::mapfile::Block>>,
}

impl ProgressUpdate {
    /// Fraction of device bytes that are `Finished` in [0.0, 1.0].
    /// Use this to display the recovery ratio (how much data has been saved).
    pub fn fraction_done(&self) -> f64 {
        if self.device_size == 0 {
            1.0
        } else {
            self.bytes_finished as f64 / self.device_size as f64
        }
    }

    /// Fraction of the *current pass* that has been processed, in [0.0, 1.0].
    ///
    /// Unlike [`fraction_done`], this advances even when reads fail, giving a
    /// meaningful progress indicator during Trim and Scrape passes where most
    /// sectors are bad and `fraction_done` barely moves.
    ///
    /// - **Copy**   — `(device - non_tried) / device`  (sectors visited so far)
    /// - **Trim**   — `(device - non_tried - non_trimmed) / device`
    /// - **Scrape** — `(device - non_tried - non_trimmed - non_scraped) / device`
    /// - **other**  — falls back to `fraction_done`
    pub fn fraction_pass(&self) -> f64 {
        if self.device_size == 0 {
            return 1.0;
        }
        let d = self.device_size as f64;
        let settled = match self.phase {
            ImagingPhase::Copy => d - self.bytes_non_tried as f64,
            ImagingPhase::Trim | ImagingPhase::Sweep => {
                d - self.bytes_non_tried as f64 - self.bytes_non_trimmed as f64
            }
            ImagingPhase::Scrape | ImagingPhase::Retry { .. } => {
                d - self.bytes_non_tried as f64
                    - self.bytes_non_trimmed as f64
                    - self.bytes_non_scraped as f64
            }
            ImagingPhase::Complete => d,
        };
        (settled / d).clamp(0.0, 1.0)
    }
}

/// Decoupled progress sink. Implemented by the TUI, CLI, and test stubs.
pub trait ProgressReporter: Send {
    /// Called after each block is processed.
    /// Return [`Signal::Cancel`] to abort the current run.
    fn report(&mut self, update: &ProgressUpdate) -> Signal;
}

/// No-op reporter for tests that do not care about progress output.
pub struct NullReporter;

impl ProgressReporter for NullReporter {
    fn report(&mut self, _: &ProgressUpdate) -> Signal {
        Signal::Continue
    }
}

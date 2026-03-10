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
}

impl ProgressUpdate {
    /// Fraction of device bytes that are `Finished` in [0.0, 1.0].
    pub fn fraction_done(&self) -> f64 {
        if self.device_size == 0 {
            1.0
        } else {
            self.bytes_finished as f64 / self.device_size as f64
        }
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

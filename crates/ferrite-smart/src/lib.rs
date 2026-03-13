//! `ferrite-smart` — S.M.A.R.T. diagnostics via `smartctl` CLI wrapper.
//!
//! # Usage
//!
//! ```ignore
//! let thresholds = SmartThresholds::default_config();
//! let (data, verdict) = ferrite_smart::query_and_assess("/dev/sda", None, &thresholds)?;
//! println!("{}: {}", data.device_path, verdict);
//! ```

mod error;
mod parser;
mod runner;
mod thresholds;
mod types;
mod verdict;

// Public API surface
pub use error::{Result, SmartError};
pub use runner::{query, query_and_assess};
pub use thresholds::SmartThresholds;
pub use types::{HealthVerdict, SmartAttribute, SmartData};
pub use verdict::assess_health;

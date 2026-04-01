//! `ferrite-artifact` — forensic artifact scanner.
//!
//! Scans raw block device data for PII artifacts: email addresses, URLs,
//! credit card numbers (Luhn-validated, masked), IBANs, Windows file paths,
//! and US Social Security Numbers.
//!
//! ## Usage
//!
//! ```ignore
//! use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
//! use std::sync::mpsc;
//! use ferrite_artifact::{ArtifactScanConfig, ScanMsg, run_scan};
//!
//! let (tx, rx) = mpsc::channel();
//! let cancel = Arc::new(AtomicBool::new(false));
//! let config = ArtifactScanConfig::default();
//!
//! // Spawn in a background thread:
//! std::thread::spawn(move || run_scan(device, config, tx, cancel));
//!
//! for msg in rx {
//!     match msg {
//!         ScanMsg::HitBatch(hits) => { /* display hits */ }
//!         ScanMsg::Done { total_hits } => break,
//!         _ => {}
//!     }
//! }
//! ```

pub mod engine;
pub mod export;
pub mod scanner;
pub mod scanners;

pub use engine::{run_scan, ArtifactScanConfig, ScanMsg, ScanProgress};
pub use export::write_csv;
pub use scanner::{ArtifactHit, ArtifactKind, ArtifactScanner, Confidence};

//! `ferrite-textcarver` — heuristic text block scanner.
//!
//! Scans raw block device data for contiguous text regions, classifies them
//! by content, and emits each as a candidate recovered file.
//!
//! ## Usage
//!
//! ```ignore
//! use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
//! use std::sync::mpsc;
//! use ferrite_textcarver::{TextScanConfig, TextScanMsg, run_scan};
//!
//! let (tx, rx) = mpsc::channel();
//! let cancel = Arc::new(AtomicBool::new(false));
//! let config = TextScanConfig::default();
//!
//! std::thread::spawn(move || run_scan(device, config, tx, cancel));
//!
//! for msg in rx {
//!     match msg {
//!         TextScanMsg::BlockBatch(blocks) => { /* display blocks */ }
//!         TextScanMsg::Done { total_blocks } => break,
//!         _ => {}
//!     }
//! }
//! ```

pub mod classifier;
pub mod engine;
pub mod export;
pub mod scanner;

pub use engine::run_scan;
pub use export::write_files;
pub use scanner::{TextBlock, TextKind, TextScanConfig, TextScanMsg, TextScanProgress};

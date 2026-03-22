//! Re-exports the shared thermal guard from `ferrite-core`.
//!
//! The implementation now lives in `ferrite_core::thermal` so that the carver,
//! artifact scanner, and text scanner can all share it without depending on
//! `ferrite-imaging`.
pub use ferrite_core::thermal::{ThermalEvent, ThermalGuard, ThermalGuardConfig};

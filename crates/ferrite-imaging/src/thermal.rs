//! Thermal guard — polls a temperature source and pauses imaging when the
//! drive exceeds a configurable threshold.
//!
//! The guard runs in its own background thread and exposes an
//! `Arc<AtomicBool>` pause flag that can be passed directly to
//! [`crate::progress::ProgressReporter`] implementations.  The thread stops
//! automatically when the guard is dropped.
//!
//! The temperature source is a plain `Fn() -> Option<u32>` closure so the
//! guard remains independent of any particular S.M.A.R.T. backend.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

// ── Configuration ─────────────────────────────────────────────────────────────

/// Configuration for the thermal guard.
#[derive(Debug, Clone)]
pub struct ThermalGuardConfig {
    /// Pause imaging when drive temperature reaches or exceeds this value (°C).
    pub pause_above_celsius: u32,
    /// Resume imaging once the drive cools to this value or below (°C).
    pub resume_below_celsius: u32,
    /// How often to poll the temperature source.
    pub poll_interval: Duration,
}

impl Default for ThermalGuardConfig {
    fn default() -> Self {
        Self {
            pause_above_celsius: 55,
            resume_below_celsius: 50,
            poll_interval: Duration::from_secs(60),
        }
    }
}

// ── Event type ────────────────────────────────────────────────────────────────

/// Events emitted by the thermal guard to its caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThermalEvent {
    /// Latest temperature reading in °C.
    Temperature(u32),
    /// Temperature exceeded `pause_above_celsius` — imaging should pause.
    Paused,
    /// Temperature fell to `resume_below_celsius` — imaging can resume.
    Resumed,
}

// ── Guard ─────────────────────────────────────────────────────────────────────

/// A background thread that polls a temperature source and manages a pause flag.
///
/// # Lifecycle
/// The guard thread starts on [`ThermalGuard::start`] and stops when the
/// `ThermalGuard` value is dropped (cancellation via an internal `AtomicBool`).
pub struct ThermalGuard {
    pause: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
}

impl ThermalGuard {
    /// Start the guard thread.
    ///
    /// - `temp_provider` — called each poll interval; returns the current
    ///   temperature in °C, or `None` if unavailable (S.M.A.R.T. offline).
    /// - `config` — thresholds and poll interval.
    /// - `on_event` — callback invoked on every temperature reading and on
    ///   state transitions.  Called from the guard thread; must be `Send`.
    pub fn start<F, C>(temp_provider: F, config: ThermalGuardConfig, on_event: C) -> Self
    where
        F: Fn() -> Option<u32> + Send + 'static,
        C: Fn(ThermalEvent) + Send + 'static,
    {
        let pause = Arc::new(AtomicBool::new(false));
        let cancel = Arc::new(AtomicBool::new(false));

        let pause_t = Arc::clone(&pause);
        let cancel_t = Arc::clone(&cancel);

        std::thread::spawn(move || {
            loop {
                if cancel_t.load(Ordering::Relaxed) {
                    break;
                }

                if let Some(temp) = temp_provider() {
                    on_event(ThermalEvent::Temperature(temp));

                    if temp >= config.pause_above_celsius && !pause_t.load(Ordering::Relaxed) {
                        pause_t.store(true, Ordering::Relaxed);
                        on_event(ThermalEvent::Paused);
                    } else if temp <= config.resume_below_celsius && pause_t.load(Ordering::Relaxed)
                    {
                        pause_t.store(false, Ordering::Relaxed);
                        on_event(ThermalEvent::Resumed);
                    }
                }

                // Sleep in steps of at most 1 s so cancel is checked promptly.
                // For sub-second intervals the single step equals the interval.
                let total_ms = config.poll_interval.as_millis().max(1) as u64;
                let step_ms = total_ms.min(1_000);
                let steps = total_ms / step_ms;
                for _ in 0..steps {
                    std::thread::sleep(Duration::from_millis(step_ms));
                    if cancel_t.load(Ordering::Relaxed) {
                        return;
                    }
                }
            }
        });

        Self { pause, cancel }
    }

    /// A clone of the pause flag suitable for passing to a
    /// [`crate::progress::ProgressReporter`] implementation.
    ///
    /// The flag is `true` while the drive is above `pause_above_celsius` and
    /// `false` otherwise.
    pub fn pause_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.pause)
    }

    /// `true` while imaging should be paused due to high temperature.
    pub fn is_paused(&self) -> bool {
        self.pause.load(Ordering::Relaxed)
    }
}

impl Drop for ThermalGuard {
    fn drop(&mut self) {
        // Signal the guard thread to exit.  It will notice within one second.
        self.cancel.store(true, Ordering::Relaxed);
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Build a guard whose temperature is controlled by an `Arc<AtomicU32>`.
    fn make_guard(
        temp_cell: Arc<std::sync::atomic::AtomicU32>,
        config: ThermalGuardConfig,
    ) -> (ThermalGuard, Arc<Mutex<Vec<ThermalEvent>>>) {
        let events: Arc<Mutex<Vec<ThermalEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let events_c = Arc::clone(&events);

        let guard = ThermalGuard::start(
            move || Some(temp_cell.load(Ordering::Relaxed)),
            config,
            move |e| events_c.lock().unwrap().push(e),
        );
        (guard, events)
    }

    fn fast_config() -> ThermalGuardConfig {
        ThermalGuardConfig {
            poll_interval: Duration::from_millis(20),
            ..ThermalGuardConfig::default()
        }
    }

    #[test]
    fn default_thresholds_are_sensible() {
        let cfg = ThermalGuardConfig::default();
        assert!(cfg.pause_above_celsius > cfg.resume_below_celsius);
        assert_eq!(cfg.pause_above_celsius, 55);
        assert_eq!(cfg.resume_below_celsius, 50);
    }

    #[test]
    fn cool_drive_never_pauses() {
        let temp = Arc::new(std::sync::atomic::AtomicU32::new(40));
        let (guard, events) = make_guard(Arc::clone(&temp), fast_config());
        std::thread::sleep(Duration::from_millis(80));
        drop(guard);

        let ev = events.lock().unwrap();
        assert!(ev.iter().all(|e| *e != ThermalEvent::Paused));
        assert!(!ev.is_empty()); // temperature events emitted
    }

    #[test]
    fn hot_drive_triggers_pause() {
        let temp = Arc::new(std::sync::atomic::AtomicU32::new(60)); // above 55
        let (guard, events) = make_guard(Arc::clone(&temp), fast_config());
        std::thread::sleep(Duration::from_millis(80));

        assert!(guard.is_paused());
        assert!(events.lock().unwrap().contains(&ThermalEvent::Paused));

        drop(guard);
    }

    #[test]
    fn pause_then_resume_lifecycle() {
        let temp = Arc::new(std::sync::atomic::AtomicU32::new(60));
        let (guard, events) = make_guard(Arc::clone(&temp), fast_config());

        // Wait for first pause.
        std::thread::sleep(Duration::from_millis(60));
        assert!(guard.is_paused());

        // Cool down.
        temp.store(45, Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(80));
        assert!(!guard.is_paused());

        let ev = events.lock().unwrap();
        assert!(ev.contains(&ThermalEvent::Paused));
        assert!(ev.contains(&ThermalEvent::Resumed));

        drop(guard);
    }

    #[test]
    fn drop_stops_guard_thread() {
        let temp = Arc::new(std::sync::atomic::AtomicU32::new(40));
        let (guard, events) = make_guard(Arc::clone(&temp), fast_config());
        std::thread::sleep(Duration::from_millis(40));
        let count_before = events.lock().unwrap().len();
        drop(guard);
        std::thread::sleep(Duration::from_millis(60));
        let count_after = events.lock().unwrap().len();
        // After drop, the thread stops — count must not grow significantly.
        assert!(count_after <= count_before + 1);
    }

    #[test]
    fn no_event_when_provider_returns_none() {
        let events: Arc<Mutex<Vec<ThermalEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let events_c = Arc::clone(&events);
        let guard = ThermalGuard::start(
            || None,
            fast_config(),
            move |e| events_c.lock().unwrap().push(e),
        );
        std::thread::sleep(Duration::from_millis(80));
        drop(guard);
        assert!(events.lock().unwrap().is_empty());
    }
}

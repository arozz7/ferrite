//! Unified thermal guard — pauses I/O operations when a drive is thermally
//! stressed, using one or both of two independent signals:
//!
//! 1. **SMART temperature** — direct °C reading from the drive firmware via a
//!    caller-supplied closure.  Works for any SMART-capable drive.
//!
//! 2. **Speed-inferred throttle** — monitors a monotonic bytes-read counter
//!    shared with the scan thread.  When the rolling read rate falls below a
//!    configurable fraction of the established baseline *and stays low* for a
//!    sustained period, it infers thermal throttling even without SMART data.
//!    This is the primary signal for USB drives and bridges that do not expose
//!    temperature attributes.
//!
//! Either signal alone is sufficient to trigger a pause.  Both can be active
//! simultaneously — the guard uses whichever fires first and lifts the pause
//! only when the active signal drops below its resume threshold.
//!
//! The guard runs in a background thread and exposes an `Arc<AtomicBool>` pause
//! flag that scan loops can spin-wait on between chunks.  The thread stops
//! automatically when the guard is dropped (RAII).

use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

use tracing::{debug, info, warn};

// ── Configuration ─────────────────────────────────────────────────────────────

/// Configuration for [`ThermalGuard`].
#[derive(Debug, Clone)]
pub struct ThermalGuardConfig {
    // ── SMART temperature thresholds ──────────────────────────────────────────
    /// Pause when drive temperature reaches or exceeds this value (°C).
    pub pause_above_celsius: u32,
    /// Resume once the drive cools to this value or below (°C).
    pub resume_below_celsius: u32,
    /// How often to poll the temperature source.
    pub poll_interval: Duration,

    // ── Speed-inference thresholds ────────────────────────────────────────────
    /// How long to observe reads before locking in the speed baseline.
    /// Set to `Duration::MAX` to disable speed-based inference entirely.
    pub speed_baseline_window: Duration,
    /// If rolling speed drops below `baseline × (speed_throttle_pct / 100)`,
    /// it is considered a thermal stall.  Range: 1–99.
    pub speed_throttle_pct: u8,
    /// How long speed must stay below the throttle threshold before pausing.
    /// A single bad-sector stall (≤30 s ERC) never triggers this.
    pub speed_sustain: Duration,
    /// How often the guard thread samples the bytes-read counter to compute
    /// the current rolling speed.
    pub speed_sample_interval: Duration,
}

impl Default for ThermalGuardConfig {
    fn default() -> Self {
        Self {
            pause_above_celsius: 55,
            resume_below_celsius: 50,
            poll_interval: Duration::from_secs(60),
            speed_baseline_window: Duration::from_secs(90),
            speed_throttle_pct: 50,
            speed_sustain: Duration::from_secs(60),
            speed_sample_interval: Duration::from_secs(5),
        }
    }
}

// ── Events ────────────────────────────────────────────────────────────────────

/// Events emitted by [`ThermalGuard`] to the caller's `on_event` callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThermalEvent {
    /// Latest SMART temperature reading (°C).
    Temperature(u32),
    /// SMART temperature exceeded `pause_above_celsius` — pausing I/O.
    Paused,
    /// SMART temperature fell to `resume_below_celsius` — resuming I/O.
    Resumed,
    /// Speed-baseline established; value is bytes/sec.
    SpeedBaseline(u64),
    /// Speed-inferred thermal throttle triggered — pausing I/O.
    SpeedThrottle,
    /// Speed recovered above the throttle threshold — resuming I/O.
    SpeedResumed,
}

// ── Guard ─────────────────────────────────────────────────────────────────────

/// Background thread that monitors drive health and manages a shared pause flag.
///
/// # Signals
///
/// - Pass `|| None` as `temp_provider` to disable SMART monitoring.
/// - Pass `None` as `bytes_read` to disable speed-based inference.
/// - Passing both as no-ops makes the guard a compile-time zero-cost stub
///   (the thread runs but never sets the pause flag).
///
/// # Lifecycle
///
/// The guard thread starts on construction and stops within ~1 s of the
/// `ThermalGuard` value being dropped.
pub struct ThermalGuard {
    pause: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
}

impl ThermalGuard {
    /// Start the thermal guard thread.
    ///
    /// - `temp_provider` — closure returning the current drive temperature in
    ///   °C, or `None` when SMART data is unavailable.
    /// - `bytes_read` — monotonic counter of bytes read by the scan thread.
    ///   Pass `None` to disable speed-based inference.
    /// - `config` — thresholds and intervals.
    /// - `on_event` — callback invoked for every notable event (temperature
    ///   readings, pause/resume transitions).  Called from the guard thread.
    pub fn start<F, C>(
        temp_provider: F,
        bytes_read: Option<Arc<AtomicU64>>,
        config: ThermalGuardConfig,
        on_event: C,
    ) -> Self
    where
        F: Fn() -> Option<u32> + Send + 'static,
        C: Fn(ThermalEvent) + Send + 'static,
    {
        let pause = Arc::new(AtomicBool::new(false));
        let cancel = Arc::new(AtomicBool::new(false));

        let pause_t = Arc::clone(&pause);
        let cancel_t = Arc::clone(&cancel);

        std::thread::spawn(move || {
            guard_thread(
                temp_provider,
                bytes_read,
                config,
                on_event,
                pause_t,
                cancel_t,
            );
        });

        Self { pause, cancel }
    }

    /// A clone of the pause flag for passing to scan loops.
    ///
    /// The flag is `true` while the drive is considered thermally stressed and
    /// `false` otherwise.  Scan loops should spin-wait on this between chunks.
    pub fn pause_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.pause)
    }

    /// `true` while the guard has the pause flag set.
    pub fn is_paused(&self) -> bool {
        self.pause.load(Ordering::Relaxed)
    }
}

impl Drop for ThermalGuard {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

// ── Guard thread implementation ───────────────────────────────────────────────

fn guard_thread<F, C>(
    temp_provider: F,
    bytes_read: Option<Arc<AtomicU64>>,
    config: ThermalGuardConfig,
    on_event: C,
    pause: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
) where
    F: Fn() -> Option<u32> + Send + 'static,
    C: Fn(ThermalEvent) + Send + 'static,
{
    // ── Speed-inference state ─────────────────────────────────────────────────
    // Phase 1: collect samples until speed_baseline_window elapses.
    // Phase 2: compare rolling rate against baseline.
    let mut baseline_bps: Option<u64> = None;
    let mut baseline_samples: Vec<u64> = Vec::new();
    let baseline_start = Instant::now();
    let mut last_sample_instant = Instant::now();
    let mut last_bytes: u64 = 0;
    let mut slow_since: Option<Instant> = None;
    let mut paused_by_speed = false;

    let speed_sample_ms = config
        .speed_sample_interval
        .as_millis()
        .max(1)
        .min(u64::MAX as u128) as u64;
    let temp_poll_ms = config
        .poll_interval
        .as_millis()
        .max(1)
        .min(u64::MAX as u128) as u64;

    // Sleep granularity: the shorter of the two configured intervals, capped
    // at 1000 ms so the cancel flag is checked at least once per second.
    let tick_ms = speed_sample_ms.min(temp_poll_ms).min(1000);

    // Run the two polling loops at different rates by tracking independent
    // elapsed times rather than nested sleep loops.
    let mut last_temp_poll = Instant::now();

    loop {
        std::thread::sleep(Duration::from_millis(tick_ms));
        if cancel.load(Ordering::Relaxed) {
            break;
        }

        let now = Instant::now();

        // ── Speed sampling ────────────────────────────────────────────────────
        if let Some(ref counter) = bytes_read {
            let elapsed_sample = now.duration_since(last_sample_instant);
            if elapsed_sample.as_millis() as u64 >= speed_sample_ms {
                let current_bytes = counter.load(Ordering::Relaxed);
                let delta = current_bytes.saturating_sub(last_bytes);
                let rate_bps = if elapsed_sample.as_secs_f64() > 0.0 {
                    (delta as f64 / elapsed_sample.as_secs_f64()) as u64
                } else {
                    0
                };
                last_bytes = current_bytes;
                last_sample_instant = now;

                if baseline_bps.is_none() {
                    // Baseline accumulation phase — only count non-zero samples
                    // (zero means scan hasn't started or is already paused).
                    if rate_bps > 0 {
                        baseline_samples.push(rate_bps);
                    }
                    if now.duration_since(baseline_start) >= config.speed_baseline_window
                        && !baseline_samples.is_empty()
                    {
                        // Use the median of samples to avoid skew from early
                        // burst reads or brief bad-sector stalls.
                        let mut sorted = baseline_samples.clone();
                        sorted.sort_unstable();
                        let median = sorted[sorted.len() / 2];
                        baseline_bps = Some(median);
                        info!(baseline_bps = median, "thermal: speed baseline established");
                        on_event(ThermalEvent::SpeedBaseline(median));
                    }
                } else if !paused_by_speed {
                    let baseline = baseline_bps.unwrap();
                    let threshold = baseline * config.speed_throttle_pct as u64 / 100;

                    if rate_bps >= threshold {
                        // Confirmed fast — reset the slow-sustain timer.
                        slow_since = None;
                    } else {
                        // rate < threshold (including 0): the drive is either slow
                        // or idle.  Accumulate the slow-sustain timer.  The
                        // `!pause.load()` guard prevents triggering during a
                        // user-initiated or back-pressure pause (where rate is
                        // legitimately 0 because the scan thread is spin-waiting).
                        if let Some(since) = slow_since {
                            if now.duration_since(since) >= config.speed_sustain
                                && !pause.load(Ordering::Relaxed)
                            {
                                warn!(
                                    rate_bps,
                                    baseline_bps,
                                    "thermal: sustained speed drop — pausing for drive rest"
                                );
                                pause.store(true, Ordering::Relaxed);
                                paused_by_speed = true;
                                on_event(ThermalEvent::SpeedThrottle);
                            }
                        } else {
                            slow_since = Some(now);
                            debug!(
                                rate_bps,
                                threshold, "thermal: speed drop detected — monitoring"
                            );
                        }
                    }
                } else {
                    // Currently paused by speed — check for recovery.
                    let baseline = baseline_bps.unwrap();
                    let resume_threshold = baseline * config.speed_throttle_pct as u64 / 100;
                    // Resume when rate climbs back above the threshold, or
                    // after a generous rest (10 × speed_sustain) to ensure the
                    // drive has had time to cool before re-checking.
                    let rested = slow_since
                        .map(|s| now.duration_since(s) >= config.speed_sustain * 10)
                        .unwrap_or(true);
                    if rate_bps >= resume_threshold || rested {
                        pause.store(false, Ordering::Relaxed);
                        paused_by_speed = false;
                        slow_since = None;
                        info!("thermal: speed recovered — resuming");
                        on_event(ThermalEvent::SpeedResumed);
                        // Reset baseline to re-measure from the current state
                        // (fragmented drive regions may have legitimately lower
                        // sustained speed than the initial baseline).
                        baseline_bps = None;
                        baseline_samples.clear();
                        last_bytes = counter.load(Ordering::Relaxed);
                        last_sample_instant = now;
                    }
                }
            }
        }

        // ── SMART temperature polling ─────────────────────────────────────────
        if now.duration_since(last_temp_poll).as_millis() as u64 >= temp_poll_ms {
            last_temp_poll = now;

            if let Some(temp) = temp_provider() {
                on_event(ThermalEvent::Temperature(temp));

                if temp >= config.pause_above_celsius && !pause.load(Ordering::Relaxed) {
                    warn!(temp, "thermal: temperature threshold reached — pausing");
                    pause.store(true, Ordering::Relaxed);
                    on_event(ThermalEvent::Paused);
                } else if temp <= config.resume_below_celsius
                    && pause.load(Ordering::Relaxed)
                    && !paused_by_speed
                {
                    info!(temp, "thermal: temperature recovered — resuming");
                    pause.store(false, Ordering::Relaxed);
                    on_event(ThermalEvent::Resumed);
                }
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn fast_config() -> ThermalGuardConfig {
        ThermalGuardConfig {
            poll_interval: Duration::from_millis(20),
            speed_baseline_window: Duration::from_millis(50),
            speed_sustain: Duration::from_millis(60),
            speed_sample_interval: Duration::from_millis(10),
            ..ThermalGuardConfig::default()
        }
    }

    fn events_guard(
        temp: Arc<std::sync::atomic::AtomicU32>,
        bytes_read: Option<Arc<AtomicU64>>,
        config: ThermalGuardConfig,
    ) -> (ThermalGuard, Arc<Mutex<Vec<ThermalEvent>>>) {
        let events: Arc<Mutex<Vec<ThermalEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let events_c = Arc::clone(&events);
        let guard = ThermalGuard::start(
            move || Some(temp.load(Ordering::Relaxed)),
            bytes_read,
            config,
            move |e| events_c.lock().unwrap().push(e),
        );
        (guard, events)
    }

    #[test]
    fn default_thresholds_are_sensible() {
        let cfg = ThermalGuardConfig::default();
        assert!(cfg.pause_above_celsius > cfg.resume_below_celsius);
        assert_eq!(cfg.pause_above_celsius, 55);
        assert_eq!(cfg.resume_below_celsius, 50);
        assert_eq!(cfg.speed_throttle_pct, 50);
    }

    #[test]
    fn cool_drive_never_pauses() {
        let temp = Arc::new(std::sync::atomic::AtomicU32::new(40));
        let (guard, events) = events_guard(Arc::clone(&temp), None, fast_config());
        std::thread::sleep(Duration::from_millis(100));
        drop(guard);

        let ev = events.lock().unwrap();
        assert!(ev.iter().all(|e| *e != ThermalEvent::Paused));
        assert!(!ev.is_empty());
    }

    #[test]
    fn hot_drive_triggers_pause() {
        let temp = Arc::new(std::sync::atomic::AtomicU32::new(60));
        let (guard, events) = events_guard(Arc::clone(&temp), None, fast_config());
        std::thread::sleep(Duration::from_millis(100));

        assert!(guard.is_paused());
        assert!(events.lock().unwrap().contains(&ThermalEvent::Paused));
        drop(guard);
    }

    #[test]
    fn pause_then_resume_lifecycle() {
        let temp = Arc::new(std::sync::atomic::AtomicU32::new(60));
        let (guard, events) = events_guard(Arc::clone(&temp), None, fast_config());

        std::thread::sleep(Duration::from_millis(60));
        assert!(guard.is_paused());

        temp.store(45, Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(100));
        assert!(!guard.is_paused());

        let ev = events.lock().unwrap();
        assert!(ev.contains(&ThermalEvent::Paused));
        assert!(ev.contains(&ThermalEvent::Resumed));
        drop(guard);
    }

    #[test]
    fn drop_stops_guard_thread() {
        let temp = Arc::new(std::sync::atomic::AtomicU32::new(40));
        let (guard, events) = events_guard(Arc::clone(&temp), None, fast_config());
        std::thread::sleep(Duration::from_millis(40));
        let count_before = events.lock().unwrap().len();
        drop(guard);
        std::thread::sleep(Duration::from_millis(80));
        let count_after = events.lock().unwrap().len();
        assert!(count_after <= count_before + 2);
    }

    #[test]
    fn no_temp_event_when_provider_returns_none() {
        let events: Arc<Mutex<Vec<ThermalEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let events_c = Arc::clone(&events);
        let guard = ThermalGuard::start(
            || None,
            None,
            fast_config(),
            move |e| events_c.lock().unwrap().push(e),
        );
        std::thread::sleep(Duration::from_millis(100));
        drop(guard);
        // No temperature events (provider returns None), no speed events (no counter).
        assert!(events.lock().unwrap().is_empty());
    }

    #[test]
    fn speed_baseline_established() {
        let temp = Arc::new(std::sync::atomic::AtomicU32::new(40));
        let counter = Arc::new(AtomicU64::new(0));
        let counter_t = Arc::clone(&counter);

        // Simulate a steady 10 MB/s reader.
        std::thread::spawn(move || {
            let chunk = 10 * 1024 * 1024u64 / 10; // 1 MB per 100 ms
            loop {
                std::thread::sleep(Duration::from_millis(100));
                counter_t.fetch_add(chunk, Ordering::Relaxed);
            }
        });

        let (guard, events) =
            events_guard(Arc::clone(&temp), Some(Arc::clone(&counter)), fast_config());

        // Wait for baseline window to pass (50 ms in fast_config).
        std::thread::sleep(Duration::from_millis(400));
        drop(guard);

        let ev = events.lock().unwrap();
        assert!(
            ev.iter()
                .any(|e| matches!(e, ThermalEvent::SpeedBaseline(_))),
            "expected SpeedBaseline event, got: {ev:?}"
        );
    }

    #[test]
    fn speed_throttle_triggers_after_sustain() {
        let temp = Arc::new(std::sync::atomic::AtomicU32::new(40));
        let counter = Arc::new(AtomicU64::new(0));
        let counter_t = Arc::clone(&counter);

        // Phase 1: fast reader (builds baseline at ~1 MB/s).
        // Use 10 ms sleep intervals to match sample_interval so every sample
        // sees a non-zero rate (avoids zero-rate samples resetting slow_since).
        let phase = Arc::new(AtomicBool::new(false)); // false=fast, true=slow
        let phase_t = Arc::clone(&phase);
        std::thread::spawn(move || {
            loop {
                let chunk = if phase_t.load(Ordering::Relaxed) {
                    100u64 // ~10 KB/s — well below 50% of ~1 MB/s baseline
                } else {
                    10 * 1024u64 // 10 KB per 10 ms → ~1 MB/s
                };
                std::thread::sleep(Duration::from_millis(10));
                counter_t.fetch_add(chunk, Ordering::Relaxed);
            }
        });

        let (guard, events) =
            events_guard(Arc::clone(&temp), Some(Arc::clone(&counter)), fast_config());

        // Let baseline establish (50 ms window + margin).
        std::thread::sleep(Duration::from_millis(200));
        assert!(!guard.is_paused());

        // Switch to slow — must sustain for speed_sustain (60 ms) before pause.
        // Wait long enough to trigger (sustain=60ms) but short enough that
        // the time-based rest (10 × sustain = 600ms) hasn't fired yet.
        phase.store(true, Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(200));

        assert!(
            guard.is_paused(),
            "expected speed throttle pause after sustained drop"
        );
        let ev = events.lock().unwrap();
        assert!(ev.contains(&ThermalEvent::SpeedThrottle));
        drop(guard);
    }

    #[test]
    fn brief_stall_does_not_trigger_pause() {
        let temp = Arc::new(std::sync::atomic::AtomicU32::new(40));
        let counter = Arc::new(AtomicU64::new(0));
        let counter_t = Arc::clone(&counter);

        // Steady reader — we'll pause the counter manually for less than sustain_ms.
        let stall = Arc::new(AtomicBool::new(false));
        let stall_t = Arc::clone(&stall);
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(20));
            if !stall_t.load(Ordering::Relaxed) {
                counter_t.fetch_add(10 * 1024 * 1024u64 / 50, Ordering::Relaxed);
            }
        });

        let (guard, events) =
            events_guard(Arc::clone(&temp), Some(Arc::clone(&counter)), fast_config());

        // Build baseline.
        std::thread::sleep(Duration::from_millis(150));

        // Stall for only 30 ms — below the 60 ms sustain threshold.
        stall.store(true, Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(30));
        stall.store(false, Ordering::Relaxed);

        // Give the guard a chance to sample.
        std::thread::sleep(Duration::from_millis(100));
        drop(guard);

        assert!(!events
            .lock()
            .unwrap()
            .contains(&ThermalEvent::SpeedThrottle));
    }
}

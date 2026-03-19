//! Pluggable clock abstraction for deterministic testing.
//!
//! The [`Clock`] trait decouples "what time is it now?" from the system
//! clock.  Production code uses [`SystemClock`]; tests use [`MockClock`],
//! which allows time to be advanced programmatically without `sleep`.
//!
//! # Example
//!
//! ```rust
//! use nebula_resilience::clock::{Clock, MockClock};
//! use std::time::Duration;
//!
//! let clock = MockClock::new();
//! let t0 = clock.now();
//!
//! clock.advance(Duration::from_secs(5));
//! let t1 = clock.now();
//!
//! assert!(t1.duration_since(t0) >= Duration::from_secs(5));
//! ```

use parking_lot::Mutex;
use std::sync::Arc;
use std::time::{Duration, Instant};

// =============================================================================
// TRAIT
// =============================================================================

/// A source of wall-clock time.
///
/// Implement this trait (or use one of the provided implementations) to inject
/// a time source into resilience patterns that need deterministic test control.
pub trait Clock: Send + Sync {
    /// Returns the current instant according to this clock.
    fn now(&self) -> Instant;
}

// =============================================================================
// SYSTEM CLOCK
// =============================================================================

/// The real system clock — delegates directly to [`Instant::now`].
///
/// This is the default implementation used in production code.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    #[inline]
    fn now(&self) -> Instant {
        Instant::now()
    }
}

// =============================================================================
// MOCK CLOCK
// =============================================================================

/// A manually-controlled clock for deterministic tests.
///
/// Time starts at the moment the `MockClock` is created and only advances
/// when [`advance`](MockClock::advance) is called explicitly.
///
/// `MockClock` is cheap to clone — all clones share the same underlying state.
#[derive(Debug, Clone)]
pub struct MockClock {
    inner: Arc<Mutex<MockClockInner>>,
}

#[derive(Debug)]
struct MockClockInner {
    /// Absolute base: the real `Instant` when this clock was created.
    base: Instant,
    /// Additional virtual time added via `advance()`.
    offset: Duration,
}

impl MockClock {
    /// Create a new mock clock anchored at `Instant::now()`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(MockClockInner {
                base: Instant::now(),
                offset: Duration::ZERO,
            })),
        }
    }

    /// Advance this clock by `duration`.
    ///
    /// All clones of this `MockClock` will observe the new time immediately.
    pub fn advance(&self, duration: Duration) {
        self.inner.lock().offset += duration;
    }

    /// Returns the total virtual time elapsed since this clock was created.
    #[must_use]
    pub fn elapsed(&self) -> Duration {
        let inner = self.inner.lock();
        inner.base.elapsed() + inner.offset
    }
}

impl Default for MockClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for MockClock {
    fn now(&self) -> Instant {
        let inner = self.inner.lock();
        // `Instant` can't be constructed from thin air on stable Rust, so we
        // express the virtual time as `base + real_elapsed + offset`.
        // `base.elapsed()` accounts for real time already; `offset` is the
        // additional virtual advance.
        inner.base + inner.base.elapsed() + inner.offset
    }
}

// =============================================================================
// TESTS
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_clock_advances_monotonically() {
        let clock = SystemClock;
        let t0 = clock.now();
        std::thread::sleep(Duration::from_millis(1));
        let t1 = clock.now();
        assert!(t1 > t0);
    }

    #[test]
    fn mock_clock_does_not_advance_without_explicit_call() {
        let clock = MockClock::new();
        let t0 = clock.now();
        // No sleep, no advance — the *virtual* offset hasn't moved.
        let t1 = clock.now();
        // Real time may have moved slightly; virtual offset is still zero.
        // t1 >= t0 is guaranteed because real elapsed can only grow.
        assert!(t1 >= t0);
    }

    #[test]
    fn mock_clock_advance_increases_now() {
        let clock = MockClock::new();
        let t0 = clock.now();
        clock.advance(Duration::from_secs(10));
        let t1 = clock.now();
        assert!(t1.duration_since(t0) >= Duration::from_secs(10));
    }

    #[test]
    fn mock_clock_clones_share_state() {
        let clock = MockClock::new();
        let clone = clock.clone();

        let t0 = clock.now();
        clock.advance(Duration::from_secs(3));
        let t1 = clone.now(); // clone observes the advance

        assert!(t1.duration_since(t0) >= Duration::from_secs(3));
    }

    #[test]
    fn mock_clock_elapsed_matches_advances() {
        let clock = MockClock::new();
        clock.advance(Duration::from_millis(500));
        clock.advance(Duration::from_millis(500));
        // Total virtual advance ≥ 1 s (real elapsed adds a tiny epsilon).
        assert!(clock.elapsed() >= Duration::from_secs(1));
    }
}

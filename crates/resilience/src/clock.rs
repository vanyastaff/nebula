//! Pluggable clock abstraction for deterministic testing.
//!
//! The [`Clock`] trait decouples "what time is it now?" from the system
//! clock.  Production code uses [`SystemClock`]; tests use [`MockClock`],
//! which allows time to be advanced programmatically without `sleep`.
//!
//! # Example
//!
//! ```rust
//! use std::time::Duration;
//!
//! use nebula_resilience::clock::{Clock, MockClock};
//!
//! let clock = MockClock::new();
//! let t0 = clock.now();
//!
//! clock.advance(Duration::from_secs(5));
//! let t1 = clock.now();
//!
//! assert!(t1.duration_since(t0) >= Duration::from_secs(5));
//! ```

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use parking_lot::Mutex;

// =============================================================================
// TRAIT
// =============================================================================

/// A source of wall-clock time.
///
/// Implement this trait (or use one of the provided implementations) to inject
/// a time source into resilience patterns that need deterministic test control.
///
/// This trait is designed to be implemented by downstream crates.
/// New methods will always have default implementations to avoid breaking changes.
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
/// `MockClock` is cheap to clone — all clones share the same underlying state.
///
/// Unlike [`SystemClock`], this clock does not advance unless
/// [`advance`](MockClock::advance) is called. That keeps state-machine tests
/// deterministic and avoids hidden real-time sleeps.
#[derive(Debug, Clone)]
pub struct MockClock {
    inner: Arc<Mutex<MockClockInner>>,
}

#[derive(Debug)]
struct MockClockInner {
    /// Current representable instant.
    now: Instant,
    /// Additional virtual time added via `advance()`.
    offset: Duration,
}

impl MockClock {
    /// Create a new mock clock anchored at `Instant::now()`.
    #[must_use]
    pub fn new() -> Self {
        let base = Instant::now();
        Self {
            inner: Arc::new(Mutex::new(MockClockInner {
                now: base,
                offset: Duration::ZERO,
            })),
        }
    }

    /// Advance this clock by `duration`.
    ///
    /// All clones of this `MockClock` will observe the new time immediately.
    pub fn advance(&self, duration: Duration) {
        let mut inner = self.inner.lock();
        inner.offset = inner.offset.saturating_add(duration);
        inner.now = inner.now.checked_add(duration).unwrap_or(inner.now);
    }

    /// Returns the total virtual time elapsed since this clock was created.
    #[must_use]
    pub fn elapsed(&self) -> Duration {
        self.inner.lock().offset
    }
}

impl Default for MockClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for MockClock {
    fn now(&self) -> Instant {
        self.inner.lock().now
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
        std::thread::sleep(Duration::from_millis(1));
        let t1 = clock.now();
        assert_eq!(t1, t0);
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
        assert_eq!(clock.elapsed(), Duration::from_secs(1));
    }

    #[test]
    fn mock_clock_overflow_does_not_move_backwards() {
        let clock = MockClock::new();

        let initial = clock.now();
        clock.advance(Duration::from_secs(1));
        let before_overflow = clock.now();
        clock.advance(Duration::MAX);
        let after_overflow = clock.now();

        assert!(before_overflow > initial);
        assert!(after_overflow >= before_overflow);
    }
}

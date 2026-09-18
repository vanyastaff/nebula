//! Pluggable monotonic instant source for deterministic testing.
//!
//! [`InstantSource`] decouples "what instant is it now?" from the process
//! monotonic clock. Production code uses [`SystemInstant`]; tests use
//! [`MockInstant`], which advances programmatically without `sleep`.
//!
//! The name deliberately avoids `Clock`: the workspace already has
//! `nebula_core::accessor::Clock` (wall time plus monotonic) and
//! `nebula_action::webhook::Clock`. Those carry wall-clock time; this trait
//! carries only `std::time::Instant`, matching Java's
//! [`java.time.InstantSource`](https://docs.oracle.com/en/java/javase/17/docs/api/java.base/java/time/InstantSource.html)
//! in scope.
//!
//! # Example
//!
//! ```rust
//! use std::time::Duration;
//!
//! use nebula_resilience::clock::{InstantSource, MockInstant};
//!
//! let clock = MockInstant::new();
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

/// A source of monotonic instants.
///
/// Implement this trait (or use one of the provided implementations) to inject
/// a time source into resilience patterns that need deterministic test control.
///
/// This trait is designed to be implemented by downstream crates.
/// New methods will always have default implementations to avoid breaking changes.
///
/// See [`MockInstant`] for a ready-made deterministic implementation and the
/// [module documentation](self) for an example.
pub trait InstantSource: Send + Sync {
    /// Returns the current instant according to this source.
    fn now(&self) -> Instant;
}

// =============================================================================
// SYSTEM INSTANT
// =============================================================================

/// The real monotonic clock — delegates directly to [`Instant::now`].
///
/// This is the default implementation used in production code.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemInstant;

impl InstantSource for SystemInstant {
    #[inline]
    fn now(&self) -> Instant {
        Instant::now()
    }
}

// =============================================================================
// MOCK INSTANT
// =============================================================================

/// A manually-controlled instant source for deterministic tests.
///
/// `MockInstant` is cheap to clone — all clones share the same underlying
/// state.
///
/// Unlike [`SystemInstant`], this source does not advance unless
/// [`advance`](MockInstant::advance) is called. That keeps state-machine tests
/// deterministic and avoids hidden real-time sleeps.
#[derive(Debug, Clone)]
pub struct MockInstant {
    inner: Arc<Mutex<MockInstantInner>>,
}

#[derive(Debug)]
struct MockInstantInner {
    /// Current representable instant.
    now: Instant,
    /// Additional virtual time added via `advance()`.
    offset: Duration,
}

impl MockInstant {
    /// Create a new mock instant source anchored at `Instant::now()`.
    #[must_use]
    pub fn new() -> Self {
        let base = Instant::now();
        Self {
            inner: Arc::new(Mutex::new(MockInstantInner {
                now: base,
                offset: Duration::ZERO,
            })),
        }
    }

    /// Advances this source by `duration`.
    ///
    /// All clones of this `MockInstant` will observe the new time immediately.
    pub fn advance(&self, duration: Duration) {
        let mut inner = self.inner.lock();
        inner.offset = inner.offset.saturating_add(duration);
        inner.now = inner.now.checked_add(duration).unwrap_or(inner.now);
    }

    /// Returns the total virtual time elapsed since this source was created.
    #[must_use]
    pub fn elapsed(&self) -> Duration {
        self.inner.lock().offset
    }
}

impl Default for MockInstant {
    fn default() -> Self {
        Self::new()
    }
}

impl InstantSource for MockInstant {
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
    fn system_instant_advances_monotonically() {
        let clock = SystemInstant;
        let t0 = clock.now();
        std::thread::sleep(Duration::from_millis(1));
        let t1 = clock.now();
        assert!(t1 > t0);
    }

    #[test]
    fn mock_instant_does_not_advance_without_explicit_call() {
        let clock = MockInstant::new();
        let t0 = clock.now();
        std::thread::sleep(Duration::from_millis(1));
        let t1 = clock.now();
        assert_eq!(t1, t0);
    }

    #[test]
    fn mock_instant_advance_increases_now() {
        let clock = MockInstant::new();
        let t0 = clock.now();
        clock.advance(Duration::from_secs(10));
        let t1 = clock.now();
        assert!(t1.duration_since(t0) >= Duration::from_secs(10));
    }

    #[test]
    fn mock_instant_clones_share_state() {
        let clock = MockInstant::new();
        let clone = clock.clone();

        let t0 = clock.now();
        clock.advance(Duration::from_secs(3));
        let t1 = clone.now(); // clone observes the advance

        assert!(t1.duration_since(t0) >= Duration::from_secs(3));
    }

    #[test]
    fn mock_instant_elapsed_matches_advances() {
        let clock = MockInstant::new();
        clock.advance(Duration::from_millis(500));
        clock.advance(Duration::from_millis(500));
        assert_eq!(clock.elapsed(), Duration::from_secs(1));
    }

    #[test]
    fn mock_instant_overflow_does_not_move_backwards() {
        let clock = MockInstant::new();

        let initial = clock.now();
        clock.advance(Duration::from_secs(1));
        let before_overflow = clock.now();
        clock.advance(Duration::MAX);
        let after_overflow = clock.now();

        assert!(before_overflow > initial);
        assert!(after_overflow >= before_overflow);
    }
}

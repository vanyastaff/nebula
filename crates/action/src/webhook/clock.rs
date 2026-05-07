//! [`Clock`] abstraction for replay-window enforcement on webhook
//! signatures.
//!
//! [`SignaturePolicy`](super::SignaturePolicy) verifies a request's
//! timestamp against a configured replay window before delegating to
//! HMAC math. Production code uses [`SystemClock`] (delegating to
//! [`SystemTime::now`]); tests use [`MockClock`] to advance time
//! deterministically.
//!
//! The transport holds an `Arc<dyn Clock>` and threads it into
//! [`RequiredPolicy::verify_with`](super::RequiredPolicy::verify_with)
//! so both programmatic and slug-routed webhook paths get replay
//! protection through the same codepath.
//!
//! # Determinism
//!
//! [`MockClock`] uses an `AtomicU64` of Unix-epoch seconds so multiple
//! tests can share a single instance without locking. `set` and
//! `advance` are atomic and serializable.

use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// Time source for webhook replay-window enforcement.
///
/// Held as `Arc<dyn Clock>` by the transport so production and test
/// code share a single trait surface. Implementors must be cheap to
/// call (no syscall per webhook in production beyond a single
/// [`SystemTime::now`]).
///
/// # Why not `chrono::Utc::now`
///
/// We avoid pulling `chrono` into `nebula-action` for one method.
/// [`SystemTime`] is sufficient for replay windows measured in
/// minutes; the transport layer translates to RFC 3339 only when
/// emitting RFC 9457 problem+json responses.
pub trait Clock: Send + Sync + 'static {
    /// Current wall-clock time. Returned as [`SystemTime`] so
    /// callers can subtract [`Duration`]s to compute replay windows
    /// without leaking the clock's internal representation.
    fn now(&self) -> SystemTime;
}

/// Production [`Clock`] backed by [`SystemTime::now`].
///
/// Use one shared `Arc<SystemClock>` — the struct is zero-sized and
/// `Clone` is just `Arc::clone`.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl SystemClock {
    /// Construct a new [`SystemClock`]. Cheap (zero-sized).
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Clock for SystemClock {
    #[inline]
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}

/// Deterministic [`Clock`] for replay-window tests.
///
/// Stores Unix-epoch seconds in an [`AtomicU64`]; [`set`](Self::set)
/// and [`advance`](Self::advance) are atomic so multiple test tasks
/// can share a single instance.
///
/// # Example
///
/// ```ignore
/// let clock = Arc::new(MockClock::at_unix_secs(1_700_000_000));
/// // ... fire webhook ...
/// clock.advance(Duration::from_secs(310)); // > 5 min replay window
/// // verify_with should now reject
/// ```
#[derive(Debug)]
pub struct MockClock {
    unix_secs: AtomicU64,
}

impl MockClock {
    /// Construct a [`MockClock`] anchored at the given Unix-epoch
    /// seconds.
    #[must_use]
    pub fn at_unix_secs(secs: u64) -> Self {
        Self {
            unix_secs: AtomicU64::new(secs),
        }
    }

    /// Construct a [`MockClock`] anchored at the current
    /// [`SystemTime::now`]. Use when you want a realistic timestamp
    /// for tests that interact with both signed payloads and
    /// system-supplied timestamps.
    #[must_use]
    pub fn at_now() -> Self {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Self::at_unix_secs(secs)
    }

    /// Replace the clock's current time with the given Unix-epoch
    /// seconds.
    pub fn set(&self, secs: u64) {
        self.unix_secs.store(secs, Ordering::SeqCst);
    }

    /// Advance the clock by `delta`. Sub-second components are
    /// truncated.
    pub fn advance(&self, delta: Duration) {
        self.unix_secs.fetch_add(delta.as_secs(), Ordering::SeqCst);
    }

    /// Convenience for `Arc::new(self)` — most call sites store
    /// `Arc<dyn Clock>`.
    #[must_use]
    pub fn into_arc(self) -> Arc<dyn Clock> {
        Arc::new(self)
    }
}

impl Default for MockClock {
    /// Anchored at Unix epoch zero. Use [`MockClock::at_now`] or
    /// [`MockClock::at_unix_secs`] for realistic anchoring.
    fn default() -> Self {
        Self::at_unix_secs(0)
    }
}

impl Clock for MockClock {
    fn now(&self) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(self.unix_secs.load(Ordering::SeqCst))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_clock_returns_recent_time() {
        let clock = SystemClock;
        let now = clock.now();
        let real = SystemTime::now();
        let diff = real
            .duration_since(now)
            .or_else(|e| Ok::<_, std::time::SystemTimeError>(e.duration()))
            .unwrap();
        assert!(diff.as_secs() < 2, "system clock drifted {diff:?}");
    }

    #[test]
    fn mock_clock_set_and_advance() {
        let clock = MockClock::at_unix_secs(1_000);
        let t0 = clock.now();

        clock.advance(Duration::from_mins(1));
        let t1 = clock.now();
        let diff = t1.duration_since(t0).unwrap();
        assert_eq!(diff.as_secs(), 60);

        clock.set(2_000);
        let t2 = clock.now();
        let from_epoch = t2.duration_since(UNIX_EPOCH).unwrap();
        assert_eq!(from_epoch.as_secs(), 2_000);
    }

    #[test]
    fn mock_clock_at_now_close_to_real() {
        let clock = MockClock::at_now();
        let now = clock.now();
        let real = SystemTime::now();
        let diff = real.duration_since(now).unwrap();
        assert!(diff.as_secs() < 2);
    }
}

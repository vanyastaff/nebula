//! Acquire-time options for resource leasing.
//!
//! [`AcquireOptions`] lets callers communicate a deadline to the resource
//! subsystem. Older drafts also carried `intent` and `tags` fields reserved
//! for engine integration; those were removed since no consumer ever wired
//! them. If the need returns, the relevant surface is added back via a new
//! spec.

use std::time::{Duration, Instant};

/// Options passed to acquire calls.
///
/// # Examples
///
/// ```
/// use std::time::{Duration, Instant};
///
/// use nebula_resource::AcquireOptions;
///
/// let opts = AcquireOptions::default().with_deadline(Instant::now() + Duration::from_secs(5));
/// assert!(opts.deadline.is_some());
/// ```
///
/// `#[non_exhaustive]`: like [`ManagerConfig`](crate::ManagerConfig) /
/// [`ShutdownConfig`](crate::ShutdownConfig) /
/// [`RegisterOptions`](crate::RegisterOptions), new tuning fields must be
/// additive without a breaking struct-literal change. Construct via
/// [`AcquireOptions::default`] then the `with_*` setters.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct AcquireOptions {
    /// Absolute deadline for the acquire operation.
    pub deadline: Option<Instant>,
    /// Per-call override of
    /// [`ManagerConfig::acquire_slow_threshold`](crate::ManagerConfig::acquire_slow_threshold).
    ///
    /// `knob`: `acquire_slow_threshold` | `default`: `None` (falls back to
    /// the manager-wide default) | `rationale`: a caller acquiring on a
    /// known-slow path (e.g. a cold-tier resource) can raise its own
    /// threshold to avoid false-positive WARNs the manager default would
    /// otherwise emit, without changing the default for every other caller.
    /// `Some(threshold)` here always wins over the manager default,
    /// regardless of which is larger.
    pub acquire_slow_threshold: Option<Duration>,
}

impl AcquireOptions {
    /// Sets a deadline for the acquire operation.
    #[must_use]
    pub fn with_deadline(mut self, deadline: Instant) -> Self {
        self.deadline = Some(deadline);
        self
    }

    /// Overrides the acquire-slow-log threshold for this call. See
    /// [`ManagerConfig::acquire_slow_threshold`](crate::ManagerConfig::acquire_slow_threshold)
    /// for the WARN contract.
    #[must_use]
    pub fn with_acquire_slow_threshold(mut self, threshold: Duration) -> Self {
        self.acquire_slow_threshold = Some(threshold);
        self
    }

    /// Returns the remaining time until the deadline, if set.
    pub fn remaining(&self) -> Option<Duration> {
        self.deadline
            .map(|d| d.saturating_duration_since(Instant::now()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_has_no_deadline() {
        let opts = AcquireOptions::default();
        assert!(opts.deadline.is_none());
    }

    #[test]
    fn with_deadline_sets_deadline() {
        let deadline = Instant::now() + Duration::from_secs(10);
        let opts = AcquireOptions::default().with_deadline(deadline);
        assert_eq!(opts.deadline, Some(deadline));
    }

    #[test]
    fn remaining_returns_some_when_deadline_set() {
        let opts = AcquireOptions::default().with_deadline(Instant::now() + Duration::from_mins(1));
        let remaining = opts.remaining().unwrap();
        assert!(remaining <= Duration::from_mins(1));
        assert!(remaining > Duration::from_secs(50));
    }

    #[test]
    fn remaining_returns_none_without_deadline() {
        let opts = AcquireOptions::default();
        assert!(opts.remaining().is_none());
    }

    #[test]
    fn default_has_no_acquire_slow_threshold() {
        let opts = AcquireOptions::default();
        assert!(opts.acquire_slow_threshold.is_none());
    }

    #[test]
    fn with_acquire_slow_threshold_sets_the_override() {
        let opts = AcquireOptions::default().with_acquire_slow_threshold(Duration::from_millis(50));
        assert_eq!(opts.acquire_slow_threshold, Some(Duration::from_millis(50)));
    }
}

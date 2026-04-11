//! Acquire-time options for resource leasing.
//!
//! [`AcquireOptions`] lets callers communicate intent and deadlines to the
//! resource subsystem, allowing topologies to make smarter scheduling and
//! prioritization decisions.

use std::{
    borrow::Cow,
    time::{Duration, Instant},
};

use smallvec::SmallVec;

/// The caller's intent when acquiring a resource lease.
///
/// Topologies may use this to select different pools, apply different
/// timeouts, or skip health checks for critical requests.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcquireIntent {
    /// Normal acquire — default path.
    Standard,
    /// Caller expects to hold the lease for a long time.
    LongRunning,
    /// Caller will stream data; includes expected duration hint.
    Streaming {
        /// Expected streaming duration.
        expected: Duration,
    },
    /// Prefetch — low priority, may be deferred.
    Prefetch,
    /// Critical — bypass queues if possible, never throttle.
    Critical,
}

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
#[derive(Debug, Clone)]
pub struct AcquireOptions {
    /// The caller's intent.
    pub intent: AcquireIntent,
    /// Absolute deadline for the acquire operation.
    pub deadline: Option<Instant>,
    /// Freeform key-value tags for routing and diagnostics.
    pub tags: SmallVec<[(Cow<'static, str>, Cow<'static, str>); 2]>,
}

impl Default for AcquireOptions {
    fn default() -> Self {
        Self {
            intent: AcquireIntent::Standard,
            deadline: None,
            tags: SmallVec::new(),
        }
    }
}

impl AcquireOptions {
    /// Sets a deadline for the acquire operation.
    pub fn with_deadline(mut self, deadline: Instant) -> Self {
        self.deadline = Some(deadline);
        self
    }

    /// Sets the acquire intent.
    pub fn with_intent(mut self, intent: AcquireIntent) -> Self {
        self.intent = intent;
        self
    }

    /// Adds a tag key-value pair.
    pub fn with_tag(
        mut self,
        key: impl Into<Cow<'static, str>>,
        value: impl Into<Cow<'static, str>>,
    ) -> Self {
        self.tags.push((key.into(), value.into()));
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
    fn default_is_standard_no_deadline() {
        let opts = AcquireOptions::default();
        assert_eq!(opts.intent, AcquireIntent::Standard);
        assert!(opts.deadline.is_none());
        assert!(opts.tags.is_empty());
    }

    #[test]
    fn with_deadline_sets_deadline() {
        let deadline = Instant::now() + Duration::from_secs(10);
        let opts = AcquireOptions::default().with_deadline(deadline);
        assert_eq!(opts.deadline, Some(deadline));
    }

    #[test]
    fn with_intent_sets_intent() {
        let opts = AcquireOptions::default().with_intent(AcquireIntent::Critical);
        assert_eq!(opts.intent, AcquireIntent::Critical);
    }

    #[test]
    fn with_tag_appends() {
        let opts = AcquireOptions::default()
            .with_tag("region", "us-east")
            .with_tag("priority", "high");
        assert_eq!(opts.tags.len(), 2);
        assert_eq!(opts.tags[0].0.as_ref(), "region");
        assert_eq!(opts.tags[0].1.as_ref(), "us-east");
    }

    #[test]
    fn remaining_returns_some_when_deadline_set() {
        let opts =
            AcquireOptions::default().with_deadline(Instant::now() + Duration::from_secs(60));
        let remaining = opts.remaining().unwrap();
        assert!(remaining <= Duration::from_secs(60));
        assert!(remaining > Duration::from_secs(50));
    }

    #[test]
    fn remaining_returns_none_without_deadline() {
        let opts = AcquireOptions::default();
        assert!(opts.remaining().is_none());
    }

    #[test]
    fn streaming_intent_carries_duration() {
        let intent = AcquireIntent::Streaming {
            expected: Duration::from_secs(300),
        };
        match intent {
            AcquireIntent::Streaming { expected } => {
                assert_eq!(expected, Duration::from_secs(300));
            }
            _ => panic!("wrong variant"),
        }
    }
}

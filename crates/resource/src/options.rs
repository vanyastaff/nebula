//! Acquire-time options for resource leasing.
//!
//! [`AcquireOptions`] lets callers communicate a deadline to the resource
//! subsystem. Older drafts also carried `intent` and `tags` fields reserved
//! for engine integration (#391); those were removed at register R-051
//! resolution since no consumer ever wired them. If #391 lands, the
//! relevant surface is added back via a new spec.

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
#[derive(Debug, Clone, Default)]
pub struct AcquireOptions {
    /// Absolute deadline for the acquire operation.
    pub deadline: Option<Instant>,
}

impl AcquireOptions {
    /// Sets a deadline for the acquire operation.
    pub fn with_deadline(mut self, deadline: Instant) -> Self {
        self.deadline = Some(deadline);
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
}

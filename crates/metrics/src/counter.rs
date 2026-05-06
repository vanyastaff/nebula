//! Monotonic counter primitive.
//!
//! Backed by [`std::sync::atomic::AtomicU64`] for lock-free hot-path increments.
//! The accompanying `last_updated_ms` timestamp drives
//! [`crate::registry::MetricsRegistry::retain_recent`] eviction.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use crate::registry::now_ms;

/// An incrementing counter.
#[derive(Debug, Clone)]
pub struct Counter {
    value: Arc<AtomicU64>,
    last_updated_ms: Arc<AtomicU64>,
}

impl Counter {
    /// Create a new counter starting at zero.
    #[must_use]
    pub fn new() -> Self {
        Self {
            value: Arc::new(AtomicU64::new(0)),
            last_updated_ms: Arc::new(AtomicU64::new(now_ms())),
        }
    }

    /// Increment by one.
    pub fn inc(&self) {
        self.value.fetch_add(1, Ordering::Relaxed);
        self.last_updated_ms.store(now_ms(), Ordering::Relaxed);
    }

    /// Increment by a given amount.
    ///
    /// `inc_by(0)` does not change the stored value or
    /// [`Self::last_updated_ms`] (see
    /// [`crate::registry::MetricsRegistry::retain_recent`]).
    pub fn inc_by(&self, n: u64) {
        if n == 0 {
            return;
        }
        self.value.fetch_add(n, Ordering::Relaxed);
        self.last_updated_ms.store(now_ms(), Ordering::Relaxed);
    }

    /// Current value.
    #[must_use]
    pub fn get(&self) -> u64 {
        self.value.load(Ordering::Relaxed)
    }

    /// Milliseconds since Unix epoch of the last write to this counter.
    #[must_use]
    pub fn last_updated_ms(&self) -> u64 {
        self.last_updated_ms.load(Ordering::Relaxed)
    }
}

impl Default for Counter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::Counter;

    #[test]
    fn counter_starts_at_zero() {
        let c = Counter::new();
        assert_eq!(c.get(), 0);
    }

    #[test]
    fn counter_increments() {
        let c = Counter::new();
        c.inc();
        c.inc_by(5);
        assert_eq!(c.get(), 6);
    }
}

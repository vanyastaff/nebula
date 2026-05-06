//! Signed gauge primitive.
//!
//! Backed by [`std::sync::atomic::AtomicI64`] for lock-free hot-path updates.
//! Idle gauges are not artificially refreshed: [`Gauge::set`] only bumps
//! `last_updated_ms` when the stored value actually changes, so retention
//! heuristics in [`crate::registry::MetricsRegistry::retain_recent`] do not
//! treat constant-value gauges as continuously active.

use std::sync::{
    Arc,
    atomic::{AtomicI64, AtomicU64, Ordering},
};

use crate::registry::now_ms;

/// A gauge that can go up and down.
#[derive(Debug, Clone)]
pub struct Gauge {
    value: Arc<AtomicI64>,
    last_updated_ms: Arc<AtomicU64>,
}

impl Gauge {
    /// Create a new gauge starting at zero.
    #[must_use]
    pub fn new() -> Self {
        Self {
            value: Arc::new(AtomicI64::new(0)),
            last_updated_ms: Arc::new(AtomicU64::new(now_ms())),
        }
    }

    /// Increment by one.
    pub fn inc(&self) {
        self.value.fetch_add(1, Ordering::Relaxed);
        self.last_updated_ms.store(now_ms(), Ordering::Relaxed);
    }

    /// Decrement by one.
    pub fn dec(&self) {
        self.value.fetch_sub(1, Ordering::Relaxed);
        self.last_updated_ms.store(now_ms(), Ordering::Relaxed);
    }

    /// Set to a specific value.
    ///
    /// If `v` equals the value already stored, [`Self::last_updated_ms`] is
    /// left unchanged so idle gauges are not kept artificially "fresh" for
    /// retention heuristics.
    pub fn set(&self, v: i64) {
        let previous = self.value.swap(v, Ordering::Relaxed);
        if previous != v {
            self.last_updated_ms.store(now_ms(), Ordering::Relaxed);
        }
    }

    /// Current value.
    #[must_use]
    pub fn get(&self) -> i64 {
        self.value.load(Ordering::Relaxed)
    }

    /// Milliseconds since Unix epoch of the last write to this gauge.
    #[must_use]
    pub fn last_updated_ms(&self) -> u64 {
        self.last_updated_ms.load(Ordering::Relaxed)
    }
}

impl Default for Gauge {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::Gauge;

    #[test]
    fn gauge_up_and_down() {
        let g = Gauge::new();
        g.inc();
        g.inc();
        g.dec();
        assert_eq!(g.get(), 1);
        g.set(42);
        assert_eq!(g.get(), 42);
    }
}

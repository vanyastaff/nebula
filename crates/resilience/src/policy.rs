//! Adaptive policy infrastructure — sources and signals.
//!
//! [`PolicySource`] provides the current configuration for a resilience pattern.
//! Static configs implement it automatically via the blanket impl; adaptive sources
//! compute the config at call-time based on a [`LoadSignal`].

use std::time::Duration;

// ═══════════════════════════════════════════════════════════════════════════════
// POLICY SOURCE
// ═══════════════════════════════════════════════════════════════════════════════

/// A source that provides the current configuration for a resilience pattern.
///
/// Static configs implement this automatically via the blanket impl below.
/// Adaptive sources compute the config at call-time based on runtime signals.
pub trait PolicySource<C: Clone>: Send + Sync {
    /// Returns the current configuration.
    fn current(&self) -> C;
}

/// Blanket impl: any `Clone + Send + Sync` value is a static policy source.
impl<C: Clone + Send + Sync> PolicySource<C> for C {
    fn current(&self) -> C {
        self.clone()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// LOAD SIGNAL
// ═══════════════════════════════════════════════════════════════════════════════

/// Runtime signal providing system load metrics for adaptive policies.
pub trait LoadSignal: Send + Sync {
    /// Overall load factor in 0.0..=1.0 (0 = idle, 1 = fully saturated).
    fn load_factor(&self) -> f64;
    /// Error rate over the last measurement window (0.0..=1.0).
    fn error_rate(&self) -> f64;
    /// Approximate p99 latency of recent operations.
    fn p99_latency(&self) -> Duration;
}

/// A constant load signal for testing adaptive policies.
pub struct ConstantLoad {
    /// Load factor: 0.0 = idle, 1.0 = saturated.
    pub factor: f64,
    /// Error rate: 0.0..=1.0.
    pub error_rate: f64,
    /// Approximate p99 latency.
    pub p99_latency: Duration,
}

impl ConstantLoad {
    /// A fully idle signal (0% load, 0% errors, 5ms latency).
    #[must_use]
    pub const fn idle() -> Self {
        Self {
            factor: 0.0,
            error_rate: 0.0,
            p99_latency: Duration::from_millis(5),
        }
    }

    /// A fully saturated signal (100% load, 50% errors, 2s latency).
    #[must_use]
    pub const fn saturated() -> Self {
        Self {
            factor: 1.0,
            error_rate: 0.5,
            p99_latency: Duration::from_secs(2),
        }
    }
}

impl LoadSignal for ConstantLoad {
    fn load_factor(&self) -> f64 {
        self.factor
    }

    fn error_rate(&self) -> f64 {
        self.error_rate
    }

    fn p99_latency(&self) -> Duration {
        self.p99_latency
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, PartialEq, Debug)]
    struct Config {
        value: u32,
    }

    #[test]
    fn static_config_is_policy_source() {
        let cfg = Config { value: 42 };
        assert_eq!(cfg.current(), Config { value: 42 });
    }

    #[test]
    fn static_config_returns_clone_each_time() {
        let cfg = Config { value: 7 };
        assert_eq!(cfg.current(), cfg.current());
    }

    #[test]
    fn idle_signal_returns_zero_load() {
        let s = ConstantLoad::idle();
        assert!((s.load_factor() - 0.0).abs() < f64::EPSILON);
        assert!((s.error_rate() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn saturated_signal_returns_full_load() {
        let s = ConstantLoad::saturated();
        assert!((s.load_factor() - 1.0).abs() < f64::EPSILON);
    }
}

//! Adaptive policy infrastructure — sources and signals.
//!
//! [`PolicySource`] provides the current configuration for a resilience pattern.
//! Static configs implement it automatically via the blanket impl; adaptive sources
//! compute the config at call-time based on a [`LoadSignal`].

use std::time::Duration;

use crate::ConfigError;

// ═══════════════════════════════════════════════════════════════════════════════
// POLICY SOURCE
// ═══════════════════════════════════════════════════════════════════════════════

/// A source that provides the current configuration for a resilience pattern.
///
/// Static configs implement this automatically via the blanket impl below.
/// Adaptive sources compute the config at call-time based on runtime signals.
///
/// # Examples
///
/// ```rust,no_run
/// use nebula_resilience::PolicySource;
///
/// // Static configs are policy sources for free.
/// let limit: u32 = 64;
/// assert_eq!(<u32 as PolicySource<u32>>::current(&limit), 64);
/// ```
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

    /// Return a validated snapshot of this signal.
    ///
    /// Adaptive policies should prefer this over calling the raw accessors
    /// independently, because downstream `LoadSignal` implementations can be
    /// buggy or fed by external telemetry. Invalid snapshots are rejected
    /// instead of silently creating nonsensical policy decisions.
    ///
    /// The default implementation calls [`load_factor`](Self::load_factor),
    /// [`error_rate`](Self::error_rate), and [`p99_latency`](Self::p99_latency)
    /// independently. Mutable signal implementations may therefore produce a
    /// mixed-window snapshot. Implementations that require a coherent/atomic
    /// capture across all fields must override this method and construct a
    /// [`LoadSnapshot`] from one internally consistent read.
    ///
    /// # Errors
    ///
    /// Returns `Err(ConfigError)` if `load_factor` or `error_rate` is not a
    /// finite value in `0.0..=1.0`.
    fn snapshot(&self) -> Result<LoadSnapshot, ConfigError> {
        LoadSnapshot::new(self.load_factor(), self.error_rate(), self.p99_latency())
    }
}

/// Validated load signal snapshot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LoadSnapshot {
    load_factor: f64,
    error_rate: f64,
    p99_latency: Duration,
}

impl LoadSnapshot {
    /// Create a validated load snapshot.
    ///
    /// # Errors
    ///
    /// Returns `Err(ConfigError)` when `load_factor` or `error_rate` is not a
    /// finite value in `0.0..=1.0`.
    pub fn new(
        load_factor: f64,
        error_rate: f64,
        p99_latency: Duration,
    ) -> Result<Self, ConfigError> {
        validate_unit_interval("load_factor", load_factor)?;
        validate_unit_interval("error_rate", error_rate)?;
        Ok(Self {
            load_factor,
            error_rate,
            p99_latency,
        })
    }

    /// Overall load factor in `0.0..=1.0`.
    #[must_use]
    pub const fn load_factor(self) -> f64 {
        self.load_factor
    }

    /// Error rate in `0.0..=1.0`.
    #[must_use]
    pub const fn error_rate(self) -> f64 {
        self.error_rate
    }

    /// Approximate p99 latency.
    #[must_use]
    pub const fn p99_latency(self) -> Duration {
        self.p99_latency
    }
}

/// A constant load signal for testing adaptive policies.
///
/// # Examples
///
/// ```rust,no_run
/// use nebula_resilience::{ConstantLoad, LoadSignal};
///
/// let idle = ConstantLoad::idle();
/// assert!(idle.load_factor() < f64::EPSILON);
///
/// let saturated = ConstantLoad::saturated();
/// assert!((saturated.load_factor() - 1.0).abs() < f64::EPSILON);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConstantLoad {
    /// Load factor: 0.0 = idle, 1.0 = saturated.
    factor: f64,
    /// Error rate: 0.0..=1.0.
    error_rate: f64,
    /// Approximate p99 latency.
    p99_latency: Duration,
}

impl ConstantLoad {
    /// Create a validated constant load signal.
    ///
    /// # Errors
    ///
    /// Returns `Err(ConfigError)` when `factor` or `error_rate` is not a finite
    /// value in `0.0..=1.0`.
    pub fn new(factor: f64, error_rate: f64, p99_latency: Duration) -> Result<Self, ConfigError> {
        let snapshot = LoadSnapshot::new(factor, error_rate, p99_latency)?;
        Ok(Self {
            factor: snapshot.load_factor(),
            error_rate: snapshot.error_rate(),
            p99_latency: snapshot.p99_latency(),
        })
    }

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

    /// Load factor: `0.0` = idle, `1.0` = saturated.
    #[must_use]
    pub const fn factor(self) -> f64 {
        self.factor
    }

    /// Error rate over the last measurement window.
    #[must_use]
    pub const fn measured_error_rate(self) -> f64 {
        self.error_rate
    }

    /// Approximate p99 latency.
    #[must_use]
    pub const fn measured_p99_latency(self) -> Duration {
        self.p99_latency
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

fn validate_unit_interval(field: &'static str, value: f64) -> Result<(), ConfigError> {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Ok(())
    } else {
        Err(ConfigError::new(field, "must be finite and in 0.0..=1.0"))
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

    #[test]
    fn constant_load_rejects_invalid_values() {
        assert!(ConstantLoad::new(f64::NAN, 0.0, Duration::ZERO).is_err());
        assert!(ConstantLoad::new(0.0, f64::INFINITY, Duration::ZERO).is_err());
        assert!(ConstantLoad::new(-0.1, 0.0, Duration::ZERO).is_err());
        assert!(ConstantLoad::new(0.0, 1.1, Duration::ZERO).is_err());
    }

    #[test]
    fn load_signal_snapshot_validates_custom_implementations() {
        struct BadSignal;

        impl LoadSignal for BadSignal {
            fn load_factor(&self) -> f64 {
                f64::NAN
            }

            fn error_rate(&self) -> f64 {
                0.0
            }

            fn p99_latency(&self) -> Duration {
                Duration::ZERO
            }
        }

        assert!(BadSignal.snapshot().is_err());
    }

    #[test]
    fn load_snapshot_exposes_validated_values() {
        let snapshot = LoadSnapshot::new(0.25, 0.5, Duration::from_millis(9)).unwrap();

        assert!((snapshot.load_factor() - 0.25).abs() < f64::EPSILON);
        assert!((snapshot.error_rate() - 0.5).abs() < f64::EPSILON);
        assert_eq!(snapshot.p99_latency(), Duration::from_millis(9));
    }

    #[test]
    fn constant_load_new_exposes_values_via_accessors() {
        let signal = ConstantLoad::new(0.25, 0.5, Duration::from_millis(9)).unwrap();

        assert!((signal.factor() - 0.25).abs() < f64::EPSILON);
        assert!((signal.measured_error_rate() - 0.5).abs() < f64::EPSILON);
        assert_eq!(signal.measured_p99_latency(), Duration::from_millis(9));
        assert!((signal.load_factor() - 0.25).abs() < f64::EPSILON);
        assert!((signal.error_rate() - 0.5).abs() < f64::EPSILON);
        assert_eq!(signal.p99_latency(), Duration::from_millis(9));
    }
}

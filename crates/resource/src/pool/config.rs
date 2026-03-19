//! Pool configuration types.
//!
//! Groups pool settings into four semantic namespaces:
//! [`PoolSizing`], [`PoolLifetime`], [`PoolAcquire`], and [`PoolResiliencePolicy`].

use std::time::Duration;

use nebula_resilience::CircuitBreakerConfig;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

// ---------------------------------------------------------------------------
// PoolStrategy
// ---------------------------------------------------------------------------

/// Strategy for selecting idle instances from the pool.
///
/// Controls whether the most-recently-used or least-recently-used
/// idle instance is returned on acquire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PoolStrategy {
    /// First-in, first-out: return the **oldest** idle instance.
    ///
    /// Distributes usage evenly across instances. This is the default.
    #[default]
    Fifo,
    /// Last-in, first-out: return the **most recently used** idle instance.
    ///
    /// Keeps a hot working set small, letting less-used instances idle-expire
    /// naturally. Useful when `min_size` is low relative to `max_size`.
    Lifo,
}

// ---------------------------------------------------------------------------
// PoolSharingMode
// ---------------------------------------------------------------------------

/// Sharing mode for a pool.
///
/// Controls whether pool instances are acquired exclusively (the default) or
/// served as cheap clones to multiple concurrent callers simultaneously.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PoolSharingMode {
    /// Classical pool — one exclusive [`Guard`](crate::guard::Guard) per
    /// instance at a time.
    ///
    /// Semaphore permits are consumed on acquire and returned on drop.
    /// This is the default and mirrors the behaviour of `bb8` / `deadpool`.
    #[default]
    Exclusive,
    /// Single shared instance — `acquire()` returns a **clone** of the single
    /// managed instance without consuming a semaphore permit.
    ///
    /// Requires `R::Instance: Clone`. Rate-limiting state, connection handles,
    /// and any mutable state must live inside the instance itself (e.g. behind
    /// an `Arc<Mutex<_>>`). The `Guard`'s on-drop is a no-op — there is nothing
    /// to return.
    ///
    /// Ideal for: Telegram `Bot`, `reqwest::Client`, `sqlx::PgPool`, structured
    /// loggers, and any resource whose instance is already `Arc`-wrapped and
    /// safe for concurrent access via `&self`.
    Shared,
}

// ---------------------------------------------------------------------------
// PoolBackpressurePolicy
// ---------------------------------------------------------------------------

/// Backpressure policy for acquire behavior when the pool is saturated.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PoolBackpressurePolicy {
    /// Immediately return [`Error::PoolExhausted`](crate::error::Error::PoolExhausted)
    /// when no permit is available.
    FailFast,
    /// Wait up to `timeout` for a permit, then return
    /// [`Error::PoolExhausted`](crate::error::Error::PoolExhausted).
    BoundedWait {
        /// Max wait time for permit acquisition.
        timeout: Duration,
    },
    /// Dynamically choose wait timeout based on current pressure.
    Adaptive(AdaptiveBackpressurePolicy),
}

// ---------------------------------------------------------------------------
// AdaptiveBackpressurePolicy
// ---------------------------------------------------------------------------

/// Configuration for adaptive acquire backpressure behavior.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdaptiveBackpressurePolicy {
    /// Utilization threshold (`active / max_size`) considered high pressure.
    pub high_pressure_utilization: f64,
    /// Waiter threshold considered high pressure.
    pub high_pressure_waiters: usize,
    /// Timeout used under low pressure.
    pub low_pressure_timeout: Duration,
    /// Timeout used under high pressure.
    pub high_pressure_timeout: Duration,
}

impl Default for AdaptiveBackpressurePolicy {
    fn default() -> Self {
        Self {
            high_pressure_utilization: 0.8,
            high_pressure_waiters: 8,
            low_pressure_timeout: Duration::from_secs(30),
            high_pressure_timeout: Duration::from_millis(100),
        }
    }
}

// ---------------------------------------------------------------------------
// RetryConfig
// ---------------------------------------------------------------------------

/// Retry policy for resource creation failures.
///
/// Used within [`PoolResiliencePolicy::create_retry`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    /// Initial delay before the first retry.
    pub initial_delay: Duration,
    /// Maximum delay cap (exponential back-off ceiling).
    pub max_delay: Duration,
    /// Multiplicative back-off factor applied after each failure.
    pub backoff_factor: f64,
    /// Maximum number of create attempts (1 = no retries).
    pub max_attempts: u32,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_millis(200),
            max_delay: Duration::from_secs(15),
            backoff_factor: 2.0,
            max_attempts: 3,
        }
    }
}

// ---------------------------------------------------------------------------
// PoolSizing
// ---------------------------------------------------------------------------

/// Pool sizing configuration: minimum and maximum instance counts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolSizing {
    /// Minimum number of instances to keep alive. Default: `1`.
    pub min_size: usize,
    /// Hard cap on concurrent checked-out + idle instances. Default: `10`.
    pub max_size: usize,
}

impl Default for PoolSizing {
    fn default() -> Self {
        Self {
            min_size: 1,
            max_size: 10,
        }
    }
}

impl PoolSizing {
    /// Single-instance pool: `min_size = 1`, `max_size = 1`.
    ///
    /// Suitable for singletons such as an exclusive file lock or a single
    /// TCP control channel.
    #[must_use]
    pub fn singleton() -> Self {
        Self {
            min_size: 1,
            max_size: 1,
        }
    }

    /// Fixed-range pool with explicit limits.
    #[must_use]
    pub fn fixed(min: usize, max: usize) -> Self {
        Self {
            min_size: min,
            max_size: max,
        }
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if self.max_size == 0 {
            return Err(Error::configuration("max_size must be greater than 0"));
        }
        if self.min_size > self.max_size {
            return Err(Error::configuration(format!(
                "min_size ({}) must not exceed max_size ({})",
                self.min_size, self.max_size
            )));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// PoolLifetime
// ---------------------------------------------------------------------------

/// Pool instance lifetime configuration: idle/age timeouts and maintenance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolLifetime {
    /// Maximum time an instance may sit idle before being evicted. Default: 10 min.
    pub idle_timeout: Duration,
    /// Maximum total age of an instance before forced eviction. Default: 30 min.
    pub max_lifetime: Duration,
    /// Interval at which idle instances are validated. Default: 30 s.
    pub validation_interval: Duration,
    /// If set, spawns a background task that calls `maintain()` at this interval.
    /// `None` disables automatic maintenance. Default: `Some(60 s)`.
    pub maintenance_interval: Option<Duration>,
}

impl Default for PoolLifetime {
    fn default() -> Self {
        Self {
            idle_timeout: Duration::from_secs(600),
            max_lifetime: Duration::from_secs(3600),
            validation_interval: Duration::from_secs(30),
            maintenance_interval: Some(Duration::from_secs(60)),
        }
    }
}

impl PoolLifetime {
    /// Never evict idle or aged-out instances.
    ///
    /// Suitable for truly persistent connections: SSH sessions, Telegram Bot,
    /// WebSocket channels, or any resource where reconnection is expensive.
    #[must_use]
    pub fn persistent() -> Self {
        Self {
            idle_timeout: Duration::MAX,
            max_lifetime: Duration::MAX,
            ..Self::default()
        }
    }
}

// ---------------------------------------------------------------------------
// PoolAcquire
// ---------------------------------------------------------------------------

/// Pool acquire configuration: timeout, strategy, backpressure, and sharing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolAcquire {
    /// Maximum time to wait for a free permit. Default: 30 s.
    pub timeout: Duration,
    /// Optional backpressure policy profile.
    ///
    /// When `None`, bounded-wait behaviour is preserved using `timeout`
    /// for backward compatibility.
    pub backpressure: Option<PoolBackpressurePolicy>,
    /// Strategy for selecting idle instances on acquire. Default: [`PoolStrategy::Fifo`].
    pub strategy: PoolStrategy,
    /// Instance sharing mode. Default: [`PoolSharingMode::Exclusive`].
    pub sharing_mode: PoolSharingMode,
    /// Pre-warm the pool on construction.
    ///
    /// When `true`, `Pool::new` will create up to `min_size` instances
    /// eagerly. Creation errors during warm-up are non-fatal. Default: `false`.
    pub warm_up: bool,
}

impl Default for PoolAcquire {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            backpressure: None,
            strategy: PoolStrategy::default(),
            sharing_mode: PoolSharingMode::default(),
            warm_up: false,
        }
    }
}

impl PoolAcquire {
    /// Create an acquire config for a shared (clone-based) pool.
    #[must_use]
    pub fn shared() -> Self {
        Self {
            sharing_mode: PoolSharingMode::Shared,
            ..Self::default()
        }
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if self.timeout.is_zero() {
            return Err(Error::configuration(
                "acquire_timeout must be greater than zero",
            ));
        }
        if let Some(policy) = &self.backpressure {
            match policy {
                PoolBackpressurePolicy::FailFast => {}
                PoolBackpressurePolicy::BoundedWait { timeout } => {
                    if timeout.is_zero() {
                        return Err(Error::configuration(
                            "backpressure bounded wait timeout must be greater than zero",
                        ));
                    }
                }
                PoolBackpressurePolicy::Adaptive(adaptive) => {
                    if !(0.0..=1.0).contains(&adaptive.high_pressure_utilization)
                        || adaptive.high_pressure_utilization == 0.0
                    {
                        return Err(Error::configuration(
                            "adaptive high_pressure_utilization must be in (0, 1]",
                        ));
                    }
                    if adaptive.high_pressure_waiters == 0 {
                        return Err(Error::configuration(
                            "adaptive high_pressure_waiters must be greater than zero",
                        ));
                    }
                    if adaptive.low_pressure_timeout.is_zero()
                        || adaptive.high_pressure_timeout.is_zero()
                    {
                        return Err(Error::configuration(
                            "adaptive timeouts must be greater than zero",
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// PoolResiliencePolicy
// ---------------------------------------------------------------------------

/// Pool resilience configuration: circuit breakers, operation timeouts, retry.
///
/// The runtime counterpart — [`PoolResilienceState`](crate::pool::inner::PoolResilienceState) —
/// is constructed from this config during pool construction.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PoolResiliencePolicy {
    /// Circuit breaker applied to `Resource::create()`.
    pub create_breaker: Option<CircuitBreakerConfig>,
    /// Circuit breaker applied to `Resource::recycle()`.
    pub recycle_breaker: Option<CircuitBreakerConfig>,
    /// Optional timeout for a single `Resource::create()` call.
    pub create_timeout: Option<Duration>,
    /// Optional timeout for a single `Resource::recycle()` call.
    pub recycle_timeout: Option<Duration>,
    /// Optional retry policy applied to `Resource::create()` failures.
    pub create_retry: Option<RetryConfig>,
}

impl PoolResiliencePolicy {
    /// Standard resilience: circuit breakers on both create and recycle.
    #[must_use]
    pub fn standard() -> Self {
        Self {
            create_breaker: Some(CircuitBreakerConfig::default()),
            recycle_breaker: Some(CircuitBreakerConfig::default()),
            ..Default::default()
        }
    }

    /// Resilience for persistent connections: create circuit breaker + retry.
    ///
    /// No recycle circuit breaker — persistent connections don't recycle.
    #[must_use]
    pub fn persistent_connection() -> Self {
        Self {
            create_breaker: Some(CircuitBreakerConfig::default()),
            create_retry: Some(RetryConfig::default()),
            ..Default::default()
        }
    }

    /// Resilience for singleton resources: create circuit breaker + aggressive retry.
    #[must_use]
    pub fn singleton() -> Self {
        Self {
            create_breaker: Some(CircuitBreakerConfig::default()),
            create_retry: Some(RetryConfig {
                max_attempts: 5,
                ..RetryConfig::default()
            }),
            ..Default::default()
        }
    }

    /// Returns `true` when all resilience fields are default (nothing configured).
    ///
    /// Used to skip allocation of [`PoolResilienceState`](crate::pool::inner::PoolResilienceState)
    /// for pools that don't need it.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.create_breaker.is_none()
            && self.recycle_breaker.is_none()
            && self.create_timeout.is_none()
            && self.recycle_timeout.is_none()
            && self.create_retry.is_none()
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if self.create_timeout.is_some_and(|t| t.is_zero()) {
            return Err(Error::configuration(
                "create_timeout must be greater than zero when set",
            ));
        }
        if self.recycle_timeout.is_some_and(|t| t.is_zero()) {
            return Err(Error::configuration(
                "recycle_timeout must be greater than zero when set",
            ));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// PoolConfig
// ---------------------------------------------------------------------------

/// Configuration for resource pooling.
///
/// Groups settings into four semantic sub-structs:
///
/// | Field | Sub-struct | Controls |
/// |---|---|---|
/// | `sizing` | [`PoolSizing`] | `min_size`, `max_size` |
/// | `lifetime` | [`PoolLifetime`] | idle/age timeouts, maintenance interval |
/// | `acquire` | [`PoolAcquire`] | acquire timeout, strategy, backpressure, sharing mode |
/// | `resilience` | [`PoolResiliencePolicy`] | circuit breakers, operation timeouts, retry |
///
/// # Example
///
/// ```rust,ignore
/// use nebula_resource::{PoolConfig, PoolSizing, PoolLifetime, PoolAcquire};
/// use std::time::Duration;
///
/// let config = PoolConfig {
///     sizing: PoolSizing { min_size: 2, max_size: 20 },
///     lifetime: PoolLifetime { maintenance_interval: None, ..Default::default() },
///     acquire: PoolAcquire { timeout: Duration::from_secs(5), ..Default::default() },
///     ..Default::default()
/// };
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PoolConfig {
    /// Minimum and maximum instance counts.
    pub sizing: PoolSizing,
    /// Idle/age timeouts and maintenance scheduling.
    pub lifetime: PoolLifetime,
    /// Acquire timeout, strategy, backpressure, and sharing mode.
    pub acquire: PoolAcquire,
    /// Circuit breakers, operation timeouts, and retry policy.
    pub resilience: PoolResiliencePolicy,
}

impl PoolConfig {
    /// Enable both create/recycle circuit breakers using standard defaults.
    #[must_use]
    pub fn with_standard_breakers(mut self) -> Self {
        self.resilience = PoolResiliencePolicy::standard();
        self
    }

    /// Validate pool configuration, returning an error if invalid.
    pub fn validate(&self) -> Result<()> {
        self.sizing.validate()?;
        self.acquire.validate()?;
        self.resilience.validate()?;
        Ok(())
    }

    /// Returns the effective acquire backpressure policy for this config.
    #[must_use]
    pub fn effective_backpressure_policy(&self) -> PoolBackpressurePolicy {
        self.acquire
            .backpressure
            .clone()
            .unwrap_or(PoolBackpressurePolicy::BoundedWait {
                timeout: self.acquire.timeout,
            })
    }
}

//! Hedge request pattern — fires duplicate requests after a delay, returns the first success.
//!
//! Losing tasks are aborted via `JoinSet::abort_all()` when the first success arrives.
//!
//! # Cancel safety
//!
//! `HedgeExecutor::call` is **not cancel-safe**. If the returned future is dropped,
//! already-spawned `tokio::spawn` tasks continue running in the background until they
//! complete or are individually aborted. This is intentional: the hedge pattern assumes
//! speculative work is cheap to abandon at the infrastructure level.

use std::collections::VecDeque;
use std::fmt;
use std::future::Future;

use smallvec::SmallVec;

use std::sync::Arc;
use std::time::Duration;
use parking_lot::RwLock;
use tokio::task::JoinSet;
use tokio::time::{Instant, sleep};

use crate::{
    CallError,
    sink::{MetricsSink, NoopSink, ResilienceEvent},
};

// ── Config ────────────────────────────────────────────────────────────────────

/// Configuration for the hedge pattern.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HedgeConfig {
    /// Delay before sending each hedge request.
    pub hedge_delay: Duration,
    /// Maximum number of hedge (duplicate) requests beyond the first.
    pub max_hedges: usize,
    /// Whether to use exponential backoff for successive hedge delays.
    pub exponential_backoff: bool,
    /// Multiplier applied when `exponential_backoff` is true.
    pub backoff_multiplier: f64,
}

impl Default for HedgeConfig {
    fn default() -> Self {
        Self {
            hedge_delay: Duration::from_millis(50),
            max_hedges: 2,
            exponential_backoff: true,
            backoff_multiplier: 2.0,
        }
    }
}

impl HedgeConfig {
    /// Validate configuration.
    ///
    /// # Errors
    ///
    /// Returns `Err(ConfigError)` if `hedge_delay` is zero, `max_hedges` is 0,
    /// or `backoff_multiplier` is not finite or less than 1.0 when exponential backoff is enabled.
    pub fn validate(&self) -> Result<(), crate::ConfigError> {
        if self.hedge_delay.is_zero() {
            return Err(crate::ConfigError::new("hedge_delay", "must be > 0"));
        }
        if self.max_hedges == 0 {
            return Err(crate::ConfigError::new("max_hedges", "must be >= 1"));
        }
        if self.exponential_backoff
            && (!self.backoff_multiplier.is_finite() || self.backoff_multiplier < 1.0)
        {
            return Err(crate::ConfigError::new(
                "backoff_multiplier",
                "must be >= 1.0 when exponential_backoff is enabled",
            ));
        }
        Ok(())
    }
}

// ── HedgeExecutor ─────────────────────────────────────────────────────────────

/// Executes an operation with hedging: fires duplicate requests after a delay and returns
/// the first successful result, aborting all other in-flight requests.
pub struct HedgeExecutor {
    config: HedgeConfig,
    sink: Arc<dyn MetricsSink>,
}

impl fmt::Debug for HedgeExecutor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HedgeExecutor")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl HedgeExecutor {
    /// Create a new hedge executor.
    ///
    /// # Errors
    ///
    /// Returns `Err(ConfigError)` if the configuration is invalid.
    pub fn new(config: HedgeConfig) -> Result<Self, crate::ConfigError> {
        config.validate()?;
        Ok(Self {
            config,
            sink: Arc::new(NoopSink),
        })
    }

    /// Inject a metrics sink.
    #[must_use]
    pub fn with_sink(mut self, sink: impl MetricsSink + 'static) -> Self {
        self.sink = Arc::new(sink);
        self
    }

    /// Call `operation` with hedging.
    ///
    /// - Returns the first `Ok(T)` result, aborting remaining requests.
    /// - Returns the last `Err` if all attempts fail.
    ///
    /// # Errors
    ///
    /// Returns `Err(CallError::Operation)` if all attempts (including hedges) fail,
    /// or `Err(CallError::Cancelled)` if all tasks were cancelled or panicked.
    ///
    /// # Cancel safety
    ///
    /// Not cancel-safe — see module-level documentation.
    pub async fn call<T, E, F, Fut>(&self, operation: F) -> Result<T, CallError<E>>
    where
        T: Send + 'static,
        E: Send + 'static,
        F: Fn() -> Fut + Send + Sync,
        Fut: Future<Output = Result<T, E>> + Send + 'static,
    {
        let mut set: JoinSet<Result<T, E>> = JoinSet::new();
        set.spawn(operation());

        let mut hedge_delay = self.config.hedge_delay;
        let mut hedges_sent = 0usize;
        let mut delay = Box::pin(sleep(hedge_delay));
        let mut last_err: Option<E> = None;

        loop {
            tokio::select! {
                biased;

                // First completed task wins; join_next() returns the earliest ready.
                // Guard prevents polling an empty JoinSet on every iteration while
                // waiting for the delay to fire the next hedge.
                Some(join_result) = set.join_next(), if !set.is_empty() => {
                    match join_result {
                        Ok(Ok(v)) => {
                            set.abort_all();
                            return Ok(v);
                        }
                        Ok(Err(e)) => last_err = Some(e),
                        Err(_) => {} // task panicked or was aborted
                    }
                    if set.is_empty() && hedges_sent >= self.config.max_hedges {
                        return Err(
                            last_err.map_or(CallError::Cancelled { reason: None }, CallError::Operation)
                        );
                    }
                }

                // Fire the next hedge after the configured delay.
                () = &mut delay, if hedges_sent < self.config.max_hedges => {
                    // Reason: max_hedges is a small config value, never exceeds u32.
                    #[allow(clippy::cast_possible_truncation)]
                    let hedge_num = (hedges_sent + 1) as u32;
                    self.sink.record(ResilienceEvent::HedgeFired { hedge_number: hedge_num });
                    set.spawn(operation());
                    hedges_sent += 1;

                    if self.config.exponential_backoff {
                        let next_secs = hedge_delay.as_secs_f64()
                            * self.config.backoff_multiplier;
                        // Cap at 1 hour to avoid Duration::from_secs_f64 panic on
                        // overflow to infinity with large max_hedges.
                        hedge_delay = Duration::from_secs_f64(next_secs.min(3600.0));
                    }
                    delay.as_mut().reset(Instant::now() + hedge_delay);
                }
            }
        }
    }
}

// ── AdaptiveHedgeExecutor ─────────────────────────────────────────────────────

/// Hedge executor that adjusts delay based on observed latency percentiles.
pub struct AdaptiveHedgeExecutor {
    base_config: HedgeConfig,
    // RwLock: percentile() only needs a shared ref; write lock taken only for record().
    // parking_lot::RwLock is used because neither record() nor percentile() cross .await points.
    latency_tracker: Arc<RwLock<LatencyTracker>>,
    target_percentile: f64,
    sink: Arc<dyn MetricsSink>,
}

impl fmt::Debug for AdaptiveHedgeExecutor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AdaptiveHedgeExecutor")
            .field("base_config", &self.base_config)
            .field("target_percentile", &self.target_percentile)
            .finish_non_exhaustive()
    }
}

impl AdaptiveHedgeExecutor {
    /// Create a new adaptive hedge executor.
    ///
    /// # Errors
    ///
    /// Returns `Err(ConfigError)` if the configuration is invalid.
    pub fn new(config: HedgeConfig) -> Result<Self, crate::ConfigError> {
        config.validate()?;
        Ok(Self {
            base_config: config,
            latency_tracker: Arc::new(RwLock::new(LatencyTracker::new(1000))),
            target_percentile: 0.95,
            sink: Arc::new(NoopSink),
        })
    }

    /// Set the target latency percentile for hedge delay calculation.
    ///
    /// # Errors
    ///
    /// Returns `Err(ConfigError)` if `percentile` is not finite or outside 0.0..=1.0.
    pub fn with_target_percentile(mut self, percentile: f64) -> Result<Self, crate::ConfigError> {
        if !percentile.is_finite() || !(0.0..=1.0).contains(&percentile) {
            return Err(crate::ConfigError::new(
                "target_percentile",
                "must be 0.0..=1.0",
            ));
        }
        self.target_percentile = percentile;
        Ok(self)
    }

    /// Inject a metrics sink.
    #[must_use]
    pub fn with_sink(mut self, sink: impl MetricsSink + 'static) -> Self {
        self.sink = Arc::new(sink);
        self
    }

    /// Set the maximum number of latency samples retained for percentile calculation.
    ///
    /// Larger values improve percentile accuracy but consume more memory.
    /// Default: 1000.
    ///
    /// # Errors
    ///
    /// Returns `Err(ConfigError)` if `max_samples` is 0.
    pub fn with_max_samples(mut self, max_samples: usize) -> Result<Self, crate::ConfigError> {
        if max_samples == 0 {
            return Err(crate::ConfigError::new("max_samples", "must be >= 1"));
        }
        self.latency_tracker = Arc::new(RwLock::new(LatencyTracker::new(max_samples)));
        Ok(self)
    }

    /// Call with adaptive hedging.
    ///
    /// # Errors
    ///
    /// Returns `Err(CallError::Operation)` if all attempts fail,
    /// or `Err(CallError::Cancelled)` if all tasks were cancelled.
    ///
    /// # Cancel safety
    ///
    /// Not cancel-safe — see module-level documentation.
    pub async fn call<T, E, F, Fut>(&self, operation: F) -> Result<T, CallError<E>>
    where
        T: Send + 'static,
        E: Send + 'static,
        F: Fn() -> Fut + Send + Sync,
        Fut: Future<Output = Result<T, E>> + Send + 'static,
    {
        let start = std::time::Instant::now();

        let hedge_delay = {
            let tracker = self.latency_tracker.read();
            let delay = tracker
                .percentile(self.target_percentile)
                .unwrap_or(self.base_config.hedge_delay);
            drop(tracker);
            // Enforce minimum delay to prevent burst-firing all hedges at once
            // when recorded latencies are near-zero (e.g., cached responses).
            delay.max(Duration::from_micros(1))
        };

        let config = HedgeConfig {
            hedge_delay,
            ..self.base_config.clone()
        };
        let executor = HedgeExecutor {
            config,
            sink: Arc::clone(&self.sink),
        };
        // Config was pre-validated at AdaptiveHedgeExecutor construction;
        // only hedge_delay differs (computed from percentile), which is always > 0.
        let result = executor.call(operation).await;

        self.latency_tracker.write().record(start.elapsed());
        result
    }
}

// ── LatencyTracker ────────────────────────────────────────────────────────────

/// Ring-buffer latency tracker with O(log n) insert and O(k) percentile computation,
/// where k is the number of distinct latency values (at most `max_samples`).
///
/// Uses a sorted `Vec<(u64, u32)>` histogram (nanoseconds → count) instead of
/// `BTreeMap` to avoid per-record heap allocations in steady-state.
///
/// Made `pub` so it can be benchmarked directly from `benches/latency_tracker.rs`.
#[doc(hidden)]
pub struct LatencyTracker {
    /// Ring buffer storing nanosecond values of recent samples.
    ring: VecDeque<u64>,
    /// Sorted histogram: `(nanos, count)` pairs, ordered by `nanos` ascending.
    ///
    /// Up to 64 distinct latency buckets are stored inline (no heap allocation).
    /// Real workloads cluster tightly, so inline capacity covers 99%+ of cases.
    histogram: SmallVec<[(u64, u32); 64]>,
    max_samples: usize,
}

impl LatencyTracker {
    #[must_use]
    pub fn new(max_samples: usize) -> Self {
        Self {
            ring: VecDeque::with_capacity(max_samples),
            histogram: SmallVec::new(),
            max_samples,
        }
    }

    // Reason: u128 nanosecond values from Duration::as_nanos() are truncated to u64
    // (max ~584 years), and f64 precision loss is acceptable for percentile calculations.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    pub fn record(&mut self, latency: Duration) {
        let nanos = latency.as_nanos() as u64;

        if self.ring.len() == self.max_samples
            && let Some(oldest) = self.ring.pop_front()
            && let Ok(idx) = self.histogram.binary_search_by_key(&oldest, |e| e.0)
        {
            if self.histogram[idx].1 <= 1 {
                self.histogram.remove(idx);
            } else {
                self.histogram[idx].1 -= 1;
            }
        }

        match self.histogram.binary_search_by_key(&nanos, |e| e.0) {
            Ok(idx) => self.histogram[idx].1 += 1,
            Err(idx) => self.histogram.insert(idx, (nanos, 1)),
        }
        self.ring.push_back(nanos);
    }

    // Reason: f64 precision loss and sign loss are acceptable for percentile index calculation.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    #[must_use]
    pub fn percentile(&self, p: f64) -> Option<Duration> {
        if self.ring.is_empty() || !p.is_finite() {
            return None;
        }
        let target = ((self.ring.len() as f64 - 1.0) * p.clamp(0.0, 1.0)) as usize;
        let mut accumulated = 0usize;
        for &(nanos, cnt) in &self.histogram {
            accumulated += cnt as usize;
            if accumulated > target {
                return Some(Duration::from_nanos(nanos));
            }
        }
        // Unreachable in practice: total accumulated == ring.len() > target for any valid p.
        // Present as a safe fallback for floating-point edge cases.
        self.histogram
            .last()
            .map(|&(nanos, _)| Duration::from_nanos(nanos))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RecordingSink, ResilienceEventKind};

    #[tokio::test]
    async fn returns_first_success() {
        let counter = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let c = counter.clone();

        let executor = HedgeExecutor::new(HedgeConfig {
            hedge_delay: Duration::from_millis(50),
            max_hedges: 2,
            ..Default::default()
        })
        .unwrap();

        let result = executor
            .call(|| {
                let c = c.clone();
                Box::pin(async move {
                    c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Ok::<_, &str>("ok")
                })
            })
            .await;

        assert_eq!(result.unwrap(), "ok");
    }

    #[tokio::test]
    async fn emits_hedge_fired_event() {
        let sink = RecordingSink::new();
        let executor = HedgeExecutor::new(HedgeConfig {
            hedge_delay: Duration::from_millis(10),
            max_hedges: 1,
            ..Default::default()
        })
        .unwrap()
        .with_sink(sink.clone());

        let _ = executor
            .call(|| {
                Box::pin(async {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    Ok::<_, &str>("late")
                })
            })
            .await;

        assert!(sink.count(ResilienceEventKind::HedgeFired) > 0);
    }

    // ── C2: HedgeConfig validation ───────────────────────────────────────

    #[test]
    fn rejects_zero_hedge_delay() {
        let config = HedgeConfig {
            hedge_delay: Duration::ZERO,
            ..Default::default()
        };
        assert_eq!(config.validate().unwrap_err().field, "hedge_delay");
    }

    #[test]
    fn rejects_zero_max_hedges() {
        let config = HedgeConfig {
            max_hedges: 0,
            ..Default::default()
        };
        assert_eq!(config.validate().unwrap_err().field, "max_hedges");
    }

    #[test]
    fn rejects_invalid_backoff_multiplier() {
        let config = HedgeConfig {
            exponential_backoff: true,
            backoff_multiplier: 0.5,
            ..Default::default()
        };
        assert_eq!(config.validate().unwrap_err().field, "backoff_multiplier");
    }

    #[test]
    fn accepts_valid_config() {
        assert!(HedgeExecutor::new(HedgeConfig::default()).is_ok());
    }

    #[test]
    fn with_max_samples_rejects_zero() {
        let executor = AdaptiveHedgeExecutor::new(HedgeConfig::default()).unwrap();
        assert_eq!(
            executor.with_max_samples(0).unwrap_err().field,
            "max_samples"
        );
    }

    #[test]
    fn with_max_samples_accepts_valid() {
        let executor = AdaptiveHedgeExecutor::new(HedgeConfig::default()).unwrap();
        assert!(executor.with_max_samples(100).is_ok());
    }
}

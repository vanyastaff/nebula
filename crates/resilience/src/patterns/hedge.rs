//! Hedge request pattern for reducing tail latency

use futures::StreamExt;
use futures::stream::FuturesUnordered;
use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::{Instant, sleep, timeout};

use crate::core::config::{ConfigError, ConfigResult};
use crate::{ResilienceError, ResilienceResult};

/// Hedge strategy configuration
#[derive(Debug, Clone)]
pub struct HedgeConfig {
    /// Delay before sending hedge request
    pub hedge_delay: Duration,
    /// Maximum number of hedge requests
    pub max_hedges: usize,
    /// Whether to use exponential backoff for hedge delays
    pub exponential_backoff: bool,
    /// Backoff multiplier
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

/// Hedge executor
pub struct HedgeExecutor {
    config: HedgeConfig,
}

impl HedgeExecutor {
    /// Create new hedge executor
    #[must_use]
    pub const fn new(config: HedgeConfig) -> Self {
        Self { config }
    }

    /// Execute with hedging
    pub async fn execute<T, F, Fut>(&self, operation: F) -> ResilienceResult<T>
    where
        F: Fn() -> Fut + Send + Sync,
        Fut: Future<Output = ResilienceResult<T>> + Send,
        T: Send,
    {
        let mut in_flight: FuturesUnordered<
            Pin<Box<dyn Future<Output = ResilienceResult<T>> + Send>>,
        > = FuturesUnordered::new();
        in_flight.push(Box::pin(operation()));

        let mut hedge_delay = self.config.hedge_delay;
        let timeout_duration = hedge_delay;
        let mut hedges_sent = 0usize;
        let mut delay = Box::pin(sleep(hedge_delay));

        loop {
            if hedges_sent >= self.config.max_hedges {
                return in_flight
                    .next()
                    .await
                    .unwrap_or(Err(ResilienceError::Timeout {
                        duration: timeout_duration,
                        context: Some("Hedge timeout".to_string()),
                    }));
            }

            tokio::select! {
                maybe_result = in_flight.next() => {
                    if let Some(result) = maybe_result {
                        return result;
                    }

                    return Err(ResilienceError::Timeout {
                        duration: timeout_duration,
                        context: Some("Hedge timeout".to_string()),
                    });
                }
                () = &mut delay => {
                    in_flight.push(Box::pin(operation()));
                    hedges_sent += 1;

                    if self.config.exponential_backoff {
                        hedge_delay = Duration::from_secs_f64(
                            hedge_delay.as_secs_f64() * self.config.backoff_multiplier,
                        );
                    }

                    delay.as_mut().reset(Instant::now() + hedge_delay);
                }
            }
        }
    }
}

/// Adaptive hedge executor that adjusts delay based on latency percentiles
pub struct AdaptiveHedgeExecutor {
    base_config: HedgeConfig,
    latency_tracker: Arc<tokio::sync::Mutex<LatencyTracker>>,
    target_percentile: f64,
}

impl AdaptiveHedgeExecutor {
    /// Create new adaptive hedge executor
    #[must_use]
    pub fn new(config: HedgeConfig) -> Self {
        Self {
            base_config: config,
            latency_tracker: Arc::new(tokio::sync::Mutex::new(LatencyTracker::new(1000))),
            target_percentile: 0.95, // Target P95
        }
    }

    /// Set target percentile for hedge delay calculation
    #[must_use = "builder methods must be chained or built"]
    pub fn with_target_percentile(mut self, percentile: f64) -> ConfigResult<Self> {
        if !percentile.is_finite() {
            return Err(ConfigError::validation(
                "target percentile must be a finite number",
            ));
        }

        if !(0.0..=1.0).contains(&percentile) {
            return Err(ConfigError::validation(
                "target percentile must be in range [0.0, 1.0]",
            ));
        }

        self.target_percentile = percentile;
        Ok(self)
    }

    /// Execute with adaptive hedging
    pub async fn execute<T, F, Fut>(&self, operation: F) -> ResilienceResult<T>
    where
        F: Fn() -> Fut + Send + Sync,
        Fut: Future<Output = ResilienceResult<T>> + Send,
        T: Send,
    {
        let start = std::time::Instant::now();

        let hedge_delay = {
            let tracker = self.latency_tracker.lock().await;
            tracker
                .percentile(self.target_percentile)
                .unwrap_or(self.base_config.hedge_delay)
        };

        let config = HedgeConfig {
            hedge_delay,
            ..self.base_config.clone()
        };

        let executor = HedgeExecutor::new(config);
        let result = executor.execute(operation).await;

        {
            let mut tracker = self.latency_tracker.lock().await;
            tracker.record(start.elapsed());
        }

        result
    }
}

/// Latency tracker for adaptive hedging.
///
/// Uses a ring buffer (`VecDeque`) paired with a sorted frequency map (`BTreeMap`) so
/// that `percentile()` runs in O(n) with no allocation instead of O(n log n) + O(n) clone.
struct LatencyTracker {
    /// Ordered ring of recorded durations (oldest at front).
    ring: VecDeque<Duration>,
    max_samples: usize,
    /// nanos → number of samples with that value. Kept in sync with `ring`.
    sorted: BTreeMap<u64, usize>,
}

impl LatencyTracker {
    fn new(max_samples: usize) -> Self {
        Self {
            ring: VecDeque::with_capacity(max_samples),
            max_samples,
            sorted: BTreeMap::new(),
        }
    }

    fn record(&mut self, latency: Duration) {
        if self.ring.len() == self.max_samples {
            if let Some(oldest) = self.ring.pop_front() {
                let key = oldest.as_nanos() as u64;
                if let Some(c) = self.sorted.get_mut(&key) {
                    if *c <= 1 {
                        self.sorted.remove(&key);
                    } else {
                        *c -= 1;
                    }
                }
            }
        }

        *self.sorted.entry(latency.as_nanos() as u64).or_insert(0) += 1;
        self.ring.push_back(latency);
    }

    fn percentile(&self, p: f64) -> Option<Duration> {
        if self.ring.is_empty() || !p.is_finite() {
            return None;
        }

        let percentile = p.clamp(0.0, 1.0);
        let target = ((self.ring.len() as f64 - 1.0) * percentile) as usize;
        let mut accumulated = 0usize;

        for (&nanos, &cnt) in &self.sorted {
            accumulated += cnt;
            if accumulated > target {
                return Some(Duration::from_nanos(nanos));
            }
        }

        self.sorted
            .keys()
            .next_back()
            .map(|&nanos| Duration::from_nanos(nanos))
    }
}

/// Bimodal hedge executor - uses different strategies for fast vs slow operations
pub struct BimodalHedgeExecutor {
    fast_threshold: Duration,
    fast_config: HedgeConfig,
    slow_config: HedgeConfig,
}

impl BimodalHedgeExecutor {
    /// Create new bimodal hedge executor
    #[must_use]
    pub const fn new(
        fast_threshold: Duration,
        fast_config: HedgeConfig,
        slow_config: HedgeConfig,
    ) -> Self {
        Self {
            fast_threshold,
            fast_config,
            slow_config,
        }
    }

    /// Execute with bimodal hedging
    pub async fn execute<T, F, Fut>(&self, operation: F) -> ResilienceResult<T>
    where
        F: Fn() -> Fut + Send + Sync,
        Fut: Future<Output = ResilienceResult<T>> + Send,
        T: Send,
    {
        let fast_executor = HedgeExecutor::new(self.fast_config.clone());
        match timeout(self.fast_threshold, fast_executor.execute(&operation)).await {
            Ok(result) => result,
            Err(_elapsed) => {
                let executor = HedgeExecutor::new(self.slow_config.clone());
                executor.execute(operation).await
            }
        }
    }
}

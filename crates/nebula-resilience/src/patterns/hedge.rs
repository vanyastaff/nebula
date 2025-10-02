//! Hedge request pattern for reducing tail latency

use futures::FutureExt;
use futures::future::{Either, select};
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

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
    pub fn new(config: HedgeConfig) -> Self {
        Self { config }
    }

    /// Execute with hedging
    pub async fn execute<T, F, Fut>(&self, operation: F) -> ResilienceResult<T>
    where
        F: Fn() -> Fut + Send + Sync,
        Fut: Future<Output = ResilienceResult<T>> + Send,
        T: Send,
    {
        // Start primary request
        let primary = operation();
        tokio::pin!(primary);

        let mut hedge_delay = self.config.hedge_delay;
        let mut hedges_sent = 0;

        // Store hedge futures
        let mut hedge_futures = Vec::new();

        loop {
            // Create delay future
            let delay = sleep(hedge_delay);
            tokio::pin!(delay);

            // Race between primary/hedges completing and delay
            match select(primary.as_mut(), delay).await {
                Either::Left((result, _)) => {
                    // Primary completed first
                    return result;
                }
                Either::Right((_, _)) => {
                    // Delay expired, send hedge request
                    if hedges_sent < self.config.max_hedges {
                        hedge_futures.push(Box::pin(operation()));
                        hedges_sent += 1;

                        // Calculate next hedge delay
                        if self.config.exponential_backoff {
                            hedge_delay = Duration::from_secs_f64(
                                hedge_delay.as_secs_f64() * self.config.backoff_multiplier,
                            );
                        }
                    } else {
                        // Max hedges reached, wait for any to complete
                        break;
                    }
                }
            }

            // Check if any hedge completed
            for (_i, hedge) in hedge_futures.iter_mut().enumerate() {
                if let Some(result) = hedge.now_or_never() {
                    return result;
                }
            }
        }

        // Wait for first to complete
        tokio::select! {
            result = primary => result,
            result = async {
                for hedge in hedge_futures {
                    if let Some(result) = hedge.now_or_never() {
                        return result;
                    }
                }
                Err(ResilienceError::Timeout {
                    duration: hedge_delay,
                    context: Some("Hedge timeout".to_string()),
                })
            } => result,
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
    pub fn new(config: HedgeConfig) -> Self {
        Self {
            base_config: config,
            latency_tracker: Arc::new(tokio::sync::Mutex::new(LatencyTracker::new(1000))),
            target_percentile: 0.95, // Target P95
        }
    }

    /// Set target percentile for hedge delay calculation
    pub fn with_target_percentile(mut self, percentile: f64) -> Self {
        self.target_percentile = percentile;
        self
    }

    /// Execute with adaptive hedging
    pub async fn execute<T, F, Fut>(&self, operation: F) -> ResilienceResult<T>
    where
        F: Fn() -> Fut + Send + Sync,
        Fut: Future<Output = ResilienceResult<T>> + Send,
        T: Send,
    {
        let start = std::time::Instant::now();

        // Get adaptive hedge delay based on historical latencies
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

        // Record latency
        {
            let mut tracker = self.latency_tracker.lock().await;
            tracker.record(start.elapsed());
        }

        result
    }
}

/// Latency tracker for adaptive hedging
struct LatencyTracker {
    samples: Vec<Duration>,
    max_samples: usize,
    current_index: usize,
}

impl LatencyTracker {
    fn new(max_samples: usize) -> Self {
        Self {
            samples: Vec::with_capacity(max_samples),
            max_samples,
            current_index: 0,
        }
    }

    fn record(&mut self, latency: Duration) {
        if self.samples.len() < self.max_samples {
            self.samples.push(latency);
        } else {
            self.samples[self.current_index] = latency;
            self.current_index = (self.current_index + 1) % self.max_samples;
        }
    }

    fn percentile(&self, p: f64) -> Option<Duration> {
        if self.samples.is_empty() {
            return None;
        }

        let mut sorted = self.samples.clone();
        sorted.sort();

        let index = ((sorted.len() as f64 - 1.0) * p) as usize;
        Some(sorted[index])
    }
}

/// Bimodal hedge executor - uses different strategies for fast vs slow operations
pub struct BimodalHedgeExecutor {
    fast_threshold: Duration,
    #[allow(dead_code)]
    fast_config: HedgeConfig,
    slow_config: HedgeConfig,
}

impl BimodalHedgeExecutor {
    /// Create new bimodal hedge executor
    pub fn new(
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
        // Sample operation to determine if it's fast or slow
        let _sample_start = std::time::Instant::now();
        let sample_future = operation();

        // Wait for fast threshold
        tokio::select! {
            result = sample_future => {
                // Operation completed within fast threshold
                return result;
            }
            _ = sleep(self.fast_threshold) => {
                // Operation is slow, use slow config
                let executor = HedgeExecutor::new(self.slow_config.clone());
                return executor.execute(operation).await;
            }
        }
    }
}

//! Hedge request pattern — fires duplicate requests after a delay, returns the first success.
//!
//! Losing futures are aborted via `JoinHandle::abort()` when the first result arrives.

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio::time::{Instant, sleep};

use crate::{
    CallError,
    sink::{MetricsSink, NoopSink, ResilienceEvent},
};

// ── Config ────────────────────────────────────────────────────────────────────

/// Configuration for the hedge pattern.
#[derive(Debug, Clone)]
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

// ── HedgeExecutor ─────────────────────────────────────────────────────────────

/// Executes an operation with hedging: fires duplicate requests after a delay and returns
/// the first successful result, aborting all other in-flight requests.
pub struct HedgeExecutor {
    config: HedgeConfig,
    sink: Arc<dyn MetricsSink>,
}

impl HedgeExecutor {
    /// Create a new hedge executor.
    #[must_use]
    pub fn new(config: HedgeConfig) -> Self {
        Self {
            config,
            sink: Arc::new(NoopSink),
        }
    }

    /// Inject a metrics sink.
    #[must_use]
    pub fn with_sink(mut self, sink: impl MetricsSink + 'static) -> Self {
        self.sink = Arc::new(sink);
        self
    }

    /// Execute `operation` with hedging.
    ///
    /// - Returns the first `Ok(T)` result, aborting remaining requests.
    /// - Returns the last `Err` if all attempts fail.
    ///
    /// # Errors
    ///
    /// Returns `Err(CallError::Operation)` if all attempts (including hedges) fail,
    /// or `Err(CallError::Cancelled)` if all tasks were cancelled.
    #[allow(clippy::excessive_nesting)]
    pub async fn execute<T, E, F>(&self, operation: F) -> Result<T, CallError<E>>
    where
        T: Send + 'static,
        E: Send + 'static,
        F: Fn() -> Pin<Box<dyn Future<Output = Result<T, E>> + Send>> + Send + Sync,
    {
        let mut handles: Vec<JoinHandle<Result<T, E>>> = Vec::new();

        // Launch first request
        handles.push(tokio::spawn(operation()));

        let mut hedge_delay = self.config.hedge_delay;
        let mut hedges_sent = 0usize;
        let mut delay = Box::pin(sleep(hedge_delay));
        let mut last_err: Option<E> = None;

        loop {
            // If no more hedges allowed, just wait for all remaining handles
            if hedges_sent >= self.config.max_hedges {
                // Poll remaining handles in order
                #[allow(clippy::iter_with_drain)]
                for handle in handles.drain(..) {
                    match handle.await {
                        Ok(Ok(v)) => return Ok(v),
                        Ok(Err(e)) => last_err = Some(e),
                        Err(_join_err) => {} // task panicked/cancelled
                    }
                }
                return Err(
                    last_err.map_or(CallError::Cancelled { reason: None }, CallError::Operation)
                );
            }

            // Check if any in-flight handle is ready, or fire a hedge after the delay
            let ready_idx = poll_first_ready(&handles);

            tokio::select! {
                biased;

                // Prefer completed handles
                () = async {}, if ready_idx.is_some() => {
                    let Some(idx) = ready_idx else { continue };
                    match handles.remove(idx).await {
                        Ok(Ok(v)) => {
                            abort_all(&mut handles);
                            return Ok(v);
                        }
                        Ok(Err(e)) => {
                            if handles.is_empty() && hedges_sent >= self.config.max_hedges {
                                return Err(CallError::Operation(e));
                            }
                            last_err = Some(e);
                        }
                        Err(_) => {}
                    }
                }

                () = &mut delay => {
                    // Fire hedge
                    // Reason: max_hedges is a small config value, never exceeds u32.
                    #[allow(clippy::cast_possible_truncation)]
                    let hedge_num = (hedges_sent + 1) as u32;
                    self.sink.record(ResilienceEvent::HedgeFired { hedge_number: hedge_num });
                    handles.push(tokio::spawn(operation()));
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

/// Returns the index of the first handle that is `is_finished()`, or `None`.
fn poll_first_ready<T, E>(handles: &[JoinHandle<Result<T, E>>]) -> Option<usize> {
    handles.iter().position(JoinHandle::is_finished)
}

/// Abort all handles in the list.
fn abort_all<T, E>(handles: &mut Vec<JoinHandle<Result<T, E>>>) {
    for h in handles.drain(..) {
        h.abort();
    }
}

// ── AdaptiveHedgeExecutor ─────────────────────────────────────────────────────

/// Hedge executor that adjusts delay based on observed latency percentiles.
pub struct AdaptiveHedgeExecutor {
    base_config: HedgeConfig,
    latency_tracker: Arc<tokio::sync::Mutex<LatencyTracker>>,
    target_percentile: f64,
    sink: Arc<dyn MetricsSink>,
}

impl AdaptiveHedgeExecutor {
    /// Create a new adaptive hedge executor.
    #[must_use]
    pub fn new(config: HedgeConfig) -> Self {
        Self {
            base_config: config,
            latency_tracker: Arc::new(tokio::sync::Mutex::new(LatencyTracker::new(1000))),
            target_percentile: 0.95,
            sink: Arc::new(NoopSink),
        }
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

    /// Execute with adaptive hedging.
    ///
    /// # Errors
    ///
    /// Returns `Err(CallError::Operation)` if all attempts fail,
    /// or `Err(CallError::Cancelled)` if all tasks were cancelled.
    pub async fn execute<T, E, F>(&self, operation: F) -> Result<T, CallError<E>>
    where
        T: Send + 'static,
        E: Send + 'static,
        F: Fn() -> Pin<Box<dyn Future<Output = Result<T, E>> + Send>> + Send + Sync,
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
        let executor = HedgeExecutor {
            config,
            sink: Arc::clone(&self.sink),
        };
        let result = executor.execute(operation).await;

        self.latency_tracker.lock().await.record(start.elapsed());
        result
    }
}

// ── LatencyTracker ────────────────────────────────────────────────────────────

/// Ring-buffer latency tracker with O(n) percentile computation.
struct LatencyTracker {
    ring: VecDeque<Duration>,
    max_samples: usize,
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

    // Reason: u128 nanosecond values from Duration::as_nanos() are truncated to u64
    // (max ~584 years), and f64 precision loss is acceptable for percentile calculations.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    fn record(&mut self, latency: Duration) {
        if self.ring.len() == self.max_samples
            && let Some(oldest) = self.ring.pop_front()
        {
            let key = oldest.as_nanos() as u64;
            if let Some(c) = self.sorted.get_mut(&key) {
                if *c <= 1 {
                    self.sorted.remove(&key);
                } else {
                    *c -= 1;
                }
            }
        }
        *self.sorted.entry(latency.as_nanos() as u64).or_insert(0) += 1;
        self.ring.push_back(latency);
    }

    // Reason: f64 precision loss and sign loss are acceptable for percentile index calculation.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    fn percentile(&self, p: f64) -> Option<Duration> {
        if self.ring.is_empty() || !p.is_finite() {
            return None;
        }
        let target = ((self.ring.len() as f64 - 1.0) * p.clamp(0.0, 1.0)) as usize;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RecordingSink;

    #[tokio::test]
    async fn returns_first_success() {
        let counter = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let c = counter.clone();

        let executor = HedgeExecutor::new(HedgeConfig {
            hedge_delay: Duration::from_millis(50),
            max_hedges: 2,
            ..Default::default()
        });

        let result = executor
            .execute(|| {
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
        .with_sink(sink.clone());

        let _ = executor
            .execute(|| {
                Box::pin(async {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    Ok::<_, &str>("late")
                })
            })
            .await;

        assert!(sink.count("hedge_fired") > 0);
    }
}

// ===== action/polling.rs =====

use async_trait::async_trait;
use serde::{Serialize, Deserialize};
use std::time::{Duration, Instant};
use tokio::time;
use tokio::sync::{watch};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn, error, debug, instrument};
use futures_util::{Stream, StreamExt};
use std::pin::Pin;
use std::task::{Context, Poll};
use rand::Rng;
use crate::action::{Action, ActionContext, ActionError};

// ===== Core Types =====

/// Context trait for polling actions that extends the base ActionContext.
///
/// Provides callbacks and control mechanisms for polling operations,
/// allowing implementers to customize polling behavior and receive
/// notifications about polling progress.
#[async_trait]
pub trait PollingContext: ActionContext {
    /// Check if polling should be stopped.
    ///
    /// This method is called before each polling attempt to allow
    /// external control over the polling process.
    ///
    /// # Returns
    ///
    /// * `true` if polling should stop
    /// * `false` if polling should continue (default)
    async fn should_stop_polling(&self) -> bool {
        false
    }

    /// Callback invoked on each polling attempt.
    ///
    /// # Arguments
    ///
    /// * `attempt` - Current attempt number (1-indexed)
    /// * `elapsed` - Total time elapsed since polling started
    async fn on_polling_attempt(&self, attempt: u32, elapsed: Duration) {
        debug!("🔄 Polling attempt {} of {}", attempt, elapsed.as_millis());
    }

    /// Callback invoked when polling succeeds.
    ///
    /// # Arguments
    ///
    /// * `attempts` - Total number of attempts made
    /// * `total_time` - Total time taken for successful polling
    async fn on_polling_success(&self, attempts: u32, total_time: Duration) {
        info!("🎉 Polling succeeded after {} attempts in {:?}", attempts, total_time);
    }

    /// Callback invoked when polling fails permanently.
    ///
    /// # Arguments
    ///
    /// * `error` - Error message describing the failure
    /// * `attempts` - Total number of attempts made before failure
    async fn on_polling_failure(&self, error: &str, attempts: u32) {
        error!("❌ Polling failed after {} attempts: {}", attempts, error);
    }

    /// Callback invoked when a retry is scheduled.
    ///
    /// # Arguments
    ///
    /// * `attempt` - Current attempt number that failed
    /// * `reason` - Reason for the retry
    /// * `next_interval` - Duration until next attempt
    async fn on_polling_retry(&self, attempt: u32, reason: &str, next_interval: Duration) {
        warn!("🔁 Polling retry after {}: {}. Next interval: {:?}", attempt, reason, next_interval);
    }

    /// Get cancellation token for graceful shutdown.
    ///
    /// # Returns
    ///
    /// Optional cancellation token that can be used to cancel polling operations
    fn cancellation_token(&self) -> Option<CancellationToken> {
        None
    }

    /// Record polling metrics for monitoring and observability.
    ///
    /// # Arguments
    ///
    /// * `metrics` - Current polling metrics including attempt count, timing, etc.
    async fn record_polling_metrics(&self, metrics: PollingMetrics) {
        // Default implementation does nothing
        debug!("📊 Polling metrics: {:?}", metrics);
    }
}

/// Result of a polling condition check.
///
/// Represents the outcome of evaluating whether a polling condition
/// has been met, failed, or should be retried.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PollingResult<T> {
    /// Condition successfully met - polling can terminate with success.
    Success(T),

    /// Condition not yet met - continue polling with a normal interval.
    Continue,

    /// Condition permanently failed - terminate polling with error.
    Failed(String),

    /// Temporary error occurred - retry with backoff strategy.
    Retry(String),
}

/// Configuration for polling behavior.
///
/// Defines timing, retry logic, and resilience patterns for polling operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PollingConfig {
    /// Base interval between polling attempts.
    pub interval: Duration,

    /// Maximum number of polling attempts (None = infinite).
    pub max_attempts: Option<u32>,

    /// Overall timeout for the entire polling operation (None = no timeout).
    pub timeout: Option<Duration>,

    /// Exponential backoff configuration.
    pub backoff: Option<BackoffConfig>,

    /// Jitter configuration to randomize intervals.
    pub jitter: Option<JitterConfig>,

    /// Circuit breaker configuration for failure protection.
    pub circuit_breaker: Option<CircuitBreakerConfig>,
}

/// Configuration for exponential backoff strategy.
///
/// Controls how polling intervals increase after failures or continued
/// unsuccessful attempts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackoffConfig {
    /// Multiplier for exponential backoff (e.g., 1.5 means 50% increase).
    pub multiplier: f64,

    /// Maximum interval to prevent excessive delays.
    pub max_interval: Duration,

    /// Randomization factor (0.0 - 1.0) to add variance to intervals.
    pub randomization_factor: f64,
}

/// Configuration for adding jitter to polling intervals.
///
/// Jitter helps prevent thundering herd problems when multiple
/// polling operations are running simultaneously.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JitterConfig {
    /// Maximum amount of jitter to add.
    pub max_jitter: Duration,

    /// Type of jitter algorithm to use.
    pub jitter_type: JitterType,
}

/// Types of jitter algorithms for randomizing polling intervals.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JitterType {
    /// Add random time to the base interval.
    Additive,
    /// Multiply interval by a random factor.
    Multiplicative,
    /// Completely randomize the interval within bounds.
    Full,
}

/// Configuration for circuit breaker pattern.
///
/// Prevents cascading failures by temporarily stopping polling
/// when too many consecutive failures occur.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitBreakerConfig {
    /// Number of consecutive failures before opening the circuit.
    pub failure_threshold: u32,

    /// Time to wait before attempting to close the circuit.
    pub recovery_timeout: Duration,
}

impl Default for PollingConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(5),
            max_attempts: Some(10),
            timeout: Some(Duration::from_secs(300)), // 5 minutes
            backoff: Some(BackoffConfig {
                multiplier: 1.5,
                max_interval: Duration::from_secs(60),
                randomization_factor: 0.1,
            }),
            jitter: Some(JitterConfig {
                max_jitter: Duration::from_millis(500),
                jitter_type: JitterType::Additive,
            }),
            circuit_breaker: None,
        }
    }
}

/// Metrics for monitoring polling operations.
///
/// Provides detailed information about the current state and
/// progress of a polling operation for observability purposes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PollingMetrics {
    /// Current attempt number (1-indexed).
    pub attempt: u32,

    /// Total time elapsed since polling started.
    pub elapsed: Duration,

    /// Current interval being used between attempts.
    pub current_interval: Duration,

    /// Result of the most recent polling attempt.
    pub last_result: PollingResultType,

    /// Whether the circuit breaker is currently open.
    pub circuit_breaker_open: bool,

    /// Estimated remaining time until completion or timeout.
    pub estimated_remaining: Option<Duration>,
}

/// Simplified representation of polling result types for metrics.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PollingResultType {
    Success,
    Continue,
    Failed,
    Retry,
}

// ===== Polling Action Trait =====

/// Trait for actions that support polling-based execution.
///
/// Extends the base Action trait with polling-specific functionality,
/// allowing actions to be executed repeatedly until a condition is met.
#[async_trait]
pub trait PollingAction: Action {
    /// Output type produced when polling succeeds.
    ///
    /// Must be serializable and cloneable for use across async boundaries.
    type Output: Send + Sync + Clone + Serialize + for<'de> Deserialize<'de>;

    /// Check the polling condition to determine next action.
    ///
    /// This method is called repeatedly until it returns Success or Failed.
    /// Implementations should be idempotent and handle transient errors gracefully.
    ///
    /// # Arguments
    ///
    /// * `context` - Polling context providing callbacks and control
    ///
    /// # Returns
    ///
    /// * `PollingResult<Self::Output>` indicating the current state
    /// * `ActionError` if an unrecoverable error occurs
    async fn poll_condition<C>(&self, context: &C) -> Result<PollingResult<Self::Output>, ActionError>
    where
        C: PollingContext + Send + Sync;

    /// Get polling configuration for this action.
    ///
    /// Override this method to customize polling behavior for specific actions.
    fn polling_config(&self) -> PollingConfig {
        PollingConfig::default()
    }

    /// Execute polling with full resilience and monitoring features.
    ///
    /// Performs the complete polling loop with exponential backoff, jitter,
    /// circuit breaking, and comprehensive error handling.
    ///
    /// # Arguments
    ///
    /// * `context` - Polling context for callbacks and control
    ///
    /// # Returns
    ///
    /// * `Self::Output` on successful completion
    /// * `ActionError` on failure or timeout
    #[instrument(skip(self, context), fields(action_name = %self.name()))]
    async fn execute_polling<C>(&self, context: &C) -> Result<Self::Output, ActionError>
    where
        C: PollingContext + Send + Sync,
    {
        let config = self.polling_config();
        let executor = PollingExecutor::new(config);
        executor.execute(self, context).await
    }

    /// Execute polling as a reactive stream.
    ///
    /// Returns a stream that yields polling results as they become available,
    /// useful for real-time monitoring or reactive programming patterns.
    ///
    /// # Arguments
    ///
    /// * `context` - Polling context for callbacks and control
    ///
    /// # Returns
    ///
    /// Stream of polling results
    fn poll_stream<C>(
        &self,
        context: &C,
    ) -> Pin<Box<dyn Stream<Item = Result<PollingResult<Self::Output>, ActionError>> + Send + '_>>
    where
        C: PollingContext + Send + Sync,
        Self: Sync,
    {
        let config = self.polling_config();
        Box::pin(PollingStream::new(self, context, config))
    }

    /// Spawn polling operation in the background.
    ///
    /// Executes the polling operation on a separate task, allowing
    /// non-blocking operation with cancellation support.
    ///
    /// # Arguments
    ///
    /// * `context` - Arc-wrapped polling context for shared access
    ///
    /// # Returns
    ///
    /// JoinHandle that can be awaited or cancelled
    async fn spawn_polling<C>(
        &self,
        context: std::sync::Arc<C>,
    ) -> tokio::task::JoinHandle<Result<Self::Output, ActionError>>
    where
        C: PollingContext + 'static,
        Self: Clone + Send + 'static,
    {
        let action = self.clone();
        tokio::spawn(async move {
            action.execute_polling(context.as_ref()).await
        })
    }
}

// ===== Polling Executor =====

/// Executor responsible for managing the polling loop and applying resilience patterns.
///
/// Handles the complexity of exponential backoff, jitter, circuit breaking,
/// and timeout management while providing detailed metrics and callbacks.
pub struct PollingExecutor {
    /// Configuration defining polling behavior.
    config: PollingConfig,
    /// Optional circuit breaker for failure protection.
    circuit_breaker: Option<CircuitBreaker>,
}

impl PollingExecutor {
    /// Create a new polling executor with the given configuration.
    ///
    /// # Arguments
    ///
    /// * `config` - Polling configuration including timeouts, backoff, etc.
    pub fn new(config: PollingConfig) -> Self {
        let circuit_breaker = config.circuit_breaker.as_ref().map(|cb_config| {
            CircuitBreaker::new(cb_config.failure_threshold, cb_config.recovery_timeout)
        });

        Self {
            config,
            circuit_breaker,
        }
    }

    /// Execute the polling loop for the given action.
    ///
    /// Manages the complete polling lifecycle including interval calculation,
    /// timeout handling, circuit breaking, and comprehensive error handling.
    ///
    /// # Arguments
    ///
    /// * `action` - The polling action to execute
    /// * `context` - Polling context for callbacks and control
    ///
    /// # Returns
    ///
    /// * `A::Output` on successful completion
    /// * `ActionError` on failure, timeout, or cancellation
    #[instrument(skip(self, action, context))]
    pub async fn execute<A, C>(&self, action: &A, context: &C) -> Result<A::Output, ActionError>
    where
        A: PollingAction,
        C: PollingContext + Send + Sync,
    {
        // Check circuit breaker state before starting
        if let Some(ref cb) = self.circuit_breaker {
            if cb.is_open().await {
                return Err(ActionError::Execution {
                    message: "Circuit breaker is open".to_string(),
                });
            }
        }

        let start_time = Instant::now();
        let mut attempt = 0;
        let mut current_interval = self.config.interval;
        let mut consecutive_failures = 0;

        // Get cancellation token for graceful shutdown
        let cancellation = context.cancellation_token();

        loop {
            attempt += 1;
            let elapsed = start_time.elapsed();

            // Check for cancellation via token
            if let Some(ref token) = cancellation {
                if token.is_cancelled() {
                    info!("Polling cancelled by token");
                    return Err(ActionError::Cancelled);
                }
            }

            // Check for cancellation via context
            if context.should_stop_polling().await {
                info!("Polling stopped by context");
                return Err(ActionError::Cancelled);
            }

            // Check overall timeout
            if let Some(timeout) = self.config.timeout {
                if elapsed >= timeout {
                    warn!("Polling timeout exceeded: {:?}", elapsed);
                    return Err(ActionError::Timeout {
                        timeout_ms: timeout.as_millis() as u64
                    });
                }
            }

            // Check maximum attempts
            if let Some(max_attempts) = self.config.max_attempts {
                if attempt > max_attempts {
                    warn!("Max polling attempts reached: {}", attempt - 1);
                    return Err(ActionError::Execution {
                        message: format!("Max attempts ({}) exceeded", max_attempts),
                    });
                }
            }

            // Notify about the current attempt
            context.on_polling_attempt(attempt, elapsed).await;

            // Prepare metrics for monitoring
            let metrics = PollingMetrics {
                attempt,
                elapsed,
                current_interval,
                last_result: PollingResultType::Continue, // Will be updated below
                circuit_breaker_open: self.circuit_breaker.as_ref()
                    .map(|cb| tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current().block_on(cb.is_open())
                    }))
                    .unwrap_or(false),
                estimated_remaining: self.estimate_remaining_time(attempt, elapsed),
            };

            // Execute the polling condition with timeout protection
            let result = match tokio::time::timeout(
                Duration::from_secs(30), // Timeout for individual condition check
                action.poll_condition(context)
            ).await {
                Ok(res) => res?,
                Err(_) => {
                    warn!("Polling condition check timed out");
                    PollingResult::Retry("Condition check timeout".to_string())
                }
            };

            // Update metrics with the actual result
            let mut updated_metrics = metrics;
            updated_metrics.last_result = match &result {
                PollingResult::Success(_) => PollingResultType::Success,
                PollingResult::Continue => PollingResultType::Continue,
                PollingResult::Failed(_) => PollingResultType::Failed,
                PollingResult::Retry(_) => PollingResultType::Retry,
            };

            context.record_polling_metrics(updated_metrics).await;

            // Handle the polling result
            match result {
                PollingResult::Success(output) => {
                    context.on_polling_success(attempt, elapsed).await;

                    // Notify circuit breaker of success
                    if let Some(ref cb) = self.circuit_breaker {
                        cb.record_success().await;
                    }

                    return Ok(output);
                }

                PollingResult::Continue => {
                    debug!("Polling condition not met, continuing...");
                    consecutive_failures = 0; // Reset failure counter

                    // Wait for the configured interval
                    self.wait_with_cancellation(current_interval, &cancellation).await?;

                    // Calculate next interval with backoff and jitter
                    current_interval = self.calculate_next_interval(current_interval, attempt);
                }

                PollingResult::Failed(reason) => {
                    error!("Polling failed permanently: {}", reason);
                    context.on_polling_failure(&reason, attempt).await;

                    // Notify circuit breaker of failure
                    if let Some(ref cb) = self.circuit_breaker {
                        cb.record_failure().await;
                    }

                    return Err(ActionError::Execution { message: reason });
                }

                PollingResult::Retry(reason) => {
                    consecutive_failures += 1;

                    // Check circuit breaker state
                    if let Some(ref cb) = self.circuit_breaker {
                        cb.record_failure().await;
                        if cb.is_open().await {
                            return Err(ActionError::Execution {
                                message: "Circuit breaker opened due to consecutive failures".to_string(),
                            });
                        }
                    }

                    let retry_interval = self.calculate_retry_interval(current_interval, consecutive_failures);
                    context.on_polling_retry(attempt, &reason, retry_interval).await;

                    // Wait for retry interval
                    self.wait_with_cancellation(retry_interval, &cancellation).await?;

                    // Update interval for next iteration
                    current_interval = self.calculate_next_interval(current_interval, attempt);
                }
            }
        }
    }

    /// Wait for a duration while respecting cancellation tokens.
    ///
    /// # Arguments
    ///
    /// * `duration` - How long to wait
    /// * `cancellation` - Optional cancellation token
    ///
    /// # Returns
    ///
    /// * `Ok(())` if wait completed normally
    /// * `Err(ActionError::Cancelled)` if cancelled
    async fn wait_with_cancellation(
        &self,
        duration: Duration,
        cancellation: &Option<CancellationToken>,
    ) -> Result<(), ActionError> {
        if let Some(ref token) = cancellation {
            tokio::select! {
                _ = time::sleep(duration) => {},
                _ = token.cancelled() => {
                    return Err(ActionError::Cancelled);
                }
            }
        } else {
            time::sleep(duration).await;
        }
        Ok(())
    }

    /// Calculate the next polling interval using exponential backoff and jitter.
    ///
    /// Applies the configured backoff strategy, randomization, and jitter
    /// to determine the optimal interval for the next polling attempt.
    ///
    /// # Arguments
    ///
    /// * `current` - Current interval duration
    /// * `attempt` - Current attempt number (used for exponential calculation)
    ///
    /// # Returns
    ///
    /// The calculated duration for the next interval
    fn calculate_next_interval(&self, current: Duration, attempt: u32) -> Duration {
        let mut next = current;

        // Apply exponential backoff based on attempt number
        if let Some(ref backoff) = self.config.backoff {
            // Use attempt number in the calculation for proper exponential backoff
            let attempt_multiplier = backoff.multiplier.powi(attempt.saturating_sub(1) as i32);
            let multiplied = Duration::from_secs_f64(
                current.as_secs_f64() * attempt_multiplier
            );
            next = multiplied.min(backoff.max_interval);

            // Apply randomization to reduce thundering herd effects
            if backoff.randomization_factor > 0.0 {
                next = self.apply_randomization(next, backoff.randomization_factor);
            }
        }

        // Apply jitter for additional randomization
        if let Some(ref jitter) = self.config.jitter {
            next = self.apply_jitter(next, jitter);
        }

        next
    }

    /// Calculate a retry interval for failed attempts with aggressive backoff.
    ///
    /// Uses a more aggressive backoff strategy for retries compared to
    /// normal interval calculation to handle transient errors effectively.
    ///
    /// # Arguments
    ///
    /// * `base_interval` - Base interval to multiply
    /// * `consecutive_failures` - Number of consecutive failures
    ///
    /// # Returns
    ///
    /// Calculated retry interval duration
    fn calculate_retry_interval(&self, base_interval: Duration, consecutive_failures: u32) -> Duration {
        // Use more aggressive backoff for retries (exponential base 2)
        let multiplier = 2.0_f64.powi(consecutive_failures as i32).min(8.0); // Cap at 8x
        let retry_interval = Duration::from_secs_f64(base_interval.as_secs_f64() * multiplier);

        // Limit to configured maximum interval
        if let Some(ref backoff) = self.config.backoff {
            retry_interval.min(backoff.max_interval)
        } else {
            retry_interval.min(Duration::from_secs(60))
        }
    }

    /// Apply a randomization factor to a duration.
    ///
    /// # Arguments
    ///
    /// * `duration` - Base duration to randomize
    /// * `factor` - Randomization factor (0.0 to 1.0)
    ///
    /// # Returns
    ///
    /// Randomized duration
    fn apply_randomization(&self, duration: Duration, factor: f64) -> Duration {
        use rand::Rng;
        let mut rng = rand::rng();
        let randomization = rng.random_range(-factor..=factor);
        let randomized = duration.as_secs_f64() * (1.0 + randomization);
        Duration::from_secs_f64(randomized.max(0.0))
    }

    /// Apply jitter to a duration based on the configured jitter type.
    ///
    /// # Arguments
    ///
    /// * `duration` - Base duration
    /// * `jitter` - Jitter configuration
    ///
    /// # Returns
    ///
    /// Duration with applied jitter
    fn apply_jitter(&self, duration: Duration, jitter: &JitterConfig) -> Duration {
        use rand::Rng;
        let mut rng = rand::rng();

        match jitter.jitter_type {
            JitterType::Additive => {
                let jitter_amount = rng.random_range(Duration::ZERO..=jitter.max_jitter);
                duration + jitter_amount
            }
            JitterType::Multiplicative => {
                let factor = rng.random_range(0.5..=1.5);
                Duration::from_secs_f64(duration.as_secs_f64() * factor)
            }
            JitterType::Full => {
                rng.random_range(Duration::ZERO..=duration + jitter.max_jitter)
            }
        }
    }

    /// Estimate the remaining time until polling completion.
    ///
    /// Provides a best-effort estimate based on current progress and configuration.
    ///
    /// # Arguments
    ///
    /// * `current_attempt` - Current attempt number
    /// * `elapsed` - Time elapsed so far
    ///
    /// # Returns
    ///
    /// Estimated remaining duration, if calculable
    fn estimate_remaining_time(&self, current_attempt: u32, elapsed: Duration) -> Option<Duration> {
        if let Some(max_attempts) = self.config.max_attempts {
            if current_attempt > 0 {
                let avg_time_per_attempt = elapsed.as_secs_f64() / current_attempt as f64;
                let remaining_attempts = max_attempts.saturating_sub(current_attempt);
                return Some(Duration::from_secs_f64(avg_time_per_attempt * remaining_attempts as f64));
            }
        }

        if let Some(timeout) = self.config.timeout {
            return Some(timeout.saturating_sub(elapsed));
        }

        None
    }
}

// ===== Circuit Breaker =====

/// Circuit breaker implementation for preventing cascading failures.
///
/// Tracks failure rates and temporarily stops operations when failures
/// exceed the configured threshold, allowing systems to recover.
pub struct CircuitBreaker {
    /// Number of failures needed to open the circuit.
    failure_threshold: u32,
    /// Time to wait before attempting to close the circuit.
    recovery_timeout: Duration,
    /// Current failure count.
    failures: watch::Sender<u32>,
    failures_rx: watch::Receiver<u32>,
    /// Timestamp of the last failure.
    last_failure: watch::Sender<Option<Instant>>,
    last_failure_rx: watch::Receiver<Option<Instant>>,
}

impl CircuitBreaker {
    /// Create a new circuit breaker with the specified configuration.
    ///
    /// # Arguments
    ///
    /// * `failure_threshold` - Number of failures before opening
    /// * `recovery_timeout` - Time to wait before attempting recovery
    pub fn new(failure_threshold: u32, recovery_timeout: Duration) -> Self {
        let (failures, failures_rx) = watch::channel(0);
        let (last_failure, last_failure_rx) = watch::channel(None);

        Self {
            failure_threshold,
            recovery_timeout,
            failures,
            failures_rx,
            last_failure,
            last_failure_rx,
        }
    }

    /// Check if the circuit breaker is currently open.
    ///
    /// # Returns
    ///
    /// * `true` if the circuit is open (requests should be blocked)
    /// * `false` if the circuit is closed (requests can proceed)
    pub async fn is_open(&self) -> bool {
        let failures = *self.failures_rx.borrow();
        if failures < self.failure_threshold {
            return false;
        }

        if let Some(last_failure_time) = *self.last_failure_rx.borrow() {
            last_failure_time.elapsed() < self.recovery_timeout
        } else {
            false
        }
    }

    /// Record a failure, potentially opening the circuit.
    pub async fn record_failure(&self) {
        let _ = self.failures.send_modify(|f| *f += 1);
        let _ = self.last_failure.send(Some(Instant::now()));
    }

    /// Record a success, closing the circuit and resetting failure count.
    pub async fn record_success(&self) {
        let _ = self.failures.send(0);
        let _ = self.last_failure.send(None);
    }
}

// ===== Polling Stream =====

/// Stream implementation for reactive polling operations.
///
/// Provides a Stream interface that yields polling results as they
/// become available, useful for real-time monitoring and reactive programming.
pub struct PollingStream<'a, A, C> {
    /// Reference to the polling action.
    action: &'a A,
    /// Reference to the polling context.
    context: &'a C,
    /// Polling configuration.
    config: PollingConfig,
    /// Tokio interval for timing.
    interval: time::Interval,
    /// Current attempt number.
    attempt: u32,
    /// Start time for timeout calculations.
    start_time: Instant,
    /// Current interval duration.
    current_interval: Duration,
}

impl<'a, A, C> PollingStream<'a, A, C> {
    /// Create a new polling stream.
    ///
    /// # Arguments
    ///
    /// * `action` - The polling action to execute
    /// * `context` - Polling context for callbacks
    /// * `config` - Polling configuration
    pub fn new(action: &'a A, context: &'a C, config: PollingConfig) -> Self {
        let mut interval = time::interval(config.interval);
        interval.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

        Self {
            action,
            context,
            current_interval: config.interval,
            config,
            interval,
            attempt: 0,
            start_time: Instant::now(),
        }
    }
}

impl<'a, A, C> Stream for PollingStream<'a, A, C>
where
    A: PollingAction + Sync,
    C: PollingContext + Sync,
{
    type Item = Result<PollingResult<A::Output>, ActionError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let elapsed = self.start_time.elapsed();

        // Check timeout constraint
        if let Some(timeout) = self.config.timeout {
            if elapsed >= timeout {
                return Poll::Ready(None);
            }
        }

        // Check maximum attempts constraint
        if let Some(max_attempts) = self.config.max_attempts {
            if self.attempt >= max_attempts {
                return Poll::Ready(None);
            }
        }

        // Wait for next interval tick
        match self.interval.poll_tick(cx) {
            Poll::Ready(_) => {
                self.attempt += 1;

                // Create future for condition check
                let fut = self.action.poll_condition(self.context);
                let mut pinned = Box::pin(fut);

                match pinned.as_mut().poll(cx) {
                    Poll::Ready(result) => {
                        // Update interval for next iteration using backoff strategy
                        let executor = PollingExecutor::new(self.config.clone());
                        self.current_interval = executor.calculate_next_interval(self.current_interval, self.attempt);
                        self.interval = time::interval(self.current_interval);
                        self.interval.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

                        Poll::Ready(Some(result))
                    }
                    Poll::Pending => Poll::Pending,
                }
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

// ===== Utility Functions =====

/// Create a simple polling configuration with basic parameters.
///
/// Provides a straightforward configuration without advanced features
/// like backoff, jitter, or circuit breaking.
///
/// # Arguments
///
/// * `interval_secs` - Interval between attempts in seconds
/// * `max_attempts` - Maximum number of attempts before giving up
///
/// # Returns
///
/// Basic polling configuration
pub fn simple_polling_config(interval_secs: u64, max_attempts: u32) -> PollingConfig {
    PollingConfig {
        interval: Duration::from_secs(interval_secs),
        max_attempts: Some(max_attempts),
        timeout: Some(Duration::from_secs(interval_secs * max_attempts as u64 * 2)),
        backoff: None,
        jitter: None,
        circuit_breaker: None,
    }
}

/// Create an aggressive polling configuration with fast retries and circuit breaking.
///
/// Suitable for scenarios where quick response is important and failures
/// should be handled aggressively with circuit breaking protection.
///
/// # Returns
///
/// Aggressive polling configuration with:
/// - 1 second initial interval
/// - 20 maximum attempts
/// - 2x exponential backoff with 20% randomization
/// - Additive jitter up to 2 seconds
/// - Circuit breaker opening after 3 failures
pub fn aggressive_polling_config() -> PollingConfig {
    PollingConfig {
        interval: Duration::from_secs(1),
        max_attempts: Some(20),
        timeout: Some(Duration::from_secs(300)),
        backoff: Some(BackoffConfig {
            multiplier: 2.0,
            max_interval: Duration::from_secs(30),
            randomization_factor: 0.2,
        }),
        jitter: Some(JitterConfig {
            max_jitter: Duration::from_secs(2),
            jitter_type: JitterType::Additive,
        }),
        circuit_breaker: Some(CircuitBreakerConfig {
            failure_threshold: 3,
            recovery_timeout: Duration::from_secs(30),
        }),
    }
}

/// Create a configuration suitable for long-running polling operations.
///
/// Designed for operations that may take extended periods to complete,
/// with conservative backoff and no strict limits on attempts or time.
///
/// # Returns
///
/// Long-running polling configuration with:
/// - 30 second initial interval
/// - No maximum attempts (infinite)
/// - No overall timeout
/// - Conservative 1.1x backoff with minimal randomization
/// - Small additive jitter to prevent synchronization
/// - No circuit breaker
pub fn long_polling_config() -> PollingConfig {
    PollingConfig {
        interval: Duration::from_secs(30),
        max_attempts: None, // Infinite attempts
        timeout: None,      // No timeout
        backoff: Some(BackoffConfig {
            multiplier: 1.1,
            max_interval: Duration::from_secs(300), // 5 minutes maximum
            randomization_factor: 0.05,
        }),
        jitter: Some(JitterConfig {
            max_jitter: Duration::from_secs(5),
            jitter_type: JitterType::Additive,
        }),
        circuit_breaker: None,
    }
}
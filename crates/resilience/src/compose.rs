//! Composable resilience patterns with layer-based architecture
//!
//! This module provides layer composition for building resilience pipelines:
//!
//! - **Layer traits** for composable middleware
//! - **`LayerBuilder`** for fluent API construction
//! - **Timeout, Retry, `CircuitBreaker`, Bulkhead** layers
//!
//! # Layer Composition
//!
//! ```rust,ignore
//! use nebula_resilience::compose::LayerBuilder;
//! use std::time::Duration;
//!
//! let chain = LayerBuilder::new()
//!     .with_timeout(Duration::from_secs(5))
//!     .with_retry_exponential(3, Duration::from_millis(100))
//!     .build();
//! ```

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use futures::stream::FuturesUnordered;

use crate::{
    ResilienceError, ResilienceResult,
    core::CancellationContext,
    manager::RetryableOperation,
    observability::sink::CircuitState,
    patterns::{
        bulkhead::Bulkhead,
        circuit_breaker::{CircuitBreaker, Outcome as CircuitOutcome},
        fallback::FallbackStrategy,
        hedge::HedgeConfig,
        rate_limiter::{AnyRateLimiter, RateLimiter},
        timeout::timeout,
    },
    policy::RetryPolicyConfig,
};

// =============================================================================
// LAYER TRAITS
// =============================================================================

/// Operation wrapper that can be boxed
pub struct BoxedOperation<T> {
    operation: Arc<dyn RetryableOperation<T> + Send + Sync>,
}

impl<T> BoxedOperation<T> {
    /// Create a new boxed operation
    pub fn new<Op>(operation: Op) -> Self
    where
        Op: RetryableOperation<T> + Send + Sync + 'static,
    {
        Self {
            operation: Arc::new(operation),
        }
    }

    /// Create from an Arc
    pub fn from_arc(operation: Arc<dyn RetryableOperation<T> + Send + Sync>) -> Self {
        Self { operation }
    }
}

impl<T: Send + Sync> RetryableOperation<T> for BoxedOperation<T> {
    fn execute<'a>(&'a self) -> Pin<Box<dyn Future<Output = ResilienceResult<T>> + Send + 'a>> {
        self.operation.execute()
    }
}

impl<T> Clone for BoxedOperation<T> {
    fn clone(&self) -> Self {
        Self {
            operation: self.operation.clone(),
        }
    }
}

/// Middleware layer that can be applied to operations
pub trait ResilienceLayer<T>: Send + Sync {
    /// Apply this layer to an operation
    fn apply<'a>(
        &'a self,
        operation: &'a BoxedOperation<T>,
        next: &'a (dyn LayerStack<T> + Send + Sync),
        cancellation: Option<&'a CancellationContext>,
    ) -> Pin<Box<dyn Future<Output = ResilienceResult<T>> + Send + 'a>>;

    /// Get layer name for debugging
    fn name(&self) -> &'static str;
}

/// Stack of layers that can be applied
pub trait LayerStack<T>: Send + Sync {
    /// Execute the operation with remaining layers
    fn execute<'a>(
        &'a self,
        operation: &'a BoxedOperation<T>,
    ) -> Pin<Box<dyn Future<Output = ResilienceResult<T>> + Send + 'a>> {
        self.execute_with_cancellation(operation, None)
    }

    /// Execute the operation with remaining layers and cooperative cancellation.
    fn execute_with_cancellation<'a>(
        &'a self,
        operation: &'a BoxedOperation<T>,
        cancellation: Option<&'a CancellationContext>,
    ) -> Pin<Box<dyn Future<Output = ResilienceResult<T>> + Send + 'a>>;
}

// =============================================================================
// LAYER IMPLEMENTATIONS
// =============================================================================

/// Terminal layer that just executes the operation
pub struct TerminalLayer;

impl<T: Send + Sync + 'static> LayerStack<T> for TerminalLayer {
    fn execute_with_cancellation<'a>(
        &'a self,
        operation: &'a BoxedOperation<T>,
        cancellation: Option<&'a CancellationContext>,
    ) -> Pin<Box<dyn Future<Output = ResilienceResult<T>> + Send + 'a>> {
        Box::pin(async move {
            if let Some(ctx) = cancellation
                && ctx.is_cancelled()
            {
                return Err(cancelled_error(ctx));
            }

            operation.execute().await
        })
    }
}

/// A composed stack of resilience layers
pub struct ComposedStack<T> {
    layer: Arc<dyn ResilienceLayer<T> + Send + Sync>,
    next: Arc<dyn LayerStack<T> + Send + Sync>,
}

impl<T: Send + Sync + 'static> LayerStack<T> for ComposedStack<T> {
    fn execute_with_cancellation<'a>(
        &'a self,
        operation: &'a BoxedOperation<T>,
        cancellation: Option<&'a CancellationContext>,
    ) -> Pin<Box<dyn Future<Output = ResilienceResult<T>> + Send + 'a>> {
        self.layer
            .apply(operation, self.next.as_ref(), cancellation)
    }
}

fn cancelled_error(cancellation: &CancellationContext) -> ResilienceError {
    ResilienceError::Cancelled {
        reason: cancellation.reason().map(str::to_owned),
    }
}

/// Timeout layer
pub struct TimeoutLayer {
    duration: Duration,
}

impl TimeoutLayer {
    pub(crate) const fn new(duration: Duration) -> Self {
        Self { duration }
    }
}

impl<T: Send + Sync + 'static> ResilienceLayer<T> for TimeoutLayer {
    fn apply<'a>(
        &'a self,
        operation: &'a BoxedOperation<T>,
        next: &'a (dyn LayerStack<T> + Send + Sync),
        cancellation: Option<&'a CancellationContext>,
    ) -> Pin<Box<dyn Future<Output = ResilienceResult<T>> + Send + 'a>> {
        let duration = self.duration;
        Box::pin(async move {
            timeout(duration, next.execute_with_cancellation(operation, cancellation))
                .await
                .unwrap_or_else(|_| {
                    Err(ResilienceError::Timeout {
                        duration,
                        context: Some("Layer timeout".to_string()),
                    })
                })
        })
    }

    fn name(&self) -> &'static str {
        "timeout"
    }
}

/// Retry layer using `RetryPolicyConfig`
pub struct RetryLayer {
    config: RetryPolicyConfig,
}

impl RetryLayer {
    pub(crate) const fn new(config: RetryPolicyConfig) -> Self {
        Self { config }
    }
}

impl<T: Send + Sync + 'static> ResilienceLayer<T> for RetryLayer {
    fn apply<'a>(
        &'a self,
        operation: &'a BoxedOperation<T>,
        next: &'a (dyn LayerStack<T> + Send + Sync),
        cancellation: Option<&'a CancellationContext>,
    ) -> Pin<Box<dyn Future<Output = ResilienceResult<T>> + Send + 'a>> {
        Box::pin(async move {
            let mut last_error = None;

            for attempt in 0..self.config.max_attempts {
                if let Some(ctx) = cancellation
                    && ctx.is_cancelled()
                {
                    return Err(cancelled_error(ctx));
                }

                let result = next
                    .execute_with_cancellation(operation, cancellation)
                    .await;

                match result {
                    Ok(value) => return Ok(value),
                    Err(e) if attempt + 1 == self.config.max_attempts => {
                        last_error = Some(e);
                        break;
                    }
                    Err(e) if e.is_retryable() => {
                        last_error = Some(e);

                        if let Some(delay) = self.config.delay_for_attempt(attempt) {
                            if let Some(ctx) = cancellation {
                                tokio::select! {
                                    () = tokio::time::sleep(delay) => {}
                                    () = ctx.token().cancelled() => return Err(cancelled_error(ctx)),
                                }
                            } else {
                                tokio::time::sleep(delay).await;
                            }
                        }
                    }
                    Err(e) => {
                        return Err(e);
                    }
                }
            }

            Err(ResilienceError::RetryLimitExceeded {
                attempts: self.config.max_attempts,
                last_error: last_error.map(Box::new),
            })
        })
    }

    fn name(&self) -> &'static str {
        "retry"
    }
}

/// Circuit breaker layer
pub struct CircuitBreakerLayer {
    circuit_breaker: Arc<CircuitBreaker>,
}

impl CircuitBreakerLayer {
    pub(crate) const fn new(circuit_breaker: Arc<CircuitBreaker>) -> Self {
        Self { circuit_breaker }
    }
}

impl<T: Send + Sync + 'static> ResilienceLayer<T> for CircuitBreakerLayer {
    fn apply<'a>(
        &'a self,
        operation: &'a BoxedOperation<T>,
        next: &'a (dyn LayerStack<T> + Send + Sync),
        cancellation: Option<&'a CancellationContext>,
    ) -> Pin<Box<dyn Future<Output = ResilienceResult<T>> + Send + 'a>> {
        Box::pin(async move {
            // Check if circuit is open
            if self.circuit_breaker.circuit_state() == CircuitState::Open {
                return Err(ResilienceError::circuit_breaker_open(
                    crate::CircuitBreakerOpenState::Open,
                ));
            }

            let result = next
                .execute_with_cancellation(operation, cancellation)
                .await;

            match &result {
                Ok(_) => self.circuit_breaker.record_outcome(CircuitOutcome::Success),
                Err(_e) => self.circuit_breaker.record_outcome(CircuitOutcome::Failure),
            }

            result
        })
    }

    fn name(&self) -> &'static str {
        "circuit_breaker"
    }
}

/// Bulkhead layer
pub struct BulkheadLayer {
    bulkhead: Arc<Bulkhead>,
}

impl BulkheadLayer {
    pub(crate) const fn new(bulkhead: Arc<Bulkhead>) -> Self {
        Self { bulkhead }
    }
}

impl<T: Send + Sync + 'static> ResilienceLayer<T> for BulkheadLayer {
    fn apply<'a>(
        &'a self,
        operation: &'a BoxedOperation<T>,
        next: &'a (dyn LayerStack<T> + Send + Sync),
        cancellation: Option<&'a CancellationContext>,
    ) -> Pin<Box<dyn Future<Output = ResilienceResult<T>> + Send + 'a>> {
        Box::pin(async move {
            let _permit = self.bulkhead.acquire().await?;
            next.execute_with_cancellation(operation, cancellation)
                .await
        })
    }

    fn name(&self) -> &'static str {
        "bulkhead"
    }
}

/// Rate limiter layer
pub struct RateLimiterLayer {
    limiter: Arc<AnyRateLimiter>,
}

impl RateLimiterLayer {
    pub(crate) fn new(limiter: Arc<AnyRateLimiter>) -> Self {
        Self { limiter }
    }
}

impl<T: Send + Sync + 'static> ResilienceLayer<T> for RateLimiterLayer {
    fn apply<'a>(
        &'a self,
        operation: &'a BoxedOperation<T>,
        next: &'a (dyn LayerStack<T> + Send + Sync),
        cancellation: Option<&'a CancellationContext>,
    ) -> Pin<Box<dyn Future<Output = ResilienceResult<T>> + Send + 'a>> {
        Box::pin(async move {
            self.limiter.acquire().await?;
            next.execute_with_cancellation(operation, cancellation).await
        })
    }

    fn name(&self) -> &'static str {
        "rate_limiter"
    }
}

/// Fallback layer — runs the fallback strategy when the inner stack returns an error
pub struct FallbackLayer<T> {
    strategy: Arc<dyn FallbackStrategy<T>>,
}

impl<T> FallbackLayer<T> {
    pub(crate) fn new(strategy: Arc<dyn FallbackStrategy<T>>) -> Self {
        Self { strategy }
    }
}

impl<T: Send + Sync + 'static> ResilienceLayer<T> for FallbackLayer<T> {
    fn apply<'a>(
        &'a self,
        operation: &'a BoxedOperation<T>,
        next: &'a (dyn LayerStack<T> + Send + Sync),
        cancellation: Option<&'a CancellationContext>,
    ) -> Pin<Box<dyn Future<Output = ResilienceResult<T>> + Send + 'a>> {
        Box::pin(async move {
            match next.execute_with_cancellation(operation, cancellation).await {
                Ok(value) => Ok(value),
                Err(error) => {
                    if self.strategy.should_fallback(&error) {
                        self.strategy.fallback(error).await
                    } else {
                        Err(error)
                    }
                }
            }
        })
    }

    fn name(&self) -> &'static str {
        "fallback"
    }
}

/// Hedge layer — issues a redundant request after a delay and returns the first response
pub struct HedgeLayer {
    config: HedgeConfig,
}

impl HedgeLayer {
    pub(crate) fn new(config: HedgeConfig) -> Self {
        Self { config }
    }
}

impl<T: Send + Sync + 'static> ResilienceLayer<T> for HedgeLayer {
    fn apply<'a>(
        &'a self,
        operation: &'a BoxedOperation<T>,
        next: &'a (dyn LayerStack<T> + Send + Sync),
        _cancellation: Option<&'a CancellationContext>,
    ) -> Pin<Box<dyn Future<Output = ResilienceResult<T>> + Send + 'a>> {
        let config = &self.config;
        Box::pin(async move {
            let timeout_duration = config.hedge_delay;

            let mut in_flight: FuturesUnordered<
                Pin<Box<dyn Future<Output = ResilienceResult<T>> + Send + 'a>>,
            > = FuturesUnordered::new();
            in_flight.push(next.execute(operation));

            let mut hedge_delay = config.hedge_delay;
            let mut hedges_sent = 0usize;
            let mut delay = Box::pin(tokio::time::sleep(hedge_delay));

            loop {
                if hedges_sent >= config.max_hedges {
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
                        return maybe_result.unwrap_or(Err(ResilienceError::Timeout {
                            duration: timeout_duration,
                            context: Some("Hedge timeout".to_string()),
                        }));
                    }
                    () = &mut delay => {
                        in_flight.push(next.execute(operation));
                        hedges_sent += 1;

                        if config.exponential_backoff {
                            hedge_delay = Duration::from_secs_f64(
                                hedge_delay.as_secs_f64() * config.backoff_multiplier,
                            );
                        }

                        delay
                            .as_mut()
                            .reset(tokio::time::Instant::now() + hedge_delay);
                    }
                }
            }
        })
    }

    fn name(&self) -> &'static str {
        "hedge"
    }
}

// =============================================================================
// LAYER BUILDER
// =============================================================================

/// Builder for composing resilience layers
pub struct LayerBuilder<T> {
    layers: Vec<Arc<dyn ResilienceLayer<T> + Send + Sync>>,
}

impl<T: Send + Sync + 'static> Default for LayerBuilder<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Send + Sync + 'static> LayerBuilder<T> {
    /// Create new layer builder
    #[must_use]
    pub fn new() -> Self {
        Self { layers: Vec::new() }
    }

    /// Add timeout layer
    #[must_use = "builder methods must be chained or built"]
    pub fn with_timeout(mut self, duration: Duration) -> Self {
        self.layers.push(Arc::new(TimeoutLayer::new(duration)));
        self
    }

    /// Add retry layer with configuration
    #[must_use = "builder methods must be chained or built"]
    pub fn with_retry(mut self, config: RetryPolicyConfig) -> Self {
        self.layers.push(Arc::new(RetryLayer::new(config)));
        self
    }

    /// Add retry layer with exponential backoff
    #[must_use = "builder methods must be chained or built"]
    pub fn with_retry_exponential(mut self, max_attempts: usize, base_delay: Duration) -> Self {
        self.layers
            .push(Arc::new(RetryLayer::new(RetryPolicyConfig::exponential(
                max_attempts,
                base_delay,
            ))));
        self
    }

    /// Add retry layer with fixed delay
    #[must_use = "builder methods must be chained or built"]
    pub fn with_retry_fixed(mut self, max_attempts: usize, delay: Duration) -> Self {
        self.layers
            .push(Arc::new(RetryLayer::new(RetryPolicyConfig::fixed(
                max_attempts,
                delay,
            ))));
        self
    }

    /// Add circuit breaker layer
    #[must_use = "builder methods must be chained or built"]
    pub fn with_circuit_breaker(mut self, circuit_breaker: Arc<CircuitBreaker>) -> Self {
        self.layers
            .push(Arc::new(CircuitBreakerLayer::new(circuit_breaker)));
        self
    }

    /// Add bulkhead layer
    #[must_use = "builder methods must be chained or built"]
    pub fn with_bulkhead(mut self, bulkhead: Arc<Bulkhead>) -> Self {
        self.layers.push(Arc::new(BulkheadLayer::new(bulkhead)));
        self
    }

    /// Add rate limiter layer
    #[must_use = "builder methods must be chained or built"]
    pub fn with_rate_limiter(mut self, limiter: Arc<AnyRateLimiter>) -> Self {
        self.layers.push(Arc::new(RateLimiterLayer::new(limiter)));
        self
    }

    /// Add fallback layer
    #[must_use = "builder methods must be chained or built"]
    pub fn with_fallback(mut self, strategy: Arc<dyn FallbackStrategy<T>>) -> Self {
        self.layers.push(Arc::new(FallbackLayer::new(strategy)));
        self
    }

    /// Add hedge layer
    #[must_use = "builder methods must be chained or built"]
    pub fn with_hedge(mut self, config: HedgeConfig) -> Self {
        self.layers.push(Arc::new(HedgeLayer::new(config)));
        self
    }

    /// Add custom layer
    #[must_use = "builder methods must be chained or built"]
    pub fn with_layer(mut self, layer: Arc<dyn ResilienceLayer<T> + Send + Sync>) -> Self {
        self.layers.push(layer);
        self
    }

    /// Build the composed stack, emitting warnings for suboptimal layer ordering.
    ///
    /// Layer order recommendation (outermost → innermost):
    /// `timeout → retry → circuit_breaker → bulkhead → rate_limiter → hedge → fallback`
    ///
    /// Layers are applied in the order they were added: the first added layer is
    /// outermost (executes first); the last added is innermost (closest to the operation).
    #[must_use]
    pub fn build(self) -> Arc<dyn LayerStack<T> + Send + Sync> {
        self.validate_layer_order();

        let mut stack: Arc<dyn LayerStack<T> + Send + Sync> = Arc::new(TerminalLayer);

        for layer in self.layers.into_iter().rev() {
            stack = Arc::new(ComposedStack { layer, next: stack });
        }

        stack
    }

    /// Warn via tracing if the layer order is likely suboptimal.
    fn validate_layer_order(&self) {
        let names: Vec<&str> = self.layers.iter().map(|l| l.name()).collect();

        // retry should be INSIDE (after) circuit_breaker, not outside.
        // If retry is at index i and circuit_breaker at index j with i < j,
        // retry is outermost — each retry attempt re-checks the circuit breaker,
        // which is correct. The wrong pattern is circuit_breaker outside retry
        // (all retries share one circuit check, none of the retries affect the CB).
        // Actually: CB outside retry = CB wraps the whole retry loop → correct.
        // CB inside retry = each attempt independently checks CB → also correct.
        // What IS wrong: timeout inside retry (each attempt gets the full timeout).
        // Flag: timeout is inner (higher index) than retry.
        let retry_pos = names.iter().position(|&n| n == "retry");
        let timeout_pos = names.iter().position(|&n| n == "timeout");

        if let (Some(r), Some(t)) = (retry_pos, timeout_pos) {
            if t > r {
                // timeout is inner to retry: each attempt resets the timeout — likely intended,
                // but log so the user is aware.
                tracing::debug!(
                    "Layer order: timeout is inside retry (each attempt gets its own timeout). \
                     To apply a single deadline across all attempts, place timeout before retry."
                );
            }
        }

        // fallback should typically be outermost so it catches all upstream errors.
        let fallback_pos = names.iter().position(|&n| n == "fallback");
        if let Some(f) = fallback_pos {
            if f != 0 && names.len() > 1 {
                tracing::debug!(
                    "Layer order: fallback is not the outermost layer. \
                     Errors from layers added before it will not be caught by the fallback."
                );
            }
        }
    }
}

/// Convenience type for a complete resilience chain
pub type ResilienceChain<T> = Arc<dyn LayerStack<T> + Send + Sync>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn create_chain<T: Send + Sync + 'static>() -> LayerBuilder<T> {
        LayerBuilder::new()
    }

    async fn execute_with_chain<T, Op>(
        chain: ResilienceChain<T>,
        operation: Op,
    ) -> ResilienceResult<T>
    where
        Op: RetryableOperation<T> + Send + Sync + 'static,
    {
        let boxed_op = BoxedOperation::new(operation);
        chain.execute(&boxed_op).await
    }

    async fn execute_with_chain_and_cancellation<T, Op>(
        chain: ResilienceChain<T>,
        operation: Op,
        cancellation: &CancellationContext,
    ) -> ResilienceResult<T>
    where
        Op: RetryableOperation<T> + Send + Sync + 'static,
    {
        let boxed_op = BoxedOperation::new(operation);
        chain
            .execute_with_cancellation(&boxed_op, Some(cancellation))
            .await
    }

    #[tokio::test]
    async fn test_layer_composition() {
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        let operation = move || {
            let counter = counter_clone.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok::<u32, ResilienceError>(42)
            }
        };

        let chain = create_chain()
            .with_timeout(Duration::from_secs(5))
            .with_retry_fixed(2, Duration::from_millis(10))
            .build();

        let result = execute_with_chain(chain, operation).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    #[expect(clippy::excessive_nesting)]
    async fn test_retry_with_timeout() {
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        let operation = move || {
            let counter = counter_clone.clone();
            async move {
                let count = counter.fetch_add(1, Ordering::SeqCst);
                if count < 2 {
                    Err(ResilienceError::Custom {
                        message: "Simulated failure".to_string(),
                        retryable: true,
                        source: None,
                    })
                } else {
                    Ok::<u32, ResilienceError>(100)
                }
            }
        };

        let chain = create_chain()
            .with_timeout(Duration::from_secs(5))
            .with_retry_fixed(5, Duration::from_millis(10))
            .build();

        let result = execute_with_chain(chain, operation).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 100);
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_retry_sleep_is_interruptible_by_cancellation() {
        let cancellation = CancellationContext::with_reason("retry cancelled");
        let cancellation_clone = cancellation.clone();

        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(15)).await;
            cancellation_clone.cancel();
        });

        let chain = create_chain()
            .with_retry_fixed(5, Duration::from_millis(500))
            .build();

        let result = execute_with_chain_and_cancellation(
            chain,
            || async {
                Err::<u32, ResilienceError>(ResilienceError::Custom {
                    message: "transient".to_string(),
                    retryable: true,
                    source: None,
                })
            },
            &cancellation,
        )
        .await;

        assert!(matches!(result, Err(ResilienceError::Cancelled { .. })));
    }
}

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

use std::sync::Arc;
use std::time::Duration;

use crate::{
    ResilienceError, ResilienceResult,
    manager::RetryableOperation,
    patterns::{bulkhead::Bulkhead, circuit_breaker::CircuitBreaker, timeout::timeout},
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

#[async_trait::async_trait]
impl<T> RetryableOperation<T> for BoxedOperation<T> {
    async fn execute(&self) -> ResilienceResult<T> {
        self.operation.execute().await
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
#[async_trait::async_trait]
pub trait ResilienceLayer<T>: Send + Sync {
    /// Apply this layer to an operation
    async fn apply(
        &self,
        operation: BoxedOperation<T>,
        next: Arc<dyn LayerStack<T> + Send + Sync>,
    ) -> ResilienceResult<T>;

    /// Get layer name for debugging
    fn name(&self) -> &'static str;
}

/// Stack of layers that can be applied
#[async_trait::async_trait]
pub trait LayerStack<T>: Send + Sync {
    /// Execute the operation with remaining layers
    async fn execute(&self, operation: BoxedOperation<T>) -> ResilienceResult<T>;
}

// =============================================================================
// LAYER IMPLEMENTATIONS
// =============================================================================

/// Terminal layer that just executes the operation
pub(crate) struct TerminalLayer;

#[async_trait::async_trait]
impl<T: Send + 'static> LayerStack<T> for TerminalLayer {
    async fn execute(&self, operation: BoxedOperation<T>) -> ResilienceResult<T> {
        operation.execute().await
    }
}

/// A composed stack of resilience layers
pub(crate) struct ComposedStack<T> {
    layer: Arc<dyn ResilienceLayer<T> + Send + Sync>,
    next: Arc<dyn LayerStack<T> + Send + Sync>,
}

#[async_trait::async_trait]
impl<T: Send + 'static> LayerStack<T> for ComposedStack<T> {
    async fn execute(&self, operation: BoxedOperation<T>) -> ResilienceResult<T> {
        self.layer.apply(operation, self.next.clone()).await
    }
}

/// Timeout layer
pub(crate) struct TimeoutLayer {
    duration: Duration,
}

impl TimeoutLayer {
    pub(crate) fn new(duration: Duration) -> Self {
        Self { duration }
    }
}

#[async_trait::async_trait]
impl<T: Send + 'static> ResilienceLayer<T> for TimeoutLayer {
    async fn apply(
        &self,
        operation: BoxedOperation<T>,
        next: Arc<dyn LayerStack<T> + Send + Sync>,
    ) -> ResilienceResult<T> {
        match timeout(self.duration, next.execute(operation)).await {
            Ok(result) => result,
            Err(_) => Err(ResilienceError::Timeout {
                duration: self.duration,
                context: Some("Layer timeout".to_string()),
            }),
        }
    }

    fn name(&self) -> &'static str {
        "timeout"
    }
}

/// Retry layer using `RetryPolicyConfig`
pub(crate) struct RetryLayer {
    config: RetryPolicyConfig,
}

impl RetryLayer {
    pub(crate) fn new(config: RetryPolicyConfig) -> Self {
        Self { config }
    }
}

#[async_trait::async_trait]
impl<T: Send + 'static> ResilienceLayer<T> for RetryLayer {
    async fn apply(
        &self,
        operation: BoxedOperation<T>,
        next: Arc<dyn LayerStack<T> + Send + Sync>,
    ) -> ResilienceResult<T> {
        let mut last_error = None;

        for attempt in 0..self.config.max_attempts {
            let op_clone = operation.clone();
            let result = next.execute(op_clone).await;

            match result {
                Ok(value) => return Ok(value),
                Err(e) if attempt + 1 == self.config.max_attempts => {
                    last_error = Some(e);
                    break;
                }
                Err(e) if e.is_retryable() => {
                    last_error = Some(e);

                    if let Some(delay) = self.config.delay_for_attempt(attempt) {
                        tokio::time::sleep(delay).await;
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
    }

    fn name(&self) -> &'static str {
        "retry"
    }
}

/// Circuit breaker layer
pub(crate) struct CircuitBreakerLayer {
    circuit_breaker: Arc<CircuitBreaker>,
}

impl CircuitBreakerLayer {
    pub(crate) fn new(circuit_breaker: Arc<CircuitBreaker>) -> Self {
        Self { circuit_breaker }
    }
}

#[async_trait::async_trait]
impl<T: Send + 'static> ResilienceLayer<T> for CircuitBreakerLayer {
    async fn apply(
        &self,
        operation: BoxedOperation<T>,
        next: Arc<dyn LayerStack<T> + Send + Sync>,
    ) -> ResilienceResult<T> {
        self.circuit_breaker.can_execute().await?;

        let result = next.execute(operation).await;

        match &result {
            Ok(_) => self.circuit_breaker.record_success().await,
            Err(_e) => self.circuit_breaker.record_failure().await,
        }

        result
    }

    fn name(&self) -> &'static str {
        "circuit_breaker"
    }
}

/// Bulkhead layer
pub(crate) struct BulkheadLayer {
    bulkhead: Arc<Bulkhead>,
}

impl BulkheadLayer {
    pub(crate) fn new(bulkhead: Arc<Bulkhead>) -> Self {
        Self { bulkhead }
    }
}

#[async_trait::async_trait]
impl<T: Send + 'static> ResilienceLayer<T> for BulkheadLayer {
    async fn apply(
        &self,
        operation: BoxedOperation<T>,
        next: Arc<dyn LayerStack<T> + Send + Sync>,
    ) -> ResilienceResult<T> {
        let _permit = self.bulkhead.acquire().await?;
        next.execute(operation).await
    }

    fn name(&self) -> &'static str {
        "bulkhead"
    }
}

// =============================================================================
// LAYER BUILDER
// =============================================================================

/// Builder for composing resilience layers
pub struct LayerBuilder<T> {
    layers: Vec<Arc<dyn ResilienceLayer<T> + Send + Sync>>,
}

impl<T: Send + 'static> Default for LayerBuilder<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Send + 'static> LayerBuilder<T> {
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

    /// Add custom layer
    #[must_use = "builder methods must be chained or built"]
    pub fn with_layer(mut self, layer: Arc<dyn ResilienceLayer<T> + Send + Sync>) -> Self {
        self.layers.push(layer);
        self
    }

    /// Build the composed stack
    #[must_use]
    pub fn build(self) -> Arc<dyn LayerStack<T> + Send + Sync> {
        let mut stack: Arc<dyn LayerStack<T> + Send + Sync> = Arc::new(TerminalLayer);

        for layer in self.layers.into_iter().rev() {
            stack = Arc::new(ComposedStack { layer, next: stack });
        }

        stack
    }
}

/// Convenience type for a complete resilience chain
pub type ResilienceChain<T> = Arc<dyn LayerStack<T> + Send + Sync>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn create_chain<T: Send + 'static>() -> LayerBuilder<T> {
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
        chain.execute(boxed_op).await
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
}

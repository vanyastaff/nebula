//! # Nebula Resilience
//! 
//! Resilience patterns for the Nebula workflow engine, providing robust error handling,
//! retry mechanisms, circuit breakers, and bulkhead patterns.
//! 
//! ## Key Features
//! 
//! - **Timeout Management**: Configurable timeouts for all I/O operations
//! - **Retry Strategies**: Exponential backoff with jitter and circuit breaker integration
//! - **Circuit Breakers**: Automatic failure detection and recovery
//! - **Bulkheads**: Resource isolation and parallelism limits
//! - **Resilience Policies**: Configurable resilience strategies per operation type
//! 
//! ## Usage
//! 
//! ```rust
//! use nebula_resilience::{
//!     timeout, retry, circuit_breaker, bulkhead,
//!     ResiliencePolicy, ResilienceBuilder
//! };
//! 
//! // Simple timeout wrapper
//! let result = timeout(Duration::from_secs(30), async_operation()).await;
//! 
//! // With retry and circuit breaker
//! let policy = ResiliencePolicy::default()
//!     .with_timeout(Duration::from_secs(10))
//!     .with_retry(3, Duration::from_secs(1))
//!     .with_circuit_breaker(5, Duration::from_secs(60));
//! 
//! let result = policy.execute(async_operation()).await;
//! 
//! // Bulkhead for resource isolation
//! let bulkhead = bulkhead::Bulkhead::new(10);
//! let result = bulkhead.execute(async_operation()).await;
//! ```

pub mod timeout;
pub mod retry;
pub mod circuit_breaker;
pub mod bulkhead;
pub mod policy;
pub mod error;

// Re-export main types
pub use timeout::timeout;
pub use retry::{RetryStrategy, Retryable};
pub use circuit_breaker::{CircuitBreaker, CircuitState};
pub use bulkhead::Bulkhead;
pub use policy::{ResiliencePolicy, ResilienceBuilder};
pub use error::ResilienceError;

/// Common prelude for resilience patterns
pub mod prelude {
    pub use super::{
        timeout, RetryStrategy, Retryable, CircuitBreaker, CircuitState,
        Bulkhead, ResiliencePolicy, ResilienceBuilder, ResilienceError,
    };
}

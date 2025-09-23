//! Resilience patterns implementations

// Basic patterns
pub mod bulkhead;
pub mod circuit_breaker;
pub mod retry;
pub mod timeout;
pub mod fallback;
pub mod hedge;

// Rate limiting
pub mod rate_limiter;
mod token_bucket;
mod leaky_bucket;
mod sliding_window;
mod adaptive_limiter;

// Re-exports
pub use bulkhead::{Bulkhead, BulkheadConfig};
pub use circuit_breaker::{CircuitBreaker, CircuitBreakerConfig, CircuitState};
pub use retry::{retry, retry_with_operation, RetryStrategy, RetryBuilder};
pub use timeout::{timeout, timeout_with_original_error};
pub use fallback::{FallbackStrategy, ValueFallback, FunctionFallback, CacheFallback};
pub use hedge::{HedgeExecutor, HedgeConfig, AdaptiveHedgeExecutor};

// Rate limiter exports
pub use rate_limiter::{RateLimiter, RateLimiterFactory};
pub use token_bucket::TokenBucket;
pub use leaky_bucket::LeakyBucket;
pub use sliding_window::SlidingWindow;
pub use adaptive_limiter::AdaptiveRateLimiter;
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

// Re-exports
pub use bulkhead::{Bulkhead, BulkheadConfig};
pub use circuit_breaker::{CircuitBreaker, CircuitBreakerConfig, CircuitState};
pub use retry::{retry, RetryStrategy};
pub use timeout::{timeout, timeout_with_original_error};
pub use fallback::{FallbackStrategy, ValueFallback, FunctionFallback, CacheFallback, AnyStringFallbackStrategy};
pub use hedge::{HedgeExecutor, HedgeConfig};

// Rate limiter exports
pub use rate_limiter::{RateLimiter, AnyRateLimiter, TokenBucket, LeakyBucket, SlidingWindow, AdaptiveRateLimiter};
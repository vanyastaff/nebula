//! Resilience patterns implementations

// Basic patterns
pub mod bulkhead;
pub mod circuit_breaker;
pub mod fallback;
pub mod hedge;
pub mod retry;
pub mod timeout;

// Rate limiting
pub mod rate_limiter;

// Re-exports
pub use bulkhead::{Bulkhead, BulkheadConfig};
pub use circuit_breaker::{CircuitBreaker, CircuitBreakerConfig, CircuitState};
pub use fallback::{
    AnyStringFallbackStrategy, CacheFallback, FallbackStrategy, FunctionFallback, ValueFallback,
};
pub use hedge::{HedgeConfig, HedgeExecutor};
pub use retry::{RetryStrategy, retry};
pub use timeout::{timeout, timeout_with_original_error};

// Rate limiter exports
pub use rate_limiter::{
    AdaptiveRateLimiter, AnyRateLimiter, GovernorRateLimiter, LeakyBucket, RateLimiter,
    SlidingWindow, TokenBucket,
};


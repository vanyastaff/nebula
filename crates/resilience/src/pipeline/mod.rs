//! `ResiliencePipeline` — compose multiple resilience patterns into a single call chain.
//!
//! Recommended layer order (outermost → innermost):
//! `load_shed → rate_limiter → timeout → retry → circuit_breaker → bulkhead`
//!
//! Layers are applied in the order added: first added = outermost.
//!
//! # Examples
//!
//! ```rust
//! use std::time::Duration;
//!
//! use nebula_resilience::{
//!     ResiliencePipeline,
//!     retry::{BackoffConfig, RetryConfig},
//! };
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let pipeline = ResiliencePipeline::<&str>::builder()
//!     .timeout(Duration::from_secs(2))
//!     .retry(
//!         RetryConfig::new(3)?
//!             .backoff(BackoffConfig::Fixed(Duration::from_millis(10))),
//!     )
//!     .build();
//!
//! // The operation returns `Ok` unconditionally, so this cannot fail.
//! let value = pipeline
//!     .call(|| Box::pin(async { Ok::<_, &str>(42u32) }))
//!     .await
//!     .expect("the operation succeeds by construction");
//! assert_eq!(value, 42);
//! # Ok(())
//! # }
//! ```

use std::{future::Future, pin::Pin, sync::Arc, time::Duration};

use crate::{
    CallError, CallErrorKind, bulkhead::Bulkhead, circuit_breaker::CircuitBreaker,
    retry::RetryConfig,
};

/// Async predicate for rate limiting — returns `Ok(())` or `Err(CallError::RateLimited)`.
pub type RateLimitCheck =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = Result<(), CallError<()>>> + Send>> + Send + Sync>;

/// Predicate for load shedding — returns `true` to shed the request.
pub type LoadShedPredicate = Arc<dyn Fn() -> bool + Send + Sync>;

type RetryHintFn<E> = Arc<dyn Fn(&E) -> Option<Duration> + Send + Sync>;

enum Step<E: 'static> {
    Timeout(Duration),
    Retry(Box<RetryConfig<E>>),
    CircuitBreaker(Arc<CircuitBreaker>),
    Bulkhead(Arc<Bulkhead>),
    RateLimiter(RateLimitCheck),
    LoadShed(LoadShedPredicate),
}

/// Final outcome of a pipeline invocation.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[doc(alias = "PipelineResult")]
pub enum PipelineOutcome {
    /// Pipeline returned the primary operation result.
    Success,
    /// Pipeline failed and no fallback recovered it.
    Failure {
        /// Final failure kind.
        error: CallErrorKind,
    },
    /// Fallback recovered the primary failure.
    FallbackSucceeded {
        /// Primary failure kind that was recovered.
        primary_error: CallErrorKind,
    },
    /// Fallback was attempted but failed.
    FallbackFailed {
        /// Primary failure kind that triggered fallback.
        primary_error: CallErrorKind,
        /// Fallback failure kind.
        fallback_error: CallErrorKind,
    },
}

pub mod builder;
pub mod executor;

pub use builder::PipelineBuilder;
pub use executor::ResiliencePipeline;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

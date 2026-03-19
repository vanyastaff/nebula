//! Token bucket rate limiter implementation

use parking_lot::Mutex;
use std::fmt;
use std::future::Future;
use std::time::Instant;

use super::RateLimiter;
use crate::CallError;

#[derive(Debug)]
struct TokenBucketState {
    tokens: f64,
    last_refill: Instant,
}

/// Token bucket rate limiter
///
/// Classic token bucket algorithm with configurable capacity and refill rate.
/// Tokens are added at a constant rate and consumed by operations.
///
/// # Security
///
/// - Maximum capacity limited to 100,000 to prevent memory exhaustion
/// - Refill rate clamped between 0.001 and 10,000 req/sec
///
/// # Examples
///
/// ```
/// use nebula_resilience::patterns::rate_limiter::TokenBucket;
///
/// let limiter = TokenBucket::new(100, 10.0); // 100 capacity, 10 req/sec
/// ```
pub struct TokenBucket {
    /// Maximum tokens in bucket
    capacity: usize,
    /// Mutable runtime state
    state: Mutex<TokenBucketState>,
    /// Token refill rate per second
    refill_rate: f64,
    /// Burst size
    burst_size: usize,
}

// C-DEBUG: All public types implement Debug
impl fmt::Debug for TokenBucket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenBucket")
            .field("capacity", &self.capacity)
            .field("refill_rate", &self.refill_rate)
            .field("burst_size", &self.burst_size)
            .finish_non_exhaustive()
    }
}

impl TokenBucket {
    /// Create new token bucket with validation
    #[must_use]
    pub fn new(capacity: usize, refill_rate: f64) -> Self {
        // Security: prevent creating token buckets with invalid parameters
        let safe_capacity = capacity.min(100_000); // Prevent memory exhaustion
        let safe_refill_rate = refill_rate.clamp(0.001, 10_000.0); // Reasonable limits

        Self {
            capacity: safe_capacity,
            state: Mutex::new(TokenBucketState {
                tokens: safe_capacity as f64,
                last_refill: Instant::now(),
            }),
            refill_rate: safe_refill_rate,
            burst_size: safe_capacity,
        }
    }

    /// Set burst size
    #[must_use = "builder methods must be chained or built"]
    pub const fn with_burst(mut self, burst_size: usize) -> Self {
        self.burst_size = burst_size;
        self
    }
}

impl RateLimiter for TokenBucket {
    async fn acquire(&self) -> Result<(), CallError<()>> {
        let mut state = self.state.lock();

        let now = Instant::now();
        let elapsed = now.duration_since(state.last_refill).as_secs_f64();
        let tokens_to_add = elapsed * self.refill_rate;
        state.tokens = (state.tokens + tokens_to_add).min(self.burst_size as f64);
        state.last_refill = now;

        if state.tokens >= 1.0 {
            state.tokens -= 1.0;
            drop(state);
            Ok(())
        } else {
            drop(state);
            Err(CallError::RateLimited)
        }
    }

    async fn execute<T, E, F, Fut>(&self, operation: F) -> Result<T, CallError<E>>
    where
        F: FnOnce() -> Fut + Send,
        Fut: Future<Output = Result<T, E>> + Send,
        T: Send,
    {
        self.acquire().await.map_err(|_| CallError::RateLimited)?;
        operation().await.map_err(CallError::Operation)
    }

    async fn current_rate(&self) -> f64 {
        let state = self.state.lock();
        let tokens = state.tokens;
        drop(state);
        tokens
    }

    async fn reset(&self) {
        let mut state = self.state.lock();
        state.tokens = self.capacity as f64;
        state.last_refill = Instant::now();
    }
}

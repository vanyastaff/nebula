use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Token refresh policy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshPolicy {
    /// Refresh when this percentage of TTL has elapsed (0.0 - 1.0)
    pub threshold: f32,

    /// Safety margin before expiry
    pub skew: Duration,

    /// Maximum age for eternal tokens
    pub max_age: Option<Duration>,

    /// Maximum number of refresh retries
    pub max_retries: u32,

    /// Base backoff duration
    pub backoff_base: Duration,

    /// Backoff multiplication factor
    pub backoff_factor: f64,

    /// Maximum backoff duration
    pub backoff_max: Duration,

    /// Negative cache TTL
    pub negative_cache_ttl: Duration,
}

impl Default for RefreshPolicy {
    fn default() -> Self {
        Self {
            threshold: 0.8,
            skew: Duration::from_secs(45),
            max_age: Some(Duration::from_secs(3600)),
            max_retries: 3,
            backoff_base: Duration::from_millis(100),
            backoff_factor: 2.0,
            backoff_max: Duration::from_secs(10),
            negative_cache_ttl: Duration::from_secs(60),
        }
    }
}

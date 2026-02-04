//! Configuration types for credential manager

use crate::utils::RetryPolicy;
use std::time::Duration;

/// Configuration for credential manager
#[derive(Debug, Clone)]
pub struct ManagerConfig {
    /// Cache configuration (if caching enabled)
    pub cache_config: Option<CacheConfig>,

    /// Default scope for operations without explicit scope (TODO: add ScopeId in Phase 4)
    pub default_scope: Option<String>,

    /// Maximum concurrent operations in batch
    pub batch_concurrency: usize,

    /// Retry policy for storage operations
    pub retry_policy: RetryPolicy,
}

impl Default for ManagerConfig {
    fn default() -> Self {
        Self {
            cache_config: None, // Caching disabled by default
            default_scope: None,
            batch_concurrency: 10,
            retry_policy: RetryPolicy::default(),
        }
    }
}

/// Configuration for credential caching
#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// Enable/disable caching
    pub enabled: bool,

    /// Time-to-live for cache entries
    pub ttl: Option<Duration>,

    /// Time-to-idle (evict if not accessed)
    pub idle_timeout: Option<Duration>,

    /// Maximum number of cached credentials
    pub max_capacity: usize,

    /// Cache eviction strategy
    pub eviction_strategy: EvictionStrategy,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            ttl: Some(Duration::from_secs(300)), // 5 minutes
            idle_timeout: None,
            max_capacity: 1000,
            eviction_strategy: EvictionStrategy::Lru,
        }
    }
}

/// Cache eviction strategy
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvictionStrategy {
    /// Least-recently-used (default)
    Lru,
    /// Least-frequently-used
    Lfu,
}

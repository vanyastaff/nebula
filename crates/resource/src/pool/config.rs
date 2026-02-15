//! Pool configuration types

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Configuration for resource pooling
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PoolConfig {
    /// Minimum number of resources in the pool
    pub min_size: usize,
    /// Maximum number of resources in the pool
    pub max_size: usize,
    /// Timeout for acquiring a resource from the pool
    pub acquire_timeout: std::time::Duration,
    /// Time after which idle resources are removed
    pub idle_timeout: std::time::Duration,
    /// Maximum lifetime of a resource
    pub max_lifetime: std::time::Duration,
    /// Interval for validation/health checks
    pub validation_interval: std::time::Duration,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            min_size: 1,
            max_size: 10,
            acquire_timeout: std::time::Duration::from_secs(30),
            idle_timeout: std::time::Duration::from_secs(600),
            max_lifetime: std::time::Duration::from_secs(3600),
            validation_interval: std::time::Duration::from_secs(30),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pool_config_default() {
        let config = PoolConfig::default();
        assert_eq!(config.min_size, 1);
        assert_eq!(config.max_size, 10);
        assert_eq!(config.acquire_timeout, std::time::Duration::from_secs(30));
    }
}

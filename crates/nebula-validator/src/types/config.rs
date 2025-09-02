//! Configuration types for validation

use std::collections::HashMap;
use std::time::Duration;
use serde::{Serialize, Deserialize};

/// Main configuration for validation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationConfig {
    /// Cache configuration
    pub cache: CacheConfig,
    /// Performance configuration
    pub performance: PerformanceConfig,
    /// Retry configuration
    pub retry: RetryConfig,
    /// Timeout configuration
    pub timeout: TimeoutConfig,
    /// Feature flags
    pub features: HashMap<String, bool>,
    /// Custom configuration values
    pub custom: HashMap<String, serde_json::Value>,
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            cache: CacheConfig::default(),
            performance: PerformanceConfig::default(),
            retry: RetryConfig::default(),
            timeout: TimeoutConfig::default(),
            features: Self::default_features(),
            custom: HashMap::new(),
        }
    }
}

impl ValidationConfig {
    /// Create a new configuration
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Create a production configuration
    pub fn production() -> Self {
        Self {
            cache: CacheConfig::production(),
            performance: PerformanceConfig::production(),
            retry: RetryConfig::production(),
            timeout: TimeoutConfig::production(),
            features: Self::default_features(),
            custom: HashMap::new(),
        }
    }
    
    /// Create a development configuration
    pub fn development() -> Self {
        Self {
            cache: CacheConfig::development(),
            performance: PerformanceConfig::development(),
            retry: RetryConfig::disabled(),
            timeout: TimeoutConfig::development(),
            features: Self::all_features(),
            custom: HashMap::new(),
        }
    }
    
    /// Get default features
    fn default_features() -> HashMap<String, bool> {
        let mut features = HashMap::new();
        features.insert("caching".to_string(), true);
        features.insert("async".to_string(), true);
        features.insert("metrics".to_string(), true);
        features.insert("tracing".to_string(), false);
        features
    }
    
    /// Get all features enabled
    fn all_features() -> HashMap<String, bool> {
        let mut features = HashMap::new();
        features.insert("caching".to_string(), true);
        features.insert("async".to_string(), true);
        features.insert("metrics".to_string(), true);
        features.insert("tracing".to_string(), true);
        features.insert("debug".to_string(), true);
        features
    }
    
    /// Check if a feature is enabled
    pub fn is_feature_enabled(&self, feature: &str) -> bool {
        self.features.get(feature).copied().unwrap_or(false)
    }
}

/// Cache configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    /// Whether caching is enabled
    pub enabled: bool,
    /// Maximum number of cache entries
    pub max_entries: usize,
    /// Default TTL for cache entries
    pub default_ttl: Duration,
    /// Eviction policy
    pub eviction_policy: EvictionPolicy,
    /// Whether to cache failures
    pub cache_failures: bool,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_entries: 1000,
            default_ttl: Duration::from_secs(300), // 5 minutes
            eviction_policy: EvictionPolicy::LRU,
            cache_failures: false,
        }
    }
}

impl CacheConfig {
    /// Production configuration
    pub fn production() -> Self {
        Self {
            enabled: true,
            max_entries: 10000,
            default_ttl: Duration::from_secs(3600), // 1 hour
            eviction_policy: EvictionPolicy::LRU,
            cache_failures: false,
        }
    }
    
    /// Development configuration
    pub fn development() -> Self {
        Self {
            enabled: false, // Disable caching in development
            max_entries: 100,
            default_ttl: Duration::from_secs(60),
            eviction_policy: EvictionPolicy::LRU,
            cache_failures: true,
        }
    }
}

/// Cache eviction policies
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvictionPolicy {
    /// Least Recently Used
    LRU,
    /// Least Frequently Used
    LFU,
    /// First In First Out
    FIFO,
    /// Time To Live only
    TTL,
}

/// Performance configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceConfig {
    /// Maximum concurrent validations
    pub max_concurrency: usize,
    /// Maximum recursion depth
    pub max_recursion_depth: usize,
    /// Performance budget (milliseconds)
    pub budget_ms: u64,
    /// Whether to collect metrics
    pub collect_metrics: bool,
    /// Batch size for batch operations
    pub batch_size: usize,
}

impl Default for PerformanceConfig {
    fn default() -> Self {
        Self {
            max_concurrency: 10,
            max_recursion_depth: 10,
            budget_ms: 1000,
            collect_metrics: true,
            batch_size: 100,
        }
    }
}

impl PerformanceConfig {
    /// Production configuration
    pub fn production() -> Self {
        Self {
            max_concurrency: 100,
            max_recursion_depth: 20,
            budget_ms: 5000,
            collect_metrics: true,
            batch_size: 1000,
        }
    }
    
    /// Development configuration
    pub fn development() -> Self {
        Self {
            max_concurrency: 5,
            max_recursion_depth: 5,
            budget_ms: 10000,
            collect_metrics: true,
            batch_size: 10,
        }
    }
}

/// Retry configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    /// Whether retries are enabled
    pub enabled: bool,
    /// Maximum number of retry attempts
    pub max_attempts: usize,
    /// Initial retry delay
    pub initial_delay: Duration,
    /// Maximum retry delay
    pub max_delay: Duration,
    /// Backoff multiplier
    pub backoff_multiplier: f64,
    /// Whether to use jitter
    pub use_jitter: bool,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_attempts: 3,
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(10),
            backoff_multiplier: 2.0,
            use_jitter: true,
        }
    }
}

impl RetryConfig {
    /// Production configuration
    pub fn production() -> Self {
        Self {
            enabled: true,
            max_attempts: 5,
            initial_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(30),
            backoff_multiplier: 2.0,
            use_jitter: true,
        }
    }
    
    /// Disabled configuration
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Default::default()
        }
    }
}

/// Timeout configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeoutConfig {
    /// Default timeout for validations
    pub default_timeout: Duration,
    /// Timeout for async operations
    pub async_timeout: Duration,
    /// Timeout for network operations
    pub network_timeout: Duration,
    /// Whether to use adaptive timeouts
    pub adaptive: bool,
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            default_timeout: Duration::from_secs(5),
            async_timeout: Duration::from_secs(10),
            network_timeout: Duration::from_secs(30),
            adaptive: false,
        }
    }
}

impl TimeoutConfig {
    /// Production configuration
    pub fn production() -> Self {
        Self {
            default_timeout: Duration::from_secs(10),
            async_timeout: Duration::from_secs(30),
            network_timeout: Duration::from_secs(60),
            adaptive: true,
        }
    }
    
    /// Development configuration
    pub fn development() -> Self {
        Self {
            default_timeout: Duration::from_secs(60),
            async_timeout: Duration::from_secs(120),
            network_timeout: Duration::from_secs(300),
            adaptive: false,
        }
    }
}

/// Configuration builder
pub struct ConfigBuilder {
    config: ValidationConfig,
}

impl ConfigBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self {
            config: ValidationConfig::default(),
        }
    }
    
    /// Set cache configuration
    pub fn cache(mut self, cache: CacheConfig) -> Self {
        self.config.cache = cache;
        self
    }
    
    /// Set performance configuration
    pub fn performance(mut self, performance: PerformanceConfig) -> Self {
        self.config.performance = performance;
        self
    }
    
    /// Set retry configuration
    pub fn retry(mut self, retry: RetryConfig) -> Self {
        self.config.retry = retry;
        self
    }
    
    /// Set timeout configuration
    pub fn timeout(mut self, timeout: TimeoutConfig) -> Self {
        self.config.timeout = timeout;
        self
    }
    
    /// Enable a feature
    pub fn enable_feature(mut self, feature: impl Into<String>) -> Self {
        self.config.features.insert(feature.into(), true);
        self
    }
    
    /// Disable a feature
    pub fn disable_feature(mut self, feature: impl Into<String>) -> Self {
        self.config.features.insert(feature.into(), false);
        self
    }
    
    /// Set custom configuration
    pub fn custom(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.config.custom.insert(key.into(), value);
        self
    }
    
    /// Build the configuration
    pub fn build(self) -> ValidationConfig {
        self.config
    }
}

impl Default for ConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}
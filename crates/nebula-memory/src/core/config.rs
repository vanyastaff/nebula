//! Configuration for nebula-memory
//!
//! This module provides comprehensive configuration management for all memory
//! components in the nebula-memory crate, following nebula patterns.

use core::fmt;
use core::time::Duration;

use super::error::{MemoryError, MemoryResult};

#[cfg(feature = "logging")]
use nebula_log::{info, debug, warn};

// ============================================================================
// Core Configuration Types
// ============================================================================

/// Global memory system configuration
#[derive(Debug, Clone)]
pub struct MemoryConfig {
    /// Allocator configuration
    pub allocator: AllocatorConfig,
    /// Pool configuration
    #[cfg(feature = "pool")]
    pub pool: PoolConfig,
    /// Arena configuration
    #[cfg(feature = "arena")]
    pub arena: ArenaConfig,
    /// Cache configuration
    #[cfg(feature = "cache")]
    pub cache: CacheConfig,
    /// Budget configuration
    #[cfg(feature = "budget")]
    pub budget: BudgetConfig,
    /// Statistics configuration
    #[cfg(feature = "stats")]
    pub stats: StatsConfig,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            allocator: AllocatorConfig::default(),
            #[cfg(feature = "pool")]
            pool: PoolConfig::default(),
            #[cfg(feature = "arena")]
            arena: ArenaConfig::default(),
            #[cfg(feature = "cache")]
            cache: CacheConfig::default(),
            #[cfg(feature = "budget")]
            budget: BudgetConfig::default(),
            #[cfg(feature = "stats")]
            stats: StatsConfig::default(),
        }
    }
}

impl MemoryConfig {
    /// Create a new configuration with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Production configuration - optimized for maximum performance
    pub fn production() -> Self {
        let mut config = Self::default();
        config.allocator = AllocatorConfig::production();

        #[cfg(feature = "pool")]
        {
            config.pool = PoolConfig::production();
        }

        #[cfg(feature = "arena")]
        {
            config.arena = ArenaConfig::production();
        }

        #[cfg(feature = "cache")]
        {
            config.cache = CacheConfig::production();
        }

        config
    }

    /// Debug configuration - optimized for debugging and error detection
    pub fn debug() -> Self {
        let mut config = Self::default();
        config.allocator = AllocatorConfig::debug();

        #[cfg(feature = "pool")]
        {
            config.pool = PoolConfig::debug();
        }

        #[cfg(feature = "arena")]
        {
            config.arena = ArenaConfig::debug();
        }

        #[cfg(feature = "cache")]
        {
            config.cache = CacheConfig::debug();
        }

        config
    }

    /// Create a configuration optimized for high-performance scenarios
    pub fn high_performance() -> Self {
        Self::production()
    }

    /// Create a configuration optimized for low memory usage
    pub fn low_memory() -> Self {
        let mut config = Self::default();
        config.allocator = AllocatorConfig::low_memory();

        #[cfg(feature = "pool")]
        {
            config.pool = PoolConfig::low_memory();
        }

        #[cfg(feature = "arena")]
        {
            config.arena = ArenaConfig::low_memory();
        }

        #[cfg(feature = "cache")]
        {
            config.cache = CacheConfig::low_memory();
        }

        config
    }

    /// Validate the configuration
    pub fn validate(&self) -> MemoryResult<()> {
        #[cfg(feature = "logging")]
        {
            debug!("Validating memory configuration");
        }

        self.allocator.validate()
            .map_err(|e| MemoryError::invalid_config(format!("allocator: {}", e)))?;

        #[cfg(feature = "pool")]
        {
            self.pool.validate()
                .map_err(|e| MemoryError::invalid_config(format!("pool: {}", e)))?;
        }

        #[cfg(feature = "arena")]
        {
            self.arena.validate()
                .map_err(|e| MemoryError::invalid_config(format!("arena: {}", e)))?;
        }

        #[cfg(feature = "cache")]
        {
            self.cache.validate()
                .map_err(|e| MemoryError::invalid_config(format!("cache: {}", e)))?;
        }

        #[cfg(feature = "budget")]
        {
            self.budget.validate()
                .map_err(|e| MemoryError::invalid_config(format!("budget: {}", e)))?;
        }

        #[cfg(feature = "logging")]
        {
            info!("Memory configuration validation successful");
        }

        Ok(())
    }
}

// ============================================================================
// Allocator Configuration
// ============================================================================

/// Configuration for memory allocators
#[derive(Debug, Clone)]
pub struct AllocatorConfig {
    /// Default allocator type to use
    pub default_allocator: AllocatorType,
    /// Maximum allocation size
    pub max_allocation_size: usize,
    /// Enable allocation tracking
    pub enable_tracking: bool,
    /// Enable safety checks (may impact performance)
    pub enable_safety_checks: bool,
    /// Memory alignment preference
    pub alignment_preference: AlignmentPreference,
}

impl Default for AllocatorConfig {
    fn default() -> Self {
        Self {
            default_allocator: AllocatorType::System,
            max_allocation_size: 1 << 30, // 1GB
            enable_tracking: cfg!(debug_assertions),
            enable_safety_checks: cfg!(debug_assertions),
            alignment_preference: AlignmentPreference::Natural,
        }
    }
}

impl AllocatorConfig {
    /// Production configuration - optimized for maximum performance
    pub fn production() -> Self {
        Self {
            default_allocator: AllocatorType::Bump,
            max_allocation_size: 1 << 32, // 4GB
            enable_tracking: false,
            enable_safety_checks: false,
            alignment_preference: AlignmentPreference::CacheLine,
        }
    }

    /// Debug configuration - optimized for debugging and error detection
    pub fn debug() -> Self {
        Self {
            default_allocator: AllocatorType::Tracked,
            max_allocation_size: 1 << 30, // 1GB
            enable_tracking: true,
            enable_safety_checks: true,
            alignment_preference: AlignmentPreference::Natural,
        }
    }

    /// Configuration optimized for high performance (alias for production)
    pub fn high_performance() -> Self {
        Self::production()
    }

    /// Configuration optimized for low memory usage
    pub fn low_memory() -> Self {
        Self {
            default_allocator: AllocatorType::System,
            max_allocation_size: 1 << 26, // 64MB
            enable_tracking: true,
            enable_safety_checks: true,
            alignment_preference: AlignmentPreference::Natural,
        }
    }

    /// Validate allocator configuration
    pub fn validate(&self) -> MemoryResult<()> {
        if self.max_allocation_size == 0 {
            return Err(MemoryError::invalid_config("max_allocation_size cannot be zero"));
        }

        if !self.max_allocation_size.is_power_of_two() {
            #[cfg(feature = "logging")]
            {
                warn!("max_allocation_size is not a power of two, this may impact performance");
            }
        }

        Ok(())
    }
}

/// Available allocator types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocatorType {
    /// System allocator (malloc/free)
    System,
    /// Bump allocator for sequential allocation
    Bump,
    /// Stack allocator with markers
    Stack,
    /// Pool allocator for object reuse
    Pool,
    /// Tracked allocator with statistics
    Tracked,
}

impl fmt::Display for AllocatorType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AllocatorType::System => write!(f, "system"),
            AllocatorType::Bump => write!(f, "bump"),
            AllocatorType::Stack => write!(f, "stack"),
            AllocatorType::Pool => write!(f, "pool"),
            AllocatorType::Tracked => write!(f, "tracked"),
        }
    }
}

/// Memory alignment preferences
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignmentPreference {
    /// Use natural alignment for the type
    Natural,
    /// Align to cache line boundaries (typically 64 bytes)
    CacheLine,
    /// Custom alignment value
    Custom(usize),
}

// ============================================================================
// Pool Configuration
// ============================================================================

#[cfg(feature = "pool")]
/// Configuration for object pools
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// Default pool capacity
    pub default_capacity: usize,
    /// Maximum pool capacity
    pub max_capacity: usize,
    /// Enable pool statistics
    pub enable_stats: bool,
    /// Pool growth strategy
    pub growth_strategy: PoolGrowthStrategy,
    /// Pool shrink strategy
    pub shrink_strategy: PoolShrinkStrategy,
    /// Cleanup interval for unused pools
    pub cleanup_interval: Option<Duration>,
}

#[cfg(feature = "pool")]
impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            default_capacity: 32,
            max_capacity: 1024,
            enable_stats: cfg!(feature = "stats"),
            growth_strategy: PoolGrowthStrategy::Double,
            shrink_strategy: PoolShrinkStrategy::Lazy,
            cleanup_interval: Some(Duration::from_secs(60)),
        }
    }
}

#[cfg(feature = "pool")]
impl PoolConfig {
    /// Production configuration - optimized for maximum performance
    pub fn production() -> Self {
        Self {
            default_capacity: 128,
            max_capacity: 4096,
            enable_stats: false,
            growth_strategy: PoolGrowthStrategy::Fixed(256),
            shrink_strategy: PoolShrinkStrategy::Never,
            cleanup_interval: None,
        }
    }

    /// Debug configuration - optimized for debugging and error detection
    pub fn debug() -> Self {
        Self {
            default_capacity: 16,
            max_capacity: 256,
            enable_stats: true,
            growth_strategy: PoolGrowthStrategy::Linear(8),
            shrink_strategy: PoolShrinkStrategy::Lazy,
            cleanup_interval: Some(Duration::from_secs(10)),
        }
    }

    /// Configuration optimized for high performance (alias for production)
    pub fn high_performance() -> Self {
        Self::production()
    }

    /// Configuration optimized for low memory usage
    pub fn low_memory() -> Self {
        Self {
            default_capacity: 8,
            max_capacity: 128,
            enable_stats: true,
            growth_strategy: PoolGrowthStrategy::Linear(4),
            shrink_strategy: PoolShrinkStrategy::Aggressive,
            cleanup_interval: Some(Duration::from_secs(30)),
        }
    }

    /// Validate pool configuration
    pub fn validate(&self) -> MemoryResult<()> {
        if self.default_capacity == 0 {
            return Err(MemoryError::invalid_config("default_capacity cannot be zero"));
        }

        if self.max_capacity < self.default_capacity {
            return Err(MemoryError::invalid_config("max_capacity must be >= default_capacity"));
        }

        Ok(())
    }
}

#[cfg(feature = "pool")]
/// Pool growth strategies
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolGrowthStrategy {
    /// Double the pool size
    Double,
    /// Add a fixed number of objects
    Fixed(usize),
    /// Add a linear increment
    Linear(usize),
}

#[cfg(feature = "pool")]
/// Pool shrink strategies
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolShrinkStrategy {
    /// Never shrink
    Never,
    /// Shrink when idle for a while
    Lazy,
    /// Shrink aggressively
    Aggressive,
}

// ============================================================================
// Arena Configuration
// ============================================================================

#[cfg(feature = "arena")]
/// Configuration for memory arenas
#[derive(Debug, Clone)]
pub struct ArenaConfig {
    /// Default arena size
    pub default_size: usize,
    /// Maximum arena size
    pub max_size: usize,
    /// Enable arena statistics
    pub enable_stats: bool,
    /// Arena growth strategy
    pub growth_strategy: ArenaGrowthStrategy,
    /// Enable compression for large arenas
    pub enable_compression: bool,
}

#[cfg(feature = "arena")]
impl Default for ArenaConfig {
    fn default() -> Self {
        Self {
            default_size: 64 * 1024, // 64KB
            max_size: 16 * 1024 * 1024, // 16MB
            enable_stats: cfg!(feature = "stats"),
            growth_strategy: ArenaGrowthStrategy::Double,
            enable_compression: false,
        }
    }
}

#[cfg(feature = "arena")]
impl ArenaConfig {
    /// Production configuration - optimized for maximum performance
    pub fn production() -> Self {
        Self {
            default_size: 1024 * 1024, // 1MB
            max_size: 256 * 1024 * 1024, // 256MB
            enable_stats: false,
            growth_strategy: ArenaGrowthStrategy::Fixed(2 * 1024 * 1024), // 2MB
            enable_compression: false,
        }
    }

    /// Debug configuration - optimized for debugging and error detection
    pub fn debug() -> Self {
        Self {
            default_size: 64 * 1024, // 64KB
            max_size: 16 * 1024 * 1024, // 16MB
            enable_stats: true,
            growth_strategy: ArenaGrowthStrategy::Double,
            enable_compression: false,
        }
    }

    /// Configuration optimized for high performance (alias for production)
    pub fn high_performance() -> Self {
        Self::production()
    }

    /// Configuration optimized for low memory usage
    pub fn low_memory() -> Self {
        Self {
            default_size: 4 * 1024, // 4KB
            max_size: 1024 * 1024, // 1MB
            enable_stats: true,
            growth_strategy: ArenaGrowthStrategy::Linear(4 * 1024), // 4KB
            enable_compression: true,
        }
    }

    /// Validate arena configuration
    pub fn validate(&self) -> MemoryResult<()> {
        if self.default_size == 0 {
            return Err(MemoryError::invalid_config("default_size cannot be zero"));
        }

        if self.max_size < self.default_size {
            return Err(MemoryError::invalid_config("max_size must be >= default_size"));
        }

        if !self.default_size.is_power_of_two() {
            #[cfg(feature = "logging")]
            {
                warn!("Arena default_size is not a power of two, this may impact performance");
            }
        }

        Ok(())
    }
}

#[cfg(feature = "arena")]
/// Arena growth strategies
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArenaGrowthStrategy {
    /// Double the arena size
    Double,
    /// Add a fixed amount
    Fixed(usize),
    /// Add a linear increment
    Linear(usize),
}

// ============================================================================
// Cache Configuration
// ============================================================================

#[cfg(feature = "cache")]
/// Configuration for caches
#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// Default cache capacity
    pub default_capacity: usize,
    /// Maximum cache capacity
    pub max_capacity: usize,
    /// Cache eviction policy
    pub eviction_policy: EvictionPolicy,
    /// Enable cache statistics
    pub enable_stats: bool,
    /// TTL for cache entries
    pub default_ttl: Option<Duration>,
}

#[cfg(feature = "cache")]
impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            default_capacity: 256,
            max_capacity: 4096,
            eviction_policy: EvictionPolicy::Lru,
            enable_stats: cfg!(feature = "stats"),
            default_ttl: None,
        }
    }
}

#[cfg(feature = "cache")]
impl CacheConfig {
    /// Production configuration - optimized for maximum performance
    pub fn production() -> Self {
        Self {
            default_capacity: 1024,
            max_capacity: 16384,
            eviction_policy: EvictionPolicy::Lfu,
            enable_stats: false,
            default_ttl: None,
        }
    }

    /// Debug configuration - optimized for debugging and error detection
    pub fn debug() -> Self {
        Self {
            default_capacity: 128,
            max_capacity: 1024,
            eviction_policy: EvictionPolicy::Lru,
            enable_stats: true,
            default_ttl: Some(Duration::from_secs(60)), // 1 minute
        }
    }

    /// Configuration optimized for high performance (alias for production)
    pub fn high_performance() -> Self {
        Self::production()
    }

    /// Configuration optimized for low memory usage
    pub fn low_memory() -> Self {
        Self {
            default_capacity: 64,
            max_capacity: 512,
            eviction_policy: EvictionPolicy::Fifo,
            enable_stats: true,
            default_ttl: Some(Duration::from_secs(300)), // 5 minutes
        }
    }

    /// Validate cache configuration
    pub fn validate(&self) -> MemoryResult<()> {
        if self.default_capacity == 0 {
            return Err(MemoryError::invalid_config("default_capacity cannot be zero"));
        }

        if self.max_capacity < self.default_capacity {
            return Err(MemoryError::invalid_config("max_capacity must be >= default_capacity"));
        }

        Ok(())
    }
}

#[cfg(feature = "cache")]
/// Cache eviction policies
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvictionPolicy {
    /// Least Recently Used
    Lru,
    /// Least Frequently Used
    Lfu,
    /// First In, First Out
    Fifo,
    /// Adaptive Replacement Cache
    Arc,
}

// ============================================================================
// Budget Configuration
// ============================================================================

#[cfg(feature = "budget")]
/// Configuration for memory budgets
#[derive(Debug, Clone)]
pub struct BudgetConfig {
    /// Global memory budget in bytes
    pub global_budget: Option<usize>,
    /// Per-operation budget in bytes
    pub operation_budget: Option<usize>,
    /// Enable budget enforcement
    pub enforce_budgets: bool,
    /// Budget check interval
    pub check_interval: Duration,
}

#[cfg(feature = "budget")]
impl Default for BudgetConfig {
    fn default() -> Self {
        Self {
            global_budget: None,
            operation_budget: Some(64 * 1024 * 1024), // 64MB
            enforce_budgets: true,
            check_interval: Duration::from_millis(100),
        }
    }
}

#[cfg(feature = "budget")]
impl BudgetConfig {
    /// Validate budget configuration
    pub fn validate(&self) -> MemoryResult<()> {
        if let (Some(global), Some(operation)) = (self.global_budget, self.operation_budget) {
            if operation > global {
                return Err(MemoryError::invalid_config("operation_budget cannot exceed global_budget"));
            }
        }

        Ok(())
    }
}

// ============================================================================
// Statistics Configuration
// ============================================================================

#[cfg(feature = "stats")]
/// Configuration for statistics collection
#[derive(Debug, Clone)]
pub struct StatsConfig {
    /// Enable detailed statistics
    pub enable_detailed: bool,
    /// Statistics collection interval
    pub collection_interval: Duration,
    /// Maximum history to keep
    pub max_history: usize,
    /// Enable real-time monitoring
    pub enable_realtime: bool,
}

#[cfg(feature = "stats")]
impl Default for StatsConfig {
    fn default() -> Self {
        Self {
            enable_detailed: false,
            collection_interval: Duration::from_secs(1),
            max_history: 1000,
            enable_realtime: false,
        }
    }
}

// ============================================================================
// Configuration Loading and Saving
// ============================================================================

impl MemoryConfig {
    /// Load configuration from environment variables
    #[cfg(feature = "std")]
    pub fn from_env() -> MemoryResult<Self> {
        let mut config = Self::default();

        // Load allocator config from env
        if let Ok(max_size) = std::env::var("NEBULA_MEMORY_MAX_ALLOCATION_SIZE") {
            config.allocator.max_allocation_size = max_size.parse()
                .map_err(|_| MemoryError::invalid_config("Invalid NEBULA_MEMORY_MAX_ALLOCATION_SIZE"))?;
        }

        if let Ok(tracking) = std::env::var("NEBULA_MEMORY_ENABLE_TRACKING") {
            config.allocator.enable_tracking = tracking.parse()
                .map_err(|_| MemoryError::invalid_config("Invalid NEBULA_MEMORY_ENABLE_TRACKING"))?;
        }

        // Add more environment variable parsing as needed

        config.validate()?;
        Ok(config)
    }

    /// Create a builder for the configuration
    pub fn builder() -> MemoryConfigBuilder {
        MemoryConfigBuilder::new()
    }
}

// ============================================================================
// Configuration Builder
// ============================================================================

/// Builder for MemoryConfig
#[derive(Debug, Default)]
pub struct MemoryConfigBuilder {
    config: MemoryConfig,
}

impl MemoryConfigBuilder {
    /// Create a new configuration builder
    pub fn new() -> Self {
        Self {
            config: MemoryConfig::default(),
        }
    }

    /// Set allocator configuration
    pub fn allocator(mut self, allocator: AllocatorConfig) -> Self {
        self.config.allocator = allocator;
        self
    }

    /// Set the default allocator type
    pub fn default_allocator(mut self, allocator_type: AllocatorType) -> Self {
        self.config.allocator.default_allocator = allocator_type;
        self
    }

    /// Set maximum allocation size
    pub fn max_allocation_size(mut self, size: usize) -> Self {
        self.config.allocator.max_allocation_size = size;
        self
    }

    /// Enable or disable allocation tracking
    pub fn enable_tracking(mut self, enable: bool) -> Self {
        self.config.allocator.enable_tracking = enable;
        self
    }

    #[cfg(feature = "pool")]
    /// Set pool configuration
    pub fn pool(mut self, pool: PoolConfig) -> Self {
        self.config.pool = pool;
        self
    }

    #[cfg(feature = "arena")]
    /// Set arena configuration
    pub fn arena(mut self, arena: ArenaConfig) -> Self {
        self.config.arena = arena;
        self
    }

    #[cfg(feature = "cache")]
    /// Set cache configuration
    pub fn cache(mut self, cache: CacheConfig) -> Self {
        self.config.cache = cache;
        self
    }

    /// Build the configuration
    pub fn build(self) -> MemoryResult<MemoryConfig> {
        self.config.validate()?;
        Ok(self.config)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = MemoryConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_high_performance_config() {
        let config = MemoryConfig::high_performance();
        assert!(config.validate().is_ok());
        assert_eq!(config.allocator.default_allocator, AllocatorType::Bump);
    }

    #[test]
    fn test_low_memory_config() {
        let config = MemoryConfig::low_memory();
        assert!(config.validate().is_ok());
        assert!(config.allocator.enable_tracking);
    }

    #[test]
    fn test_config_builder() {
        let config = MemoryConfig::builder()
            .default_allocator(AllocatorType::Pool)
            .max_allocation_size(1024 * 1024)
            .enable_tracking(true)
            .build()
            .unwrap();

        assert_eq!(config.allocator.default_allocator, AllocatorType::Pool);
        assert_eq!(config.allocator.max_allocation_size, 1024 * 1024);
        assert!(config.allocator.enable_tracking);
    }

    #[test]
    fn test_invalid_config() {
        let mut config = MemoryConfig::default();
        config.allocator.max_allocation_size = 0;
        assert!(config.validate().is_err());
    }

    #[cfg(feature = "pool")]
    #[test]
    fn test_pool_config_validation() {
        let mut config = PoolConfig::default();
        config.max_capacity = config.default_capacity - 1;
        assert!(config.validate().is_err());
    }

    #[cfg(feature = "arena")]
    #[test]
    fn test_arena_config_validation() {
        let mut config = ArenaConfig::default();
        config.max_size = config.default_size - 1;
        assert!(config.validate().is_err());
    }
}
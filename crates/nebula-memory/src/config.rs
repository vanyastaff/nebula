//! Global configuration for nebula-memory
//!
//! This module provides configuration structures and initialization
//! for the memory management system.

#[cfg(all(not(feature = "std"), feature = "alloc"))]
extern crate alloc;
#[cfg(all(not(feature = "std"), feature = "alloc"))]
use alloc::boxed::Box;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
#[cfg(feature = "std")]
use std::sync::OnceLock;
#[cfg(feature = "std")]
use std::time::Duration;

#[cfg(not(feature = "std"))]
use once_cell::race::OnceBox;

/// Global memory configuration
#[derive(Debug, Clone)]
pub struct MemoryConfig {
    /// Enable global memory tracking
    pub enable_tracking: bool,

    /// Enable memory leak detection
    pub enable_leak_detection: bool,

    /// Global memory limit in bytes (None for unlimited)
    pub global_memory_limit: Option<usize>,

    /// Default pool configuration
    pub default_pool_config: DefaultPoolConfig,

    /// Default arena configuration
    pub default_arena_config: DefaultArenaConfig,

    /// Default cache configuration
    pub default_cache_config: DefaultCacheConfig,

    /// Platform-specific optimizations
    pub platform_optimizations: PlatformOptimizations,

    /// Memory pressure thresholds
    pub pressure_thresholds: PressureThresholds,

    /// Profiling configuration
    #[cfg(feature = "profiling")]
    pub profiling: ProfilingConfig,
}

/// Default configuration for object pools
#[derive(Debug, Clone)]
pub struct DefaultPoolConfig {
    /// Initial capacity for new pools
    pub initial_capacity: usize,

    /// Enable statistics by default
    pub enable_stats: bool,

    /// Validate objects on return by default
    pub validate_on_return: bool,

    /// Pre-warm pools on creation
    pub pre_warm: bool,

    /// Default growth strategy
    pub growth_factor: f32,
}

/// Default configuration for arenas
#[derive(Debug, Clone)]
pub struct DefaultArenaConfig {
    /// Default chunk size in bytes
    pub chunk_size: usize,

    /// Maximum chunk size
    pub max_chunk_size: usize,

    /// Chunk growth factor
    pub growth_factor: f32,

    /// Enable deallocation tracking
    pub track_deallocations: bool,
}

/// Default configuration for caches
#[derive(Debug, Clone)]
pub struct DefaultCacheConfig {
    /// Default eviction policy
    pub eviction_policy: EvictionPolicy,

    /// Default capacity
    pub default_capacity: usize,

    /// Enable statistics
    pub enable_stats: bool,

    /// TTL for cached items
    #[cfg(feature = "std")]
    pub default_ttl: Option<Duration>,
}

/// Memory eviction policies
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvictionPolicy {
    /// Least Recently Used
    LRU,
    /// Least Frequently Used
    LFU,
    /// First In First Out
    FIFO,
    /// Adaptive Replacement Cache
    ARC,
    /// Time-based eviction
    TTL,
}

/// Platform-specific optimizations
#[derive(Debug, Clone)]
pub struct PlatformOptimizations {
    /// Use huge pages when available
    pub use_huge_pages: bool,

    /// Enable NUMA awareness
    pub numa_aware: bool,

    /// Preferred NUMA node (-1 for any)
    pub preferred_numa_node: i32,

    /// Use platform-specific allocators
    pub use_platform_allocator: bool,

    /// Page size hint
    pub page_size: usize,

    /// CPU cache line size
    pub cache_line_size: usize,
}

/// Memory pressure thresholds
#[derive(Debug, Clone)]
pub struct PressureThresholds {
    /// Low pressure threshold (percentage)
    pub low: u8,

    /// Medium pressure threshold (percentage)
    pub medium: u8,

    /// High pressure threshold (percentage)
    pub high: u8,

    /// Critical pressure threshold (percentage)
    pub critical: u8,
}

/// Profiling configuration
#[cfg(feature = "profiling")]
#[derive(Debug, Clone)]
pub struct ProfilingConfig {
    /// Enable allocation tracking
    pub track_allocations: bool,

    /// Enable call stack recording
    pub record_callstacks: bool,

    /// Maximum callstack depth
    pub max_callstack_depth: usize,

    /// Enable heap profiling
    pub heap_profiling: bool,

    /// Sampling rate (1 in N allocations)
    pub sampling_rate: usize,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            enable_tracking: cfg!(feature = "stats"),
            enable_leak_detection: cfg!(debug_assertions),
            global_memory_limit: None,
            default_pool_config: DefaultPoolConfig::default(),
            default_arena_config: DefaultArenaConfig::default(),
            default_cache_config: DefaultCacheConfig::default(),
            platform_optimizations: PlatformOptimizations::default(),
            pressure_thresholds: PressureThresholds::default(),
            #[cfg(feature = "profiling")]
            profiling: ProfilingConfig::default(),
        }
    }
}

impl Default for DefaultPoolConfig {
    fn default() -> Self {
        Self {
            initial_capacity: 128,
            enable_stats: cfg!(feature = "stats"),
            validate_on_return: cfg!(debug_assertions),
            pre_warm: true,
            growth_factor: 2.0,
        }
    }
}

impl Default for DefaultArenaConfig {
    fn default() -> Self {
        Self {
            chunk_size: 64 * 1024,            // 64 KB
            max_chunk_size: 16 * 1024 * 1024, // 16 MB
            growth_factor: 2.0,
            track_deallocations: false,
        }
    }
}

impl Default for DefaultCacheConfig {
    fn default() -> Self {
        Self {
            eviction_policy: EvictionPolicy::LRU,
            default_capacity: 1000,
            enable_stats: cfg!(feature = "stats"),
            #[cfg(feature = "std")]
            default_ttl: None,
        }
    }
}

impl Default for PlatformOptimizations {
    fn default() -> Self {
        Self {
            use_huge_pages: false,
            numa_aware: cfg!(target_os = "linux"),
            preferred_numa_node: -1,
            use_platform_allocator: true,
            page_size: 4096,
            cache_line_size: 64,
        }
    }
}

impl Default for PressureThresholds {
    fn default() -> Self {
        Self { low: 50, medium: 70, high: 85, critical: 95 }
    }
}

#[cfg(feature = "profiling")]
impl Default for ProfilingConfig {
    fn default() -> Self {
        Self {
            track_allocations: true,
            record_callstacks: cfg!(debug_assertions),
            max_callstack_depth: 32,
            heap_profiling: false,
            sampling_rate: 100,
        }
    }
}

// Global configuration instance
#[cfg(feature = "std")]
static GLOBAL_CONFIG: OnceLock<MemoryConfig> = OnceLock::new();

#[cfg(not(feature = "std"))]
static GLOBAL_CONFIG: OnceBox<MemoryConfig> = OnceBox::new();

// Runtime state
static INITIALIZED: AtomicBool = AtomicBool::new(false);
static TOTAL_ALLOCATED: AtomicUsize = AtomicUsize::new(0);
static ACTIVE_ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

/// Initialize the memory system with configuration
pub fn initialize(config: MemoryConfig) -> Result<(), ConfigError> {
    if INITIALIZED.load(Ordering::Acquire) {
        return Err(ConfigError::AlreadyInitialized);
    }

    // Validate configuration
    config.validate()?;

    // Set global configuration
    #[cfg(feature = "std")]
    {
        GLOBAL_CONFIG.set(config).map_err(|_| ConfigError::AlreadyInitialized)?;
    }

    #[cfg(not(feature = "std"))]
    {
        GLOBAL_CONFIG.set(Box::new(config)).map_err(|_| ConfigError::AlreadyInitialized)?;
    }

    // Initialize platform-specific features
    #[cfg(all(feature = "platform", feature = "std"))]
    if let Err(e) = crate::platform::initialize() {
        return Err(ConfigError::InitializationError(format!(
            "Platform initialization failed: {}",
            e
        )));
    }

    INITIALIZED.store(true, Ordering::Release);
    Ok(())
}

/// Get the global configuration
pub fn get() -> &'static MemoryConfig {
    #[cfg(feature = "std")]
    {
        GLOBAL_CONFIG.get().unwrap_or_else(|| {
            // Initialize with defaults if not set
            let _ = initialize(MemoryConfig::default());
            GLOBAL_CONFIG.get().unwrap()
        })
    }

    #[cfg(not(feature = "std"))]
    {
        GLOBAL_CONFIG.get().unwrap_or_else(|| {
            // For no_std, panic if not initialized
            panic!("Memory system not initialized");
        })
    }
}

/// Check if the memory system is initialized
pub fn is_initialized() -> bool {
    INITIALIZED.load(Ordering::Acquire)
}

/// Update total allocated memory
pub fn add_allocation(size: usize) {
    TOTAL_ALLOCATED.fetch_add(size, Ordering::Relaxed);
    ACTIVE_ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
}

/// Update total allocated memory
pub fn remove_allocation(size: usize) {
    TOTAL_ALLOCATED.fetch_sub(size, Ordering::Relaxed);
    ACTIVE_ALLOCATIONS.fetch_sub(1, Ordering::Relaxed);
}

/// Get total allocated memory
pub fn total_allocated() -> usize {
    TOTAL_ALLOCATED.load(Ordering::Relaxed)
}

/// Get number of active allocations
pub fn active_allocations() -> usize {
    ACTIVE_ALLOCATIONS.load(Ordering::Relaxed)
}

/// Configuration errors
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// System already initialized
    AlreadyInitialized,
    /// Invalid configuration value
    InvalidValue(&'static str),
    /// Platform error
    PlatformError(&'static str),
    /// Initialization error
    InitializationError(String),
}

impl core::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::AlreadyInitialized => write!(f, "Memory system already initialized"),
            Self::InvalidValue(msg) => write!(f, "Invalid configuration: {}", msg),
            Self::PlatformError(msg) => write!(f, "Platform error: {}", msg),
            Self::InitializationError(msg) => write!(f, "Initialization error: {}", msg),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for ConfigError {}

impl MemoryConfig {
    /// Validate configuration
    pub fn validate(&self) -> Result<(), ConfigError> {
        // Validate pool config
        if self.default_pool_config.growth_factor < 1.0 {
            return Err(ConfigError::InvalidValue("Pool growth factor must be >= 1.0"));
        }

        // Validate arena config
        if self.default_arena_config.chunk_size == 0 {
            return Err(ConfigError::InvalidValue("Arena chunk size must be > 0"));
        }

        if self.default_arena_config.max_chunk_size < self.default_arena_config.chunk_size {
            return Err(ConfigError::InvalidValue("Max chunk size must be >= chunk size"));
        }

        // Validate pressure thresholds
        if self.pressure_thresholds.low >= self.pressure_thresholds.medium {
            return Err(ConfigError::InvalidValue("Low threshold must be < medium"));
        }

        if self.pressure_thresholds.medium >= self.pressure_thresholds.high {
            return Err(ConfigError::InvalidValue("Medium threshold must be < high"));
        }

        if self.pressure_thresholds.high >= self.pressure_thresholds.critical {
            return Err(ConfigError::InvalidValue("High threshold must be < critical"));
        }

        if self.pressure_thresholds.critical > 100 {
            return Err(ConfigError::InvalidValue("Critical threshold must be <= 100"));
        }

        Ok(())
    }

    /// Create a builder for configuration
    pub fn builder() -> ConfigBuilder {
        ConfigBuilder::new()
    }
}

/// Builder for memory configuration
pub struct ConfigBuilder {
    config: MemoryConfig,
}

impl ConfigBuilder {
    /// Create new builder with defaults
    pub fn new() -> Self {
        Self { config: MemoryConfig::default() }
    }

    /// Enable or disable memory tracking
    pub fn enable_tracking(mut self, enable: bool) -> Self {
        self.config.enable_tracking = enable;
        self
    }

    /// Enable or disable leak detection
    pub fn enable_leak_detection(mut self, enable: bool) -> Self {
        self.config.enable_leak_detection = enable;
        self
    }

    /// Set global memory limit
    pub fn global_memory_limit(mut self, limit: Option<usize>) -> Self {
        self.config.global_memory_limit = limit;
        self
    }

    /// Set default pool capacity
    pub fn default_pool_capacity(mut self, capacity: usize) -> Self {
        self.config.default_pool_config.initial_capacity = capacity;
        self
    }

    /// Set default arena chunk size
    pub fn default_arena_chunk_size(mut self, size: usize) -> Self {
        self.config.default_arena_config.chunk_size = size;
        self
    }

    /// Set default cache capacity
    pub fn default_cache_capacity(mut self, capacity: usize) -> Self {
        self.config.default_cache_config.default_capacity = capacity;
        self
    }

    /// Set default eviction policy
    pub fn default_eviction_policy(mut self, policy: EvictionPolicy) -> Self {
        self.config.default_cache_config.eviction_policy = policy;
        self
    }

    /// Enable huge pages
    pub fn use_huge_pages(mut self, enable: bool) -> Self {
        self.config.platform_optimizations.use_huge_pages = enable;
        self
    }

    /// Enable NUMA awareness
    pub fn numa_aware(mut self, enable: bool) -> Self {
        self.config.platform_optimizations.numa_aware = enable;
        self
    }

    /// Set preferred NUMA node
    pub fn preferred_numa_node(mut self, node: i32) -> Self {
        self.config.platform_optimizations.preferred_numa_node = node;
        self
    }

    /// Set memory pressure thresholds
    pub fn pressure_thresholds(mut self, low: u8, medium: u8, high: u8, critical: u8) -> Self {
        self.config.pressure_thresholds = PressureThresholds { low, medium, high, critical };
        self
    }

    /// Build the configuration
    pub fn build(self) -> Result<MemoryConfig, ConfigError> {
        self.config.validate()?;
        Ok(self.config)
    }
}

impl Default for ConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_validation() {
        let config = MemoryConfig::default();
        assert!(config.validate().is_ok());

        // Test invalid pressure thresholds
        let mut bad_config = config.clone();
        bad_config.pressure_thresholds.low = 80;
        bad_config.pressure_thresholds.medium = 70;
        assert!(bad_config.validate().is_err());
    }

    #[test]
    fn test_config_builder() {
        let config = MemoryConfig::builder()
            .enable_tracking(true)
            .global_memory_limit(Some(1024 * 1024 * 1024))
            .default_pool_capacity(256)
            .use_huge_pages(true)
            .build()
            .unwrap();

        assert!(config.enable_tracking);
        assert_eq!(config.global_memory_limit, Some(1024 * 1024 * 1024));
        assert_eq!(config.default_pool_config.initial_capacity, 256);
        assert!(config.platform_optimizations.use_huge_pages);
    }
}

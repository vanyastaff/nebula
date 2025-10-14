//! Pool allocator configuration

/// Configuration for pool allocator
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// Enable statistics tracking
    pub track_stats: bool,

    /// Fill pattern byte for newly allocated memory (for debugging)
    pub alloc_pattern: Option<u8>,
    /// Fill pattern byte for deallocated memory (for debugging)
    pub dealloc_pattern: Option<u8>,

    /// Use exponential backoff for CAS retries
    pub use_backoff: bool,

    /// Maximum CAS retry attempts before failing
    pub max_retries: usize,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            track_stats: cfg!(debug_assertions),
            alloc_pattern: if cfg!(debug_assertions) {
                Some(0xBB)
            } else {
                None
            },
            dealloc_pattern: if cfg!(debug_assertions) {
                Some(0xDD)
            } else {
                None
            },
            use_backoff: true,
            max_retries: 1000,
        }
    }
}

impl PoolConfig {
    /// Production configuration - optimized for performance
    #[must_use]
    pub fn production() -> Self {
        Self {
            track_stats: false,
            alloc_pattern: None,
            dealloc_pattern: None,
            use_backoff: true,
            max_retries: 10000,
        }
    }

    /// Debug configuration - optimized for debugging
    #[must_use]
    pub fn debug() -> Self {
        Self {
            track_stats: true,
            alloc_pattern: Some(0xBB),
            dealloc_pattern: Some(0xDD),
            use_backoff: false,
            max_retries: 100,
        }
    }

    /// Performance configuration - minimal overhead
    #[must_use]
    pub fn performance() -> Self {
        Self {
            track_stats: false,
            alloc_pattern: None,
            dealloc_pattern: None,
            use_backoff: false,
            max_retries: 100,
        }
    }
}

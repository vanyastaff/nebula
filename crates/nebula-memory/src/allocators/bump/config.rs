//! Configuration types for bump allocator

/// Configuration for bump allocator
#[derive(Debug, Clone)]
pub struct BumpConfig {
    /// Enable statistics tracking
    pub track_stats: bool,

    /// Fill patterns for debugging
    pub alloc_pattern: Option<u8>,
    pub dealloc_pattern: Option<u8>,

    /// Prefetching configuration
    pub enable_prefetch: bool,
    pub prefetch_distance: usize,

    /// Minimum allocation size (helps avoid false sharing)
    pub min_alloc_size: usize,

    /// Thread-safe mode (use atomics vs Cell)
    pub thread_safe: bool,
}

impl Default for BumpConfig {
    fn default() -> Self {
        Self {
            track_stats: cfg!(debug_assertions),
            alloc_pattern: if cfg!(debug_assertions) { Some(0xAA) } else { None },
            dealloc_pattern: if cfg!(debug_assertions) { Some(0xDD) } else { None },
            enable_prefetch: true,
            prefetch_distance: 4,
            min_alloc_size: 8,
            thread_safe: true,
        }
    }
}

impl BumpConfig {
    /// Production configuration - optimized for maximum performance
    pub fn production() -> Self {
        Self {
            track_stats: false,
            alloc_pattern: None,
            dealloc_pattern: None,
            enable_prefetch: true,
            prefetch_distance: 8,
            min_alloc_size: 16,
            thread_safe: true,
        }
    }

    /// Debug configuration - optimized for debugging and error detection
    pub fn debug() -> Self {
        Self {
            track_stats: true,
            alloc_pattern: Some(0xAA),
            dealloc_pattern: Some(0xDD),
            enable_prefetch: false,
            prefetch_distance: 0,
            min_alloc_size: 1,
            thread_safe: true,
        }
    }

    /// Single-threaded configuration - avoids atomic overhead
    pub fn single_thread() -> Self {
        Self {
            thread_safe: false,
            ..Self::production()
        }
    }

    /// Performance-optimized configuration
    pub fn performance() -> Self {
        Self {
            track_stats: false,
            alloc_pattern: None,
            dealloc_pattern: None,
            enable_prefetch: true,
            prefetch_distance: 16,
            min_alloc_size: 64,
            thread_safe: true,
        }
    }

    /// Conservative configuration - balanced defaults
    pub fn conservative() -> Self {
        Self {
            track_stats: true,
            alloc_pattern: None,
            dealloc_pattern: None,
            enable_prefetch: true,
            prefetch_distance: 2,
            min_alloc_size: 8,
            thread_safe: true,
        }
    }
}

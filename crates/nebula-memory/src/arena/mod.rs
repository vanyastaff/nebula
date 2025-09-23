//! High-performance arena allocation module for nebula-memory
//!
//! This module provides various arena allocators optimized for different use
//! cases:
//!
//! # Arena Types
//!
//! - [`Arena`]: Basic single-threaded bump allocator for general use
//! - [`TypedArena<T>`]: Type-safe arena for homogeneous allocations
//! - [`ThreadSafeArena`]: Lock-free arena with atomic operations for concurrent
//!   access
//! - [`LocalArena`]: Thread-local arena for maximum performance
//! - [`CrossThreadArena`]: Arena that can be moved between threads (exclusive
//!   access)
//! - [`CompressedArena`]: Arena with transparent compression support
//! - [`StreamingArena`]: Arena optimized for streaming/sequential allocation
//!   patterns
//!
//! # Examples
//!
//! Basic usage:
//! ```rust
//! use nebula_memory::arena::{Arena, ArenaConfig};
//!
//! let arena = Arena::new(ArenaConfig::default());
//! let value = arena.alloc(42).unwrap();
//! assert_eq!(*value, 42);
//! ```
//!
//! Thread-safe usage:
//! ```rust
//! use std::sync::Arc;
//! use std::thread;
//!
//! use nebula_memory::arena::{ArenaConfig, ThreadSafeArena};
//!
//! let arena = Arc::new(ThreadSafeArena::new(ArenaConfig::default()));
//! let arena_clone = Arc::clone(&arena);
//!
//! let handle = thread::spawn(move || {
//!     let value = arena_clone.alloc(100).unwrap();
//!     *value
//! });
//!
//! assert_eq!(handle.join().unwrap(), 100);
//! ```

use std::alloc::Layout;

use crate::error::MemoryError;

// Core arena implementations
mod allocator;
mod arena;
mod cross_thread;
mod local;
mod stats;
mod thread_safe;
mod typed;

// Optional features
#[cfg(feature = "streaming")]
mod streaming;

#[cfg(feature = "compression")]
mod compressed;

// Macros for convenient arena usage
#[macro_use]
mod macros;

// Re-exports
pub use macros::StrictArena;

pub use self::allocator::{ArenaAllocator, ArenaBackedVec};
pub use self::arena::{Arena, ArenaRef, ArenaRefMut};
#[cfg(feature = "compression")]
pub use self::compressed::{CompressedArena, CompressionLevel, CompressionStats};
pub use self::cross_thread::{
    CrossThreadArena, CrossThreadArenaBuilder, CrossThreadArenaGuard, CrossThreadArenaRef,
};
pub use self::local::{
    alloc_local, local_arena, reset_local_arena, with_local_arena, with_local_arena_mut,
    LocalArena, LocalRef, LocalRefMut,
};
pub use self::stats::{ArenaStats, ArenaStatsSnapshot};
#[cfg(feature = "streaming")]
pub use self::streaming::{StreamCheckpoint, StreamOptions, StreamingArena, StreamingArenaRef};
pub use self::thread_safe::{ThreadSafeArena, ThreadSafeArenaRef};
pub use self::typed::{TypedArena, TypedArenaRef};
// Re-export macros at crate level
pub use crate::{
    arena_alloc, arena_alloc_or, arena_config, arena_debug, arena_str, arena_struct, arena_vec,
    bench_arena, impl_arena_alloc, local_alloc, shared_arena, strict_arena, try_arena_alloc,
    typed_arena, with_arena,
};

/// Core trait for arena allocation
///
/// This trait defines the interface that all arena types must implement.
pub trait ArenaAllocate {
    /// Allocates raw bytes with specified alignment
    ///
    /// # Safety
    ///
    /// The caller must ensure:
    /// - The memory is properly initialized before use
    /// - The memory is not accessed after arena reset
    /// - Alignment is a power of two
    unsafe fn alloc_bytes(&self, size: usize, align: usize) -> Result<*mut u8, MemoryError>;

    /// Allocates and initializes a value
    fn alloc<T>(&self, value: T) -> Result<&mut T, MemoryError> {
        let layout = Layout::new::<T>();
        let ptr = unsafe { self.alloc_bytes(layout.size(), layout.align())? } as *mut T;

        unsafe {
            ptr.write(value);
            Ok(&mut *ptr)
        }
    }

    /// Allocates and initializes a slice
    fn alloc_slice<T: Copy>(&self, slice: &[T]) -> Result<&mut [T], MemoryError> {
        if slice.is_empty() {
            return Ok(&mut []);
        }

        let layout = Layout::for_value(slice);
        let ptr = unsafe { self.alloc_bytes(layout.size(), layout.align())? } as *mut T;

        unsafe {
            std::ptr::copy_nonoverlapping(slice.as_ptr(), ptr, slice.len());
            Ok(std::slice::from_raw_parts_mut(ptr, slice.len()))
        }
    }

    /// Allocates a string
    fn alloc_str(&self, s: &str) -> Result<&str, MemoryError> {
        let bytes = self.alloc_slice(s.as_bytes())?;
        // Safety: bytes came from valid UTF-8
        unsafe { Ok(std::str::from_utf8_unchecked(bytes)) }
    }

    /// Returns allocation statistics
    fn stats(&self) -> &ArenaStats;

    /// Resets the arena, invalidating all allocations
    fn reset(&mut self);

    /// Returns the total capacity of the arena
    fn capacity(&self) -> usize {
        self.stats().bytes_allocated()
    }

    /// Returns the amount of memory currently used
    fn used(&self) -> usize {
        self.stats().bytes_used()
    }

    /// Returns the amount of memory available
    fn available(&self) -> usize {
        self.capacity().saturating_sub(self.used())
    }
}

/// Thread-safe arena allocation trait
///
/// This trait extends `ArenaAllocate` with thread-safety guarantees.
/// Implementors must be both `Send` and `Sync`.
pub trait ThreadSafeArenaAllocate: ArenaAllocate + Send + Sync {}

/// Arena configuration builder
#[derive(Debug, Clone)]
pub struct ArenaConfig {
    /// Initial size of the first chunk
    pub initial_size: usize,
    /// Growth factor for subsequent chunks (must be >= 1.0)
    pub growth_factor: f64,
    /// Maximum size of a single chunk
    pub max_chunk_size: usize,
    /// Whether to track detailed statistics
    pub track_stats: bool,
    /// Whether to zero memory on allocation
    pub zero_memory: bool,
    /// Default alignment for allocations
    pub default_alignment: usize,
    /// Enable NUMA awareness (if supported)
    #[cfg(feature = "numa-aware")]
    pub numa_aware: bool,
    /// Preferred NUMA node (-1 for any)
    #[cfg(feature = "numa-aware")]
    pub numa_node: i32,
}

impl ArenaConfig {
    /// Creates new config with default values
    pub fn new() -> Self {
        Self {
            initial_size: 4096, // 4KB
            growth_factor: 2.0,
            max_chunk_size: 16 * 1024 * 1024, // 16MB
            track_stats: cfg!(debug_assertions),
            zero_memory: false,
            default_alignment: 8,
            #[cfg(feature = "numa-aware")]
            numa_aware: false,
            #[cfg(feature = "numa-aware")]
            numa_node: -1,
        }
    }

    /// Creates config from global memory configuration
    pub fn from_memory_config(config: &crate::config::MemoryConfig) -> Self {
        Self {
            initial_size: config.default_arena_config.chunk_size,
            growth_factor: config.default_arena_config.growth_factor as f64,
            max_chunk_size: config.default_arena_config.max_chunk_size,
            track_stats: config.enable_tracking,
            zero_memory: false, // Not directly mapped from current config
            default_alignment: config.platform_optimizations.cache_line_size.min(8),
            #[cfg(feature = "numa-aware")]
            numa_aware: config.platform_optimizations.numa_aware,
            #[cfg(feature = "numa-aware")]
            numa_node: config.platform_optimizations.preferred_numa_node,
        }
    }

    /// Sets initial chunk size
    pub fn with_initial_size(mut self, size: usize) -> Self {
        self.initial_size = size;
        self
    }

    /// Sets growth factor (must be >= 1.0)
    pub fn with_growth_factor(mut self, factor: f64) -> Self {
        assert!(factor >= 1.0, "Growth factor must be >= 1.0");
        self.growth_factor = factor;
        self
    }

    /// Sets maximum chunk size
    pub fn with_max_chunk_size(mut self, size: usize) -> Self {
        self.max_chunk_size = size;
        self
    }

    /// Enables/disables statistics tracking
    pub fn with_stats(mut self, enabled: bool) -> Self {
        self.track_stats = enabled;
        self
    }

    /// Enables/disables zeroing memory
    pub fn with_zero_memory(mut self, enabled: bool) -> Self {
        self.zero_memory = enabled;
        self
    }

    /// Sets default alignment (must be power of 2)
    pub fn with_alignment(mut self, align: usize) -> Self {
        assert!(align.is_power_of_two(), "Alignment must be power of 2");
        self.default_alignment = align;
        self
    }

    /// Enables NUMA awareness
    #[cfg(feature = "numa-aware")]
    pub fn with_numa_aware(mut self, enabled: bool) -> Self {
        self.numa_aware = enabled;
        self
    }

    /// Sets preferred NUMA node
    #[cfg(feature = "numa-aware")]
    pub fn with_numa_node(mut self, node: i32) -> Self {
        self.numa_node = node;
        self
    }

    /// Validates the configuration
    pub fn validate(&self) -> Result<(), MemoryError> {
        if self.initial_size == 0 {
            return Err(MemoryError::InvalidLayout {
                reason: "Initial size must be greater than 0",
            });
        }

        if self.growth_factor < 1.0 {
            return Err(MemoryError::InvalidLayout { reason: "Growth factor must be >= 1.0" });
        }

        if !self.default_alignment.is_power_of_two() {
            return Err(MemoryError::InvalidLayout {
                reason: "Default alignment must be power of 2",
            });
        }

        if self.max_chunk_size < self.initial_size {
            return Err(MemoryError::InvalidLayout {
                reason: "Max chunk size must be >= initial size",
            });
        }

        Ok(())
    }
}

impl Default for ArenaConfig {
    fn default() -> Self {
        Self::new()
    }
}

// Arena creation helpers

/// Creates new arena with default config
pub fn new_arena() -> Arena {
    Arena::new(ArenaConfig::default())
}

/// Creates new arena with initial capacity
pub fn new_arena_with_capacity(capacity: usize) -> Arena {
    Arena::new(ArenaConfig::default().with_initial_size(capacity))
}

/// Creates new typed arena
pub fn new_typed_arena<T>() -> TypedArena<T> {
    TypedArena::new()
}

/// Creates new typed arena with capacity
pub fn new_typed_arena_with_capacity<T>(capacity: usize) -> TypedArena<T> {
    TypedArena::with_capacity(capacity)
}

/// Creates new thread-safe arena
pub fn new_thread_safe_arena() -> ThreadSafeArena {
    ThreadSafeArena::new(ArenaConfig::default())
}

/// Creates new thread-safe arena with config
pub fn new_thread_safe_arena_with_config(config: ArenaConfig) -> ThreadSafeArena {
    ThreadSafeArena::new(config)
}

/// Creates new cross-thread arena
pub fn new_cross_thread_arena() -> CrossThreadArena {
    CrossThreadArena::new(ArenaConfig::default())
}

/// Creates new compressed arena
#[cfg(feature = "compression")]
pub fn new_compressed_arena(block_size: usize, level: CompressionLevel) -> CompressedArena {
    CompressedArena::new(block_size, level)
}

/// Creates new streaming arena
#[cfg(feature = "streaming")]
pub fn new_streaming_arena<T>(options: StreamOptions) -> StreamingArena<T> {
    StreamingArena::new(options)
}

// Performance hints

/// Hints for arena usage patterns
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArenaHint {
    /// Sequential allocation pattern
    Sequential,
    /// Random access pattern
    Random,
    /// Temporary allocations (will be reset soon)
    Temporary,
    /// Long-lived allocations
    LongLived,
    /// Small frequent allocations
    SmallObjects,
    /// Large infrequent allocations
    LargeObjects,
}

/// Apply hints to arena configuration
pub fn apply_hints(mut config: ArenaConfig, hints: &[ArenaHint]) -> ArenaConfig {
    for hint in hints {
        match hint {
            ArenaHint::Sequential => {
                // Larger chunks for sequential access
                config.initial_size = config.initial_size.max(64 * 1024);
            },
            ArenaHint::Random => {
                // Smaller chunks to reduce waste
                config.initial_size = config.initial_size.min(16 * 1024);
            },
            ArenaHint::Temporary => {
                // Don't zero memory for temporary allocations
                config.zero_memory = false;
                // Disable stats for performance
                config.track_stats = false;
            },
            ArenaHint::LongLived => {
                // Enable stats for monitoring
                config.track_stats = true;
            },
            ArenaHint::SmallObjects => {
                // Smaller initial size, faster growth
                config.initial_size = config.initial_size.min(8 * 1024);
                config.growth_factor = config.growth_factor.max(2.0);
            },
            ArenaHint::LargeObjects => {
                // Larger initial size, slower growth
                config.initial_size = config.initial_size.max(256 * 1024);
                config.growth_factor = config.growth_factor.min(1.5);
            },
        }
    }
    config
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::{align_up, is_aligned};

    #[test]
    fn test_alignment_utils() {
        assert_eq!(align_up(0, 8), 0);
        assert_eq!(align_up(1, 8), 8);
        assert_eq!(align_up(7, 8), 8);
        assert_eq!(align_up(8, 8), 8);
        assert_eq!(align_up(9, 8), 16);

        assert!(is_aligned(0x1000, 8));
        assert!(is_aligned(0x1000, 16));
        assert!(is_aligned(0x1000, 4096));
        assert!(!is_aligned(0x1001, 8));
    }

    #[test]
    fn test_config_builder() {
        let config = ArenaConfig::new()
            .with_initial_size(8192)
            .with_growth_factor(1.5)
            .with_stats(true)
            .with_zero_memory(true)
            .with_alignment(16);

        assert_eq!(config.initial_size, 8192);
        assert_eq!(config.growth_factor, 1.5);
        assert!(config.track_stats);
        assert!(config.zero_memory);
        assert_eq!(config.default_alignment, 16);

        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_config_validation() {
        let invalid_config = ArenaConfig { initial_size: 0, ..Default::default() };
        assert!(invalid_config.validate().is_err());

        let invalid_growth = ArenaConfig { growth_factor: 0.5, ..Default::default() };
        assert!(invalid_growth.validate().is_err());

        let invalid_align = ArenaConfig {
            default_alignment: 3, // Not power of 2
            ..Default::default()
        };
        assert!(invalid_align.validate().is_err());
    }

    #[test]
    fn test_arena_hints() {
        let config = ArenaConfig::default();

        let sequential_config = apply_hints(config.clone(), &[ArenaHint::Sequential]);
        assert!(sequential_config.initial_size >= 64 * 1024);

        let temp_config = apply_hints(config.clone(), &[ArenaHint::Temporary]);
        assert!(!temp_config.zero_memory);
        assert!(!temp_config.track_stats);

        let small_obj_config = apply_hints(config.clone(), &[ArenaHint::SmallObjects]);
        assert!(small_obj_config.growth_factor >= 2.0);
    }

    #[test]
    fn test_arena_creation_helpers() {
        let arena1 = new_arena();
        let value1 = arena1.alloc(42).unwrap();
        assert_eq!(*value1, 42);

        let arena2 = new_arena_with_capacity(8192);
        let value2 = arena2.alloc(100).unwrap();
        assert_eq!(*value2, 100);

        let typed_arena = new_typed_arena::<String>();
        let string = typed_arena.alloc("Hello".to_string()).unwrap();
        assert_eq!(string, "Hello");

        let thread_safe = new_thread_safe_arena();
        let value3 = thread_safe.alloc(200).unwrap();
        assert_eq!(*value3, 200);
    }

    #[cfg(feature = "numa-aware")]
    #[test]
    fn test_numa_config() {
        let config = ArenaConfig::new().with_numa_aware(true).with_numa_node(0);

        assert!(config.numa_aware);
        assert_eq!(config.numa_node, 0);
    }
}

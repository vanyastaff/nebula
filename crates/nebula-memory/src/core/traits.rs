//! Core traits for memory management
//!
//! This module defines the fundamental traits used throughout nebula-memory.

use super::error::{MemoryError, MemoryResult};
use core::alloc::Layout;

/// Core memory management trait for allocators and pools
///
/// This trait defines the basic interface for memory allocation and deallocation.
/// It is implemented by custom allocators, pools, and arenas.
pub trait MemoryManager {
    /// Allocate memory with the given layout
    ///
    /// # Safety
    /// The returned pointer must be valid for reads and writes of `layout.size()` bytes,
    /// and must be properly aligned to `layout.align()`.
    unsafe fn allocate(&mut self, layout: Layout) -> Result<*mut u8, MemoryError>;

    /// Deallocate previously allocated memory
    ///
    /// # Safety
    /// - `ptr` must have been allocated by this allocator with the same layout
    /// - `ptr` must not be used after deallocation
    unsafe fn deallocate(&mut self, ptr: *mut u8, layout: Layout);

    /// Reset the memory manager to initial state
    ///
    /// This operation deallocates all memory and returns the manager to a clean state.
    fn reset(&mut self) -> MemoryResult<()>;

    /// Get the name of this memory manager for debugging
    fn name(&self) -> &'static str;
}

/// Memory usage tracking trait
///
/// Implemented by allocators and memory managers that track usage statistics.
/// Provides both basic capacity information and convenience methods for monitoring.
pub trait MemoryUsage {
    /// Get currently used memory in bytes
    fn used_memory(&self) -> usize;

    /// Get available memory in bytes (if known)
    fn available_memory(&self) -> Option<usize>;

    /// Get total memory capacity in bytes (if known)
    fn total_memory(&self) -> Option<usize> {
        match (self.used_memory(), self.available_memory()) {
            (used, Some(available)) => Some(used + available),
            _ => None,
        }
    }

    /// Returns memory usage as a percentage (0.0 to 100.0)
    ///
    /// Returns `None` if total memory is unknown or zero.
    /// Useful for implementing memory pressure warnings.
    fn memory_usage_percent(&self) -> Option<f32> {
        self.total_memory().and_then(|total| {
            if total == 0 {
                Some(0.0)
            } else {
                Some((self.used_memory() as f32 / total as f32) * 100.0)
            }
        })
    }

    /// Checks if memory usage is above the specified percentage threshold
    ///
    /// Returns `None` if usage percentage cannot be determined.
    fn is_memory_pressure(&self, threshold_percent: f32) -> Option<bool> {
        self.memory_usage_percent()
            .map(|usage| usage >= threshold_percent)
    }

    /// Returns detailed memory usage information
    ///
    /// Provides a basic view of memory usage.
    fn memory_usage(&self) -> BasicMemoryUsage {
        BasicMemoryUsage {
            used: self.used_memory(),
            available: self.available_memory(),
            total: self.total_memory(),
            usage_percent: self.memory_usage_percent(),
        }
    }
}

/// Basic memory usage information
///
/// Simplified view focusing on capacity management.
/// For detailed metrics, see `AllocatorStats` from the allocator module.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BasicMemoryUsage {
    /// Currently used memory in bytes
    pub used: usize,
    /// Available memory in bytes (None if unlimited)
    pub available: Option<usize>,
    /// Total memory capacity in bytes (None if unlimited)
    pub total: Option<usize>,
    /// Memory usage as percentage (None if cannot be calculated)
    pub usage_percent: Option<f32>,
}

impl core::fmt::Display for BasicMemoryUsage {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "used: {} bytes", self.used)?;

        if let Some(total) = self.total {
            write!(f, ", total: {} bytes", total)?;
        }

        if let Some(percent) = self.usage_percent {
            write!(f, " ({:.1}%)", percent)?;
        }

        Ok(())
    }
}

/// Resettable memory manager
///
/// Trait for allocators that support resetting to initial state.
/// Resetting invalidates all previous allocations.
pub trait Resettable {
    /// Reset allocator to initial state
    ///
    /// # Safety
    /// - All pointers allocated before reset become invalid immediately
    /// - Using invalidated pointers results in undefined behavior
    /// - Caller must ensure no live references exist before calling
    unsafe fn reset(&self);

    /// Check if this allocator can be reset
    ///
    /// Returns `true` if reset is safe to perform.
    /// Some allocators may not support reset in certain states.
    fn can_reset(&self) -> bool {
        true
    }

    /// Resets the allocator only if it's safe to do so
    ///
    /// Returns `true` if reset was performed, `false` if skipped.
    ///
    /// # Safety
    /// Same requirements as `reset()`, but only applies if actually performed.
    unsafe fn try_reset(&self) -> bool {
        if self.can_reset() {
            unsafe { self.reset() };
            true
        } else {
            false
        }
    }
}

/// Cloneable allocator
///
/// Trait for allocators that can create independent clones.
pub trait CloneAllocator: Sized {
    /// Create an independent clone of this allocator
    fn clone_allocator(&self) -> Self;
}

/// Statistics provider trait
///
/// Trait for allocators that support statistics collection.
///
/// Re-exported from allocator module for convenience.
pub trait StatisticsProvider {
    /// Get current statistics
    fn statistics(&self) -> crate::allocator::AllocatorStats;

    /// Reset statistics
    fn reset_statistics(&self);

    /// Check if statistics collection is enabled
    fn statistics_enabled(&self) -> bool {
        true
    }
}

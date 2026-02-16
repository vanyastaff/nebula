//! Sealed trait pattern for internal allocator contracts
//!
//! This module provides traits that are only implementable within `nebula-memory`.
//! External crates can use these traits as bounds, but cannot implement them.
//!
//! # Why Sealed Traits?
//!
//! Sealed traits prevent external implementations while allowing:
//! - Use as trait bounds in public APIs
//! - Adding methods in minor versions (semver-compatible)
//! - Maintaining strict internal invariants
//! - Exhaustive matching over known implementations
//!
//! # Example
//!
//! ```rust
//! use nebula_memory::allocator::sealed::AllocatorInternal;
//!
//! // ✓ Can use as bound
//! fn optimize<A: AllocatorInternal>(alloc: &A) {
//!     let checkpoint = alloc.internal_checkpoint();
//!     // Use internal APIs
//! }
//!
//! // ✗ Cannot implement externally
//! // impl AllocatorInternal for MyAllocator { }  // ERROR: Sealed not accessible
//! ```

use core::fmt;

use crate::allocator::{AllocResult, Allocator};

// ============================================================================
// Sealing Mechanism
// ============================================================================

mod private {
    /// Private sealing trait - cannot be named or implemented outside this module
    ///
    /// This trait is intentionally empty and private. It serves as the sealing
    /// mechanism that prevents external implementations of public sealed traits.
    pub trait Sealed {}

    // ========================================================================
    // Seal all internal allocator types
    // ========================================================================

    // Core allocators
    impl Sealed for crate::allocator::system::SystemAllocator {}
    impl Sealed for crate::allocator::bump::BumpAllocator {}
    impl Sealed for crate::allocator::stack::StackAllocator {}
    impl Sealed for crate::allocator::pool::PoolAllocator {}

    // Wrapper allocators - sealed for any inner allocator type
    impl<A> Sealed for crate::allocator::tracked::TrackedAllocator<A> {}

    #[cfg(feature = "monitoring")]
    impl<A> Sealed for crate::allocator::monitored::MonitoredAllocator<A> {}

    // ========================================================================
    // Generic sealed implementations
    // ========================================================================

    // References to sealed types are also sealed
    impl<T: ?Sized + Sealed> Sealed for &T {}
    impl<T: ?Sized + Sealed> Sealed for &mut T {}

    // Boxed sealed types are sealed
    #[cfg(feature = "std")]
    impl<T: ?Sized + Sealed> Sealed for Box<T> {}
}

// ============================================================================
// Internal Checkpoint for State Management
// ============================================================================

/// Internal checkpoint representation for allocator state
///
/// Captures allocator state for later restoration. The exact representation
/// is implementation-specific and opaque to users.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InternalCheckpoint {
    /// Byte offset within current chunk/region
    pub(crate) offset: usize,
    /// Unique identifier for the chunk/region
    pub(crate) chunk_id: u64,
    /// Generation counter to detect stale checkpoints
    pub(crate) generation: u32,
}

impl InternalCheckpoint {
    /// Create a new checkpoint
    #[inline]
    pub(crate) const fn new(offset: usize, chunk_id: u64, generation: u32) -> Self {
        Self {
            offset,
            chunk_id,
            generation,
        }
    }
}

// ============================================================================
// Fragmentation Statistics
// ============================================================================

/// Fragmentation statistics for memory analysis
///
/// Provides insights into allocator memory fragmentation, useful for
/// monitoring and optimization.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FragmentationStats {
    /// Total free memory across all fragments (bytes)
    pub total_free: usize,

    /// Size of the largest contiguous free block (bytes)
    pub largest_block: usize,

    /// Number of distinct free fragments
    pub fragment_count: usize,

    /// External fragmentation ratio (0-100)
    ///
    /// Calculated as: `100 * (1 - largest_block / total_free)`
    /// High values indicate poor memory utilization.
    pub fragmentation_percent: u8,
}

// FragmentationStats uses derive(Default) — all numeric fields default to 0

impl FragmentationStats {
    /// Calculate fragmentation percentage from free space metrics
    pub fn calculate(total_free: usize, largest_block: usize, fragment_count: usize) -> Self {
        let fragmentation_percent = if total_free > 0 {
            let ratio = 1.0 - (largest_block as f64 / total_free as f64);
            (ratio * 100.0).clamp(0.0, 100.0) as u8
        } else {
            0
        };

        Self {
            total_free,
            largest_block,
            fragment_count,
            fragmentation_percent,
        }
    }

    /// Check if fragmentation is concerning (>50%)
    #[inline]
    pub fn is_fragmented(&self) -> bool {
        self.fragmentation_percent > 50
    }
}

impl fmt::Display for FragmentationStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "FragmentationStats {{ total_free: {} bytes, largest_block: {} bytes, \
             fragments: {}, fragmentation: {}% }}",
            self.total_free, self.largest_block, self.fragment_count, self.fragmentation_percent
        )
    }
}

// ============================================================================
// Sealed Internal Allocator Trait
// ============================================================================

/// Internal allocator contract with guaranteed invariants
///
/// This trait is **sealed** and cannot be implemented outside `nebula-memory`.
/// It provides internal methods that require knowledge of allocator invariants
/// and implementation details.
///
/// # Sealed Trait
///
/// This trait is sealed using the [sealed trait pattern]. External crates can:
/// - ✅ Use it as a trait bound
/// - ✅ Call its methods on types that implement it
/// - ❌ Implement it for their own types
///
/// [sealed trait pattern]: https://rust-lang.github.io/api-guidelines/future-proofing.html#c-sealed
///
/// # Safety
///
/// While the trait itself is safe to use, implementations must maintain
/// strict invariants:
/// - Checkpoints must be valid for the allocator that created them
/// - Restoring a checkpoint invalidates all allocations made after it
/// - Internal operations must not violate allocator safety contracts
///
/// # Examples
///
/// ```rust
/// use nebula_memory::allocator::{BumpAllocator, sealed::AllocatorInternal};
/// use nebula_memory::allocator::AllocResult;
///
/// fn analyze_allocator<A: AllocatorInternal>(alloc: &A) -> AllocResult<()> {
///     // Can use internal methods
///     let checkpoint = alloc.internal_checkpoint();
///     let stats = alloc.internal_fragmentation();
///
///     println!("Checkpoint: {:?}", checkpoint);
///     println!("Fragmentation: {}", stats);
///
///     Ok(())
/// }
/// ```
pub trait AllocatorInternal: private::Sealed + Allocator {
    /// Get internal checkpoint for state restoration
    ///
    /// Creates a snapshot of the allocator's current state that can be used
    /// to restore to this point later. Used by arena reset and pool management.
    ///
    /// # Implementation Notes
    ///
    /// - Checkpoint must be lightweight (typically just offset + ID)
    /// - Must be valid for the lifetime of the allocator
    /// - Multiple checkpoints can coexist
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use nebula_memory::allocator::{BumpAllocator, sealed::AllocatorInternal};
    /// # use nebula_memory::allocator::bump::BumpConfig;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut alloc = BumpAllocator::with_config(1024, BumpConfig::default())?;
    /// let checkpoint = alloc.internal_checkpoint();
    ///
    /// // Use checkpoint later for restoration
    /// # Ok(())
    /// # }
    /// ```
    fn internal_checkpoint(&self) -> InternalCheckpoint;

    /// Restore allocator to previous checkpoint
    ///
    /// Rewinds the allocator state to a previously captured checkpoint.
    /// All allocations made after the checkpoint are invalidated.
    ///
    /// # Safety
    ///
    /// Callers must ensure:
    /// - Checkpoint was created by this allocator instance
    /// - No allocations from after the checkpoint are still in use
    /// - Checkpoint is not stale (allocator hasn't been reset since)
    ///
    /// Violating these requirements results in undefined behavior.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Checkpoint is from a different allocator
    /// - Checkpoint generation doesn't match (stale)
    /// - Restoration would violate allocator invariants
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use nebula_memory::allocator::{Allocator, BumpAllocator, sealed::AllocatorInternal};
    /// # use nebula_memory::allocator::bump::BumpConfig;
    /// # use core::alloc::Layout;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut alloc = BumpAllocator::with_config(1024, BumpConfig::default())?;
    /// let checkpoint = alloc.internal_checkpoint();
    ///
    /// // Make some allocations
    /// unsafe {
    ///     let _ptr = alloc.allocate(Layout::new::<u64>())?;
    /// }
    ///
    /// // Restore to checkpoint (invalidates _ptr!)
    /// unsafe {
    ///     alloc.internal_restore(checkpoint)?;
    /// }
    /// # Ok(())
    /// # }
    /// ```
    unsafe fn internal_restore(&mut self, checkpoint: InternalCheckpoint) -> AllocResult<()>;

    /// Get fragmentation statistics
    ///
    /// Analyzes the allocator's memory layout and returns statistics about
    /// fragmentation. Used for monitoring and optimization decisions.
    ///
    /// # Default Implementation
    ///
    /// Returns zero fragmentation by default. Allocators that track free lists
    /// or have non-trivial fragmentation should override this.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use nebula_memory::allocator::{BumpAllocator, sealed::AllocatorInternal};
    /// # use nebula_memory::allocator::bump::BumpConfig;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let alloc = BumpAllocator::with_config(1024, BumpConfig::default())?;
    /// let stats = alloc.internal_fragmentation();
    ///
    /// if stats.is_fragmented() {
    ///     println!("Allocator is fragmented: {}", stats);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    fn internal_fragmentation(&self) -> FragmentationStats {
        FragmentationStats::default()
    }

    /// Validate internal invariants (debug builds only)
    ///
    /// Performs comprehensive validation of allocator internal state.
    /// Only compiled in debug builds for performance.
    ///
    /// # Default Implementation
    ///
    /// Returns `Ok(())` by default. Allocators with complex invariants
    /// should override to add validation logic.
    ///
    /// # Errors
    ///
    /// Returns `Err` with description if invariants are violated.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use nebula_memory::allocator::{BumpAllocator, sealed::AllocatorInternal};
    /// # use nebula_memory::allocator::bump::BumpConfig;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let alloc = BumpAllocator::with_config(1024, BumpConfig::default())?;
    ///
    /// #[cfg(debug_assertions)]
    /// {
    ///     alloc.internal_validate()
    ///         .expect("allocator invariants violated");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(debug_assertions)]
    fn internal_validate(&self) -> Result<(), &'static str> {
        Ok(())
    }

    /// Get allocator type name for debugging
    ///
    /// Returns a human-readable name for the allocator type.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use nebula_memory::allocator::{BumpAllocator, sealed::AllocatorInternal};
    /// # use nebula_memory::allocator::bump::BumpConfig;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let alloc = BumpAllocator::with_config(1024, BumpConfig::default())?;
    /// assert_eq!(alloc.internal_type_name(), "BumpAllocator");
    /// # Ok(())
    /// # }
    /// ```
    fn internal_type_name(&self) -> &'static str {
        core::any::type_name::<Self>()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkpoint_creation() {
        let checkpoint = InternalCheckpoint::new(100, 1, 0);
        assert_eq!(checkpoint.offset, 100);
        assert_eq!(checkpoint.chunk_id, 1);
        assert_eq!(checkpoint.generation, 0);
    }

    #[test]
    fn fragmentation_calculation() {
        let stats = FragmentationStats::calculate(1000, 500, 5);
        assert_eq!(stats.total_free, 1000);
        assert_eq!(stats.largest_block, 500);
        assert_eq!(stats.fragment_count, 5);
        assert_eq!(stats.fragmentation_percent, 50);

        assert!(!stats.is_fragmented()); // Exactly 50%, not >50%
    }

    #[test]
    fn high_fragmentation_detection() {
        let stats = FragmentationStats::calculate(1000, 100, 10);
        assert_eq!(stats.fragmentation_percent, 90);
        assert!(stats.is_fragmented());
    }

    #[test]
    fn zero_fragmentation() {
        let stats = FragmentationStats::default();
        assert_eq!(stats.fragmentation_percent, 0);
        assert!(!stats.is_fragmented());
    }

    #[test]
    fn fragmentation_display() {
        let stats = FragmentationStats::calculate(2048, 512, 8);
        let display = format!("{stats}");
        assert!(display.contains("2048 bytes"));
        assert!(display.contains("512 bytes"));
        assert!(display.contains("75%"));
    }
}

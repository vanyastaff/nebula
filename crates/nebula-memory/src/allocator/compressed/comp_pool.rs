//! Compressed pool allocator with inactive object compression
//!
//! # Safety
//!
//! This module wraps PoolAllocator with optional compression:
//! - Forwards all allocation operations to underlying pool allocator
//! - Tracks active allocations to identify compression candidates
//! - Compression happens on inactive objects (doesn't affect allocator safety)
//! - All safety contracts preserved through delegation to PoolAllocator
//!
//! ## Safety Contracts
//!
//! - allocate/deallocate: Forwarded to PoolAllocator (inherits its contracts)
//! - Active tracking: Uses HashMap protected by Mutex (thread-safe)
//! - Compression is transparent (doesn't change memory safety semantics)

use std::alloc::Layout;
use std::collections::HashMap;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex};

use crate::allocator::pool::{PoolAllocator, PoolConfig};
use crate::allocator::{AllocError, AllocResult, Allocator};

#[cfg(feature = "compression")]
use super::{CompressedBuffer, CompressionStats, CompressionStrategy};

/// Compressed pool allocator
///
/// Maintains a pool of allocations, compressing inactive objects
/// to save memory.
#[cfg(feature = "compression")]
pub struct CompressedPool {
    /// Underlying pool allocator
    pool: PoolAllocator,

    /// Compression strategy
    strategy: CompressionStrategy,

    /// Compression statistics
    stats: Arc<CompressionStats>,

    /// Compressed inactive objects
    compressed: Arc<Mutex<HashMap<usize, CompressedBuffer>>>,

    /// Active allocations (not compressed)
    active: Arc<Mutex<HashMap<usize, usize>>>, // ptr -> size
}

#[cfg(feature = "compression")]
impl CompressedPool {
    /// Create new compressed pool allocator
    pub fn new(block_size: usize, block_align: usize) -> AllocResult<Self> {
        const DEFAULT_BLOCK_COUNT: usize = 1024;
        Ok(Self {
            pool: PoolAllocator::with_config(
                block_size,
                block_align,
                DEFAULT_BLOCK_COUNT,
                PoolConfig::default(),
            )?,
            strategy: CompressionStrategy::default(),
            stats: Arc::new(CompressionStats::new()),
            compressed: Arc::new(Mutex::new(HashMap::new())),
            active: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Create with custom strategy
    pub fn with_strategy(
        block_size: usize,
        block_align: usize,
        strategy: CompressionStrategy,
    ) -> AllocResult<Self> {
        const DEFAULT_BLOCK_COUNT: usize = 1024;
        Ok(Self {
            pool: PoolAllocator::with_config(
                block_size,
                block_align,
                DEFAULT_BLOCK_COUNT,
                PoolConfig::default(),
            )?,
            strategy,
            stats: Arc::new(CompressionStats::new()),
            compressed: Arc::new(Mutex::new(HashMap::new())),
            active: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Get compression statistics
    pub fn stats(&self) -> &CompressionStats {
        &self.stats
    }

    /// Get compression strategy
    pub fn strategy(&self) -> CompressionStrategy {
        self.strategy
    }

    /// Set compression strategy
    pub fn set_strategy(&mut self, strategy: CompressionStrategy) {
        self.strategy = strategy;
    }

    /// Get number of compressed objects
    pub fn compressed_count(&self) -> usize {
        self.compressed.lock().map(|c| c.len()).unwrap_or(0)
    }

    /// Get number of active objects
    pub fn active_count(&self) -> usize {
        self.active.lock().map(|a| a.len()).unwrap_or(0)
    }

    /// Get total space saved
    pub fn space_saved(&self) -> u64 {
        self.stats.space_saved()
    }

    /// Compress inactive objects
    pub fn compress_inactive(&self) {
        // In a real implementation:
        // 1. Find inactive objects in pool
        // 2. Compress their data
        // 3. Free original memory
        // 4. Store compressed buffers
    }

    /// Decompress and restore object
    #[cfg(feature = "compression")]
    pub fn decompress_object(&self, key: usize) -> Option<Vec<u8>> {
        let mut compressed = self.compressed.lock().ok()?;
        let buffer = compressed.remove(&key)?;

        let start = std::time::Instant::now();
        let decompressed = buffer.decompress().ok()?;
        let duration = start.elapsed();

        self.stats
            .record_decompression(decompressed.len(), duration);
        Some(decompressed)
    }
}

#[cfg(feature = "compression")]
// SAFETY: CompressedPool forwards all operations to PoolAllocator.
// - All safety contracts preserved through delegation
// - Active tracking is orthogonal to memory safety (uses thread-safe HashMap)
// - allocate/deallocate forward to pool (inherits PoolAllocator contracts)
unsafe impl Allocator for CompressedPool {
    unsafe fn allocate(&self, layout: Layout) -> AllocResult<NonNull<[u8]>> {
        // SAFETY: Forwarding to pool.allocate.
        // - layout is valid (caller contract)
        // - pool.allocate upholds Allocator trait contract
        let ptr = self.pool.allocate(layout)?;

        // Track active allocation
        if let Ok(mut active) = self.active.lock() {
            // SAFETY: Converting pointer to usize for HashMap key.
            // - ptr is NonNull (from successful allocation)
            // - usize is purely for tracking (doesn't affect memory safety)
            active.insert(ptr.as_ptr() as *mut u8 as usize, layout.size());
        }

        Ok(ptr)
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        // Remove from active
        if let Ok(mut active) = self.active.lock() {
            // SAFETY: Converting pointer to usize for HashMap key.
            // - ptr is NonNull (caller contract)
            // - usize is purely for tracking (doesn't affect deallocation safety)
            active.remove(&(ptr.as_ptr() as usize));
        }

        // SAFETY: Forwarding to pool.deallocate.
        // - ptr/layout match allocation (caller contract)
        // - pool.deallocate upholds Allocator trait contract
        self.pool.deallocate(ptr, layout)
    }
}

// Placeholder when compression feature is disabled
#[cfg(not(feature = "compression"))]
pub struct CompressedPool {
    pool: PoolAllocator,
}

#[cfg(not(feature = "compression"))]
impl CompressedPool {
    pub fn new(block_size: usize, block_align: usize) -> AllocResult<Self> {
        Ok(Self {
            pool: PoolAllocator::with_config(block_size, block_align, PoolConfig::default())?,
        })
    }

    pub fn active_count(&self) -> usize {
        0
    }

    pub fn compressed_count(&self) -> usize {
        0
    }
}

#[cfg(not(feature = "compression"))]
// SAFETY: CompressedPool (no compression) forwards to PoolAllocator.
// - Placeholder implementation when compression feature disabled
// - All safety contracts preserved through delegation
unsafe impl Allocator for CompressedPool {
    unsafe fn allocate(&self, layout: Layout) -> AllocResult<NonNull<u8>> {
        // SAFETY: Forwarding to pool.allocate.
        // - layout is valid (caller contract)
        // - pool.allocate upholds Allocator trait contract
        self.pool.allocate(layout)
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        // SAFETY: Forwarding to pool.deallocate.
        // - ptr/layout match allocation (caller contract)
        // - pool.deallocate upholds Allocator trait contract
        self.pool.deallocate(ptr, layout)
    }
}

#[cfg(all(test, feature = "compression"))]
mod tests {
    use super::*;

    #[test]
    fn test_compressed_pool_basic() {
        let alloc = CompressedPool::new(128, 8).unwrap();

        // SAFETY: Test allocate/deallocate cycle.
        // - layout is valid
        // - ptr/layout match for deallocation
        // - No outstanding references when deallocating
        unsafe {
            let layout = Layout::from_size_align(128, 8).unwrap();
            let ptr = alloc.allocate(layout).unwrap();

            assert_eq!(alloc.active_count(), 1);
            assert_eq!(alloc.compressed_count(), 0);

            alloc.deallocate(ptr, layout);

            assert_eq!(alloc.active_count(), 0);
        }
    }

    #[test]
    fn test_compression_strategy() {
        let mut alloc = CompressedPool::new(128, 8).unwrap();

        alloc.set_strategy(CompressionStrategy::Always);
        assert!(matches!(alloc.strategy(), CompressionStrategy::Always));
    }

    #[test]
    fn test_space_saved() {
        let alloc = CompressedPool::new(128, 8).unwrap();
        assert_eq!(alloc.space_saved(), 0);
    }
}

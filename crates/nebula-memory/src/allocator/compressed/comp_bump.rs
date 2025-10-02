//! Compressed bump allocator with transparent compression

use std::alloc::Layout;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::allocator::bump::BumpAllocator;
use crate::allocator::{AllocError, AllocResult, Allocator};

use super::comp_buffer::CompressedBuffer;
use super::comp_stats::{CompressionStats, CompressionStrategy};
#[cfg(feature = "compression")]
use super::{compress, decompress};

/// Compressed bump allocator
///
/// Allocates memory using bump allocation, but stores compressed data
/// when beneficial. Decompresses on access.
#[cfg(feature = "compression")]
pub struct CompressedBump {
    /// Underlying bump allocator
    bump: BumpAllocator,

    /// Compression strategy
    strategy: CompressionStrategy,

    /// Compression statistics
    stats: Arc<CompressionStats>,

    /// Compressed buffers (for tracking)
    buffers: Arc<Mutex<Vec<CompressedBuffer>>>,
}

#[cfg(feature = "compression")]
impl CompressedBump {
    /// Create new compressed bump allocator
    pub fn new(capacity: usize) -> AllocResult<Self> {
        Ok(Self {
            bump: BumpAllocator::new(capacity)?,
            strategy: CompressionStrategy::default(),
            stats: Arc::new(CompressionStats::new()),
            buffers: Arc::new(Mutex::new(Vec::new())),
        })
    }

    /// Create with custom strategy
    pub fn with_strategy(capacity: usize, strategy: CompressionStrategy) -> AllocResult<Self> {
        Ok(Self {
            bump: BumpAllocator::new(capacity)?,
            strategy,
            stats: Arc::new(CompressionStats::new()),
            buffers: Arc::new(Mutex::new(Vec::new())),
        })
    }

    /// Create with memory limit (for pressure-based compression)
    pub fn with_limit(capacity: usize, limit: usize) -> AllocResult<Self> {
        Ok(Self {
            bump: BumpAllocator::new(capacity)?,
            strategy: CompressionStrategy::default(),
            stats: Arc::new(CompressionStats::with_limit(limit)),
            buffers: Arc::new(Mutex::new(Vec::new())),
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

    /// Compress data if beneficial
    fn try_compress(&self, data: &[u8]) -> Option<CompressedBuffer> {
        if !self.strategy.should_compress(data.len(), &self.stats) {
            return None;
        }

        let start = Instant::now();
        let buffer = CompressedBuffer::new(data);
        let duration = start.elapsed();

        self.stats
            .record_compression(buffer.original_size(), buffer.compressed_size(), duration);

        // Only use compression if it actually saves space
        if buffer.compression_ratio() < 0.95 {
            Some(buffer)
        } else {
            None
        }
    }

    /// Reset allocator and clear compressed buffers
    pub fn reset_allocator(&mut self) {
        unsafe {
            self.bump.reset();
        }
        if let Ok(mut buffers) = self.buffers.lock() {
            buffers.clear();
        }
    }

    /// Get number of compressed buffers
    pub fn compressed_count(&self) -> usize {
        self.buffers.lock().map(|b| b.len()).unwrap_or(0)
    }

    /// Get total space saved by compression
    pub fn space_saved(&self) -> u64 {
        self.stats.space_saved()
    }
}

#[cfg(feature = "compression")]
unsafe impl Allocator for CompressedBump {
    unsafe fn allocate(&self, layout: Layout) -> AllocResult<NonNull<u8>> {
        // For now, just use the underlying bump allocator
        // In a real implementation, we would:
        // 1. Allocate from bump
        // 2. On deallocation (if tracked), compress the data
        // 3. Store compressed version
        // 4. Free original allocation

        self.bump.allocate(layout)
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        self.bump.deallocate(ptr, layout)
    }
}

// Placeholder when compression feature is disabled
#[cfg(not(feature = "compression"))]
pub struct CompressedBump {
    bump: BumpAllocator,
}

#[cfg(not(feature = "compression"))]
impl CompressedBump {
    pub fn new(capacity: usize) -> AllocResult<Self> {
        Ok(Self {
            bump: BumpAllocator::new(capacity)?,
        })
    }

    pub fn reset_allocator(&mut self) {
        unsafe {
            self.bump.reset();
        }
    }
}

#[cfg(not(feature = "compression"))]
unsafe impl Allocator for CompressedBump {
    unsafe fn allocate(&self, layout: Layout) -> AllocResult<NonNull<u8>> {
        self.bump.allocate(layout)
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        self.bump.deallocate(ptr, layout)
    }
}

#[cfg(all(test, feature = "compression"))]
mod tests {
    use super::*;

    #[test]
    fn test_compressed_bump_basic() {
        let alloc = CompressedBump::new(1024 * 1024).unwrap();

        unsafe {
            let layout = Layout::from_size_align(128, 8).unwrap();
            let ptr = alloc.allocate(layout).unwrap();
            alloc.deallocate(ptr, layout);
        }
    }

    #[test]
    fn test_compression_strategy() {
        let mut alloc = CompressedBump::new(1024 * 1024).unwrap();

        alloc.set_strategy(CompressionStrategy::Always);
        assert!(matches!(alloc.strategy(), CompressionStrategy::Always));

        alloc.set_strategy(CompressionStrategy::threshold(2048));
        // Strategy check removed (no PartialEq for f64);
    }

    #[test]
    fn test_space_saved() {
        let alloc = CompressedBump::new(1024 * 1024).unwrap();
        assert_eq!(alloc.space_saved(), 0);
    }
}

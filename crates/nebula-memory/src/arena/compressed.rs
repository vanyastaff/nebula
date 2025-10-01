//! Compressed arena implementation for memory-efficient storage

use std::alloc::{alloc, dealloc, Layout};
use std::cell::{Cell, RefCell};
use std::ptr::NonNull;

#[cfg(feature = "compression")]
use lz4_flex::{compress_prepend_size, decompress_size_prepended};

use super::ArenaStats;
use crate::core::error::MemoryError;
use crate::utils::align_up;

/// Compression level for the arena
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionLevel {
    None,
    Fast,
    Default,
    Best,
}

impl CompressionLevel {
    #[cfg(feature = "compression")]
    fn to_lz4_level(&self) -> i32 {
        match self {
            Self::None => 0,
            Self::Fast => 1,
            Self::Default => 0, // LZ4 default
            Self::Best => 12,
        }
    }
}

/// Compressed block storage
struct CompressedBlock {
    data: Vec<u8>,
    uncompressed_size: usize,
}

/// Memory block in the arena
struct Block {
    ptr: NonNull<u8>,
    capacity: usize,
    used: usize,
}

impl Block {
    fn new(size: usize) -> Result<Self, MemoryError> {
        let layout = Layout::from_size_align(size, 1).map_err(|_| MemoryError::invalid_layout())?;

        let ptr = unsafe { alloc(layout) };
        let ptr = NonNull::new(ptr).ok_or(MemoryError::allocation_failed())?;

        Ok(Self { ptr, capacity: size, used: 0 })
    }

    #[inline]
    fn available(&self) -> usize {
        self.capacity - self.used
    }

    #[inline]
    fn as_slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.used) }
    }
}

impl Drop for Block {
    fn drop(&mut self) {
        unsafe {
            dealloc(self.ptr.as_ptr(), Layout::from_size_align_unchecked(self.capacity, 1));
        }
    }
}

/// Compressed arena allocator
pub struct CompressedArena {
    active_block: RefCell<Option<Block>>,
    compressed_blocks: RefCell<Vec<CompressedBlock>>,
    block_size: usize,
    compression_level: CompressionLevel,
    compression_threshold: usize,
    stats: ArenaStats,
    compressed_bytes: Cell<usize>,
}

impl CompressedArena {
    /// Creates a new compressed arena
    pub fn new(block_size: usize, compression_level: CompressionLevel) -> Self {
        Self {
            active_block: RefCell::new(None),
            compressed_blocks: RefCell::new(Vec::new()),
            block_size,
            compression_level,
            compression_threshold: block_size / 2,
            stats: ArenaStats::new(),
            compressed_bytes: Cell::new(0),
        }
    }

    /// Creates arena with default settings (64KB blocks, default compression)
    pub fn default() -> Self {
        Self::new(64 * 1024, CompressionLevel::Default)
    }

    /// Allocates bytes with alignment
    pub fn alloc_bytes(&self, size: usize, align: usize) -> Result<*mut u8, MemoryError> {
        if size > self.block_size {
            return Err(MemoryError::allocation_too_large(0));
        }

        self.maybe_compress_active()?;
        self.ensure_active_block()?;

        let mut active = self.active_block.borrow_mut();
        let block = active.as_mut().unwrap();

        let aligned_pos = align_up(block.used, align);
        let needed = aligned_pos - block.used + size;

        if needed > block.available() {
            drop(active);
            self.compress_active()?;
            return self.alloc_bytes(size, align);
        }

        let ptr = unsafe { block.ptr.as_ptr().add(aligned_pos) };
        block.used = aligned_pos + size;
        self.stats.record_allocation(size, 0);

        Ok(ptr)
    }

    /// Allocates and initializes a value
    pub fn alloc<T>(&self, value: T) -> Result<&mut T, MemoryError> {
        let ptr = self.alloc_bytes(std::mem::size_of::<T>(), std::mem::align_of::<T>())? as *mut T;

        unsafe {
            ptr.write(value);
            Ok(&mut *ptr)
        }
    }

    /// Compresses the active block if it meets the threshold
    fn maybe_compress_active(&self) -> Result<(), MemoryError> {
        let active = self.active_block.borrow();
        if let Some(block) = active.as_ref() {
            if block.used >= self.compression_threshold {
                drop(active);
                self.compress_active()?;
            }
        }
        Ok(())
    }

    /// Compresses the current active block
    fn compress_active(&self) -> Result<(), MemoryError> {
        let block = self.active_block.borrow_mut().take();
        if let Some(block) = block {
            self.compress_block(block)
        } else {
            Ok(())
        }
    }

    /// Compresses a block and stores it
    fn compress_block(&self, block: Block) -> Result<(), MemoryError> {
        let uncompressed = block.as_slice();
        let compressed = if self.compression_level != CompressionLevel::None {
            #[cfg(feature = "compression")]
            {
                compress_prepend_size(uncompressed)
            }
            #[cfg(not(feature = "compression"))]
            {
                uncompressed.to_vec()
            }
        } else {
            uncompressed.to_vec()
        };

        self.compressed_bytes.set(self.compressed_bytes.get() + compressed.len());

        self.compressed_blocks
            .borrow_mut()
            .push(CompressedBlock { data: compressed, uncompressed_size: block.used });

        Ok(())
    }

    /// Ensures there's an active block available
    fn ensure_active_block(&self) -> Result<(), MemoryError> {
        if self.active_block.borrow().is_none() {
            let block = Block::new(self.block_size)?;
            self.stats.record_chunk_allocation(self.block_size);
            *self.active_block.borrow_mut() = Some(block);
        }
        Ok(())
    }

    /// Decompresses a block by index
    pub fn decompress_block(&self, index: usize) -> Result<Vec<u8>, MemoryError> {
        let blocks = self.compressed_blocks.borrow();
        let block = blocks.get(index).ok_or(MemoryError::invalid_index(0, 0))?;

        #[cfg(feature = "compression")]
        if self.compression_level != CompressionLevel::None {
            return decompress_size_prepended(&block.data)
                .map_err(|_| MemoryError::decompression_failed("decompression error"));
        }

        Ok(block.data.clone())
    }

    /// Gets compression statistics
    pub fn compression_stats(&self) -> CompressionStats {
        CompressionStats {
            uncompressed_bytes: self.stats.bytes_used(),
            compressed_bytes: self.compressed_bytes.get(),
            compressed_blocks: self.compressed_blocks.borrow().len(),
        }
    }

    /// Flushes active block to compressed storage
    pub fn flush(&self) -> Result<(), MemoryError> {
        self.compress_active()
    }

    /// Resets the arena
    pub fn reset(&mut self) {
        *self.active_block.borrow_mut() = None;
        self.compressed_blocks.borrow_mut().clear();
        self.compressed_bytes.set(0);
        self.stats.record_reset(0);
    }

    /// Gets arena statistics
    pub fn stats(&self) -> &ArenaStats {
        &self.stats
    }
}

/// Compression statistics
#[derive(Debug, Clone)]
pub struct CompressionStats {
    pub uncompressed_bytes: usize,
    pub compressed_bytes: usize,
    pub compressed_blocks: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_allocation() {
        let arena = CompressedArena::default();
        let value = arena.alloc(42u32).unwrap();
        assert_eq!(*value, 42);
    }

    #[test]
    fn block_compression() {
        let arena = CompressedArena::new(100, CompressionLevel::Fast);

        for i in 0..20 {
            let _ = arena.alloc(i as u64).unwrap();
        }

        assert!(arena.compression_stats().compressed_blocks > 0);
    }

    #[test]
    fn large_allocation_fails() {
        let arena = CompressedArena::new(100, CompressionLevel::None);
        let result = arena.alloc(vec![0u8; 200]);
        assert!(matches!(result, Err(MemoryError::allocation_too_large(0))));
    }

    #[test]
    fn reset_clears_arena() {
        let mut arena = CompressedArena::default();
        let _ = arena.alloc(100u32).unwrap();
        arena.flush().unwrap();

        arena.reset();
        assert_eq!(arena.compression_stats().compressed_blocks, 0);
    }
}

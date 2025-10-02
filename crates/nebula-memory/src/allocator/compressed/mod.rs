//!
//! Compressed allocators with transparent compression/decompression
//! Provides allocators that automatically compress allocated data to save memory.
//! Uses LZ4 compression for fast compression/decompression with good ratios.

use crate::allocator::{AllocError, AllocResult, Allocator};
#[cfg(feature = "compression")]
use lz4_flex::{compress_prepend_size, decompress_size_prepended};
use std::alloc::Layout;
use std::ptr::NonNull;

mod comp_buffer;
mod comp_bump;
mod comp_pool;
mod comp_stats;

pub use self::comp_buffer::CompressedBuffer;
pub use self::comp_bump::CompressedBump;
pub use self::comp_pool::CompressedPool;
pub use self::comp_stats::{CompressionStats, CompressionStrategy};
/// Compresses data using LZ4
#[cfg(feature = "compression")]
pub fn compress(data: &[u8]) -> Vec<u8> {
    compress_prepend_size(data)
}
/// Decompresses LZ4 data
#[cfg(feature = "compression")]
pub fn decompress(data: &[u8]) -> Result<Vec<u8>, std::io::Error> {
    decompress_size_prepended(data)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}
/// Compression threshold - minimum size to compress
pub const DEFAULT_COMPRESSION_THRESHOLD: usize = 1024; // 1KB
/// Maximum compression ratio before giving up
pub const MAX_COMPRESSION_RATIO: f64 = 0.95; // Don't compress if saves less than 5%
/// Checks if data is worth compressing
pub fn should_compress(size: usize, threshold: usize) -> bool {
    size >= threshold
}
/// Estimates compressed size (pessimistic)
pub fn estimate_compressed_size(size: usize) -> usize {
    // LZ4 worst case: original size + (original_size / 255) + 16
    size + (size / 255) + 16
}
#[cfg(all(test, feature = "compression"))]
mod tests {
    use super::*;
    #[test]
    fn test_compress_decompress() {
        let data = b"Hello, World! This is a test of compression.";
        let compressed = compress(data);
        let decompressed = decompress(&compressed).unwrap();
        assert_eq!(data, decompressed.as_slice());
        assert!(compressed.len() < data.len()); // Should compress
    }
    #[test]
    fn test_should_compress_threshold() {
        assert!(!should_compress(512, DEFAULT_COMPRESSION_THRESHOLD));
        assert!(should_compress(2048, DEFAULT_COMPRESSION_THRESHOLD));
    }
    #[test]
    fn test_estimate_compressed_size() {
        let size = 10000;
        let estimate = estimate_compressed_size(size);
        assert!(estimate > size); // Worst case estimate
        assert!(estimate < size + size / 10); // But not too pessimistic
    }
}

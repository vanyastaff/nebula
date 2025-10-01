//! Compressed buffer for storing compressed data with metadata

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(feature = "compression")]
use super::{compress, decompress};

/// Compressed buffer with metadata
#[derive(Clone)]
pub struct CompressedBuffer {
    /// Compressed data
    data: Arc<Vec<u8>>,

    /// Original (uncompressed) size
    original_size: usize,

    /// Compressed size
    compressed_size: usize,

    /// Number of decompression operations
    decompress_count: Arc<AtomicUsize>,
}

impl CompressedBuffer {
    /// Create new compressed buffer from data
    #[cfg(feature = "compression")]
    pub fn new(data: &[u8]) -> Self {
        let compressed = compress(data);
        let compressed_size = compressed.len();

        Self {
            data: Arc::new(compressed),
            original_size: data.len(),
            compressed_size,
            decompress_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Create from already compressed data
    pub fn from_compressed(compressed: Vec<u8>, original_size: usize) -> Self {
        let compressed_size = compressed.len();

        Self {
            data: Arc::new(compressed),
            original_size,
            compressed_size,
            decompress_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Decompress and return data
    #[cfg(feature = "compression")]
    pub fn decompress(&self) -> Result<Vec<u8>, std::io::Error> {
        self.decompress_count.fetch_add(1, Ordering::Relaxed);
        decompress(&self.data)
    }

    /// Get original (uncompressed) size
    pub fn original_size(&self) -> usize {
        self.original_size
    }

    /// Get compressed size
    pub fn compressed_size(&self) -> usize {
        self.compressed_size
    }

    /// Get compression ratio (compressed / original)
    pub fn compression_ratio(&self) -> f64 {
        self.compressed_size as f64 / self.original_size as f64
    }

    /// Get space saved (bytes)
    pub fn space_saved(&self) -> usize {
        self.original_size.saturating_sub(self.compressed_size)
    }

    /// Get space saved (percentage)
    pub fn space_saved_percent(&self) -> f64 {
        (1.0 - self.compression_ratio()) * 100.0
    }

    /// Get number of decompressions
    pub fn decompress_count(&self) -> usize {
        self.decompress_count.load(Ordering::Relaxed)
    }

    /// Get reference to compressed data
    pub fn compressed_data(&self) -> &[u8] {
        &self.data
    }
}

impl std::fmt::Debug for CompressedBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompressedBuffer")
            .field("original_size", &self.original_size)
            .field("compressed_size", &self.compressed_size)
            .field("ratio", &format!("{:.2}%", self.space_saved_percent()))
            .field("decompress_count", &self.decompress_count())
            .finish()
    }
}

#[cfg(all(test, feature = "compression"))]
mod tests {
    use super::*;

    #[test]
    fn test_compressed_buffer() {
        let data = b"Hello World! ".repeat(100);
        let buffer = CompressedBuffer::new(&data);

        assert_eq!(buffer.original_size(), data.len());
        assert!(buffer.compressed_size() < data.len());
        assert!(buffer.compression_ratio() < 1.0);
        assert!(buffer.space_saved() > 0);

        let decompressed = buffer.decompress().unwrap();
        assert_eq!(decompressed, data);
        assert_eq!(buffer.decompress_count(), 1);
    }

    #[test]
    fn test_space_saved_percent() {
        let data = b"A".repeat(1000);
        let buffer = CompressedBuffer::new(&data);

        let saved_percent = buffer.space_saved_percent();
        assert!(saved_percent > 0.0);
        assert!(saved_percent < 100.0);
    }

    #[test]
    fn test_multiple_decompresses() {
        let data = b"Test data";
        let buffer = CompressedBuffer::new(data);

        buffer.decompress().unwrap();
        buffer.decompress().unwrap();
        buffer.decompress().unwrap();

        assert_eq!(buffer.decompress_count(), 3);
    }
}

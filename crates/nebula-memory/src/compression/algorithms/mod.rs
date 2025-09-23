//! Алгоритмы сжатия данных

#[cfg(feature = "lz4")]
pub mod lz4;
#[cfg(feature = "snappy")]
pub mod snappy;
#[cfg(feature = "zstd")]
pub mod zstd;

// Реэкспорт конкретных алгоритмов сжатия для удобства использования
#[cfg(feature = "lz4")]
pub use lz4::Lz4Compressor;
#[cfg(feature = "snappy")]
pub use snappy::SnappyCompressor;
#[cfg(feature = "zstd")]
pub use zstd::ZstdCompressor;

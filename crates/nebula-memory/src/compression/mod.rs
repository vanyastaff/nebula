//! Модуль для работы со сжатием данных

use std::any::Any;
use std::fmt;
use std::io::{self, Read, Write};

use cfg_if::cfg_if;

// Подмодули
mod algorithms;
// #[cfg(feature = "arena")]
// mod arena;
// #[cfg(feature = "cache")]
// mod cache;
mod custom;
mod stats;

/// Доступные алгоритмы сжатия
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Algorithm {
    /// LZ4 алгоритм сжатия
    Lz4,
    /// Zstandard алгоритм сжатия
    Zstd,
    /// Snappy алгоритм сжатия
    Snappy,
    /// Пользовательский алгоритм сжатия
    #[cfg(feature = "custom")]
    Custom(&'static str),
}

impl Default for Algorithm {
    fn default() -> Self {
        cfg_if! {
            if #[cfg(feature = "zstd")] {
                Algorithm::Zstd
            } else if #[cfg(feature = "lz4")] {
                Algorithm::Lz4
            } else if #[cfg(feature = "snappy")] {
                Algorithm::Snappy
            } else {
                compile_error!("At least one compression algorithm must be enabled")
            }
        }
    }
}

/// Общий трейт для алгоритмов сжатия
pub trait CompressionAlgorithm: Send + Sync + fmt::Debug {
    /// Сжимает данные
    fn compress(&self, data: &[u8]) -> io::Result<Vec<u8>>;

    /// Распаковывает данные
    fn decompress(&self, data: &[u8]) -> io::Result<Vec<u8>>;

    /// Возвращает рекомендуемый размер блока для этого алгоритма
    fn recommended_block_size(&self) -> usize;

    /// Предоставляет доступ к типу для даункаста
    fn as_any(&self) -> &dyn Any;
}

/// Трейт для потоковых операций сжатия
pub trait StreamingCompression: CompressionAlgorithm {
    /// Сжимает данные в потоковом режиме
    fn compress_stream<R: Read, W: Write>(&self, reader: R, writer: W) -> io::Result<()>;

    /// Распаковывает данные в потоковом режиме
    fn decompress_stream<R: Read, W: Write>(&self, reader: R, writer: W) -> io::Result<()>;
}

// Реэкспорты

// #[cfg(feature = "arena")]
// pub use self::arena::CompressedArena;
// #[cfg(feature = "cache")]
// pub use self::cache::CompressedCache;
#[cfg(feature = "custom-compression")]
pub use custom::{get_custom_compressor, register_compressor};

// Алгоритмы сжатия
#[cfg(feature = "lz4")]
pub use self::algorithms::lz4::Lz4Compressor;
#[cfg(feature = "snappy")]
pub use self::algorithms::snappy::SnappyCompressor;
#[cfg(feature = "zstd")]
pub use self::algorithms::zstd::ZstdCompressor;
pub use self::stats::{CompressionStats, InstrumentedCompressor};

/// Вспомогательные структуры для отслеживания количества прочитанных/записанных
/// байтов
struct CountingReader<R> {
    reader: R,
    count: usize,
}

impl<R: Read> Read for CountingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let result = self.reader.read(buf)?;
        self.count += result;
        Ok(result)
    }
}

struct CountingWriter<W> {
    writer: W,
    count: usize,
}

impl<W: Write> Write for CountingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let result = self.writer.write(buf)?;
        self.count += result;
        Ok(result)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

/// Создает новый экземпляр компрессора для указанного алгоритма
pub fn new_compressor(algorithm: Algorithm) -> Box<dyn CompressionAlgorithm> {
    match algorithm {
        #[cfg(feature = "lz4")]
        Algorithm::Lz4 => Box::new(algorithms::lz4::Lz4Compressor::new()),

        #[cfg(feature = "zstd")]
        Algorithm::Zstd => Box::new(algorithms::zstd::ZstdCompressor::new()),

        #[cfg(feature = "snappy")]
        Algorithm::Snappy => Box::new(algorithms::snappy::SnappyCompressor::new()),

        #[cfg(feature = "custom-compression")]
        Algorithm::Custom(name) => custom::get_custom_compressor(name)
            .unwrap_or_else(|| panic!("Custom compressor '{}' not registered", name)),

        _ => panic!("Selected compression algorithm is not enabled"),
    }
}

/// Стандартная реализация сжатия потока данных для компрессоров, не
/// поддерживающих потоковую обработку
fn default_compress_stream<R: Read, W: Write>(
    compressor: &dyn CompressionAlgorithm,
    reader: &mut R,
    writer: &mut W,
) -> io::Result<()> {
    const BUFFER_SIZE: usize = 8192;
    let mut buffer = Vec::with_capacity(BUFFER_SIZE);

    loop {
        let mut chunk = vec![0; BUFFER_SIZE];
        let bytes_read = reader.read(&mut chunk)?;
        if bytes_read == 0 {
            break;
        }

        chunk.truncate(bytes_read);
        buffer.extend_from_slice(&chunk);

        // Если накопили достаточно данных или это последний фрагмент, сжимаем и
        // записываем
        if buffer.len() >= compressor.recommended_block_size() || bytes_read < BUFFER_SIZE {
            let compressed = compressor.compress(&buffer)?;
            writer.write_all(&compressed)?;
            buffer.clear();
        }
    }

    // Если остались несжатые данные
    if !buffer.is_empty() {
        let compressed = compressor.compress(&buffer)?;
        writer.write_all(&compressed)?;
    }

    writer.flush()?;
    Ok(())
}

/// Стандартная реализация распаковки потока данных для компрессоров, не
/// поддерживающих потоковую обработку
fn default_decompress_stream<R: Read, W: Write>(
    compressor: &dyn CompressionAlgorithm,
    reader: &mut R,
    writer: &mut W,
) -> io::Result<()> {
    const BUFFER_SIZE: usize = 8192;

    loop {
        let mut chunk = vec![0; BUFFER_SIZE];
        let bytes_read = reader.read(&mut chunk)?;
        if bytes_read == 0 {
            break;
        }

        chunk.truncate(bytes_read);
        let decompressed = compressor.decompress(&chunk)?;
        writer.write_all(&decompressed)?;
    }

    writer.flush()?;
    Ok(())
}

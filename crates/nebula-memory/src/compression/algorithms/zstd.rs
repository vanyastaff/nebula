//! Реализация алгоритма сжатия Zstandard

use std::any::Any;
use std::io::{self, Read, Write};

use zstd::stream::{Decoder, Encoder};

use crate::compression::{CompressionAlgorithm, StreamingCompression};

/// Компрессор, использующий алгоритм Zstandard
#[derive(Debug, Clone)]
pub struct ZstdCompressor {
    level: i32,
}

impl ZstdCompressor {
    /// Создает новый экземпляр Zstandard компрессора со стандартным уровнем
    /// сжатия
    pub fn new() -> Self {
        Self { level: 3 } // Уровень по умолчанию
    }

    /// Создает новый экземпляр Zstandard компрессора с указанным уровнем сжатия
    pub fn with_level(level: i32) -> Self {
        Self { level }
    }
}

impl CompressionAlgorithm for ZstdCompressor {
    fn compress(&self, data: &[u8]) -> io::Result<Vec<u8>> {
        zstd::encode_all(data, self.level)
    }

    fn decompress(&self, data: &[u8]) -> io::Result<Vec<u8>> {
        zstd::decode_all(data)
    }

    fn recommended_block_size(&self) -> usize {
        128 * 1024 // 128KB оптимальный блок для Zstd
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl StreamingCompression for ZstdCompressor {
    fn compress_stream<R: Read, W: Write>(&self, mut reader: R, writer: W) -> io::Result<()> {
        let mut encoder = Encoder::new(writer, self.level)?;
        io::copy(&mut reader, &mut encoder)?;
        encoder.finish()?;
        Ok(())
    }

    fn decompress_stream<R: Read, W: Write>(&self, reader: R, mut writer: W) -> io::Result<()> {
        let mut decoder = Decoder::new(reader)?;
        io::copy(&mut decoder, &mut writer)?;
        Ok(())
    }
}

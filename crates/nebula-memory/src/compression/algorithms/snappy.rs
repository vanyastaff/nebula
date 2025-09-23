//! Реализация алгоритма сжатия Snappy

use std::any::Any;
use std::io::{self, Read, Write};

use snap::read::FrameDecoder;
use snap::write::FrameEncoder;

use crate::compression::{CompressionAlgorithm, StreamingCompression};

/// Компрессор, использующий алгоритм Snappy
#[derive(Debug, Clone)]
pub struct SnappyCompressor;

impl SnappyCompressor {
    /// Создает новый экземпляр Snappy компрессора
    pub fn new() -> Self {
        Self
    }
}

impl CompressionAlgorithm for SnappyCompressor {
    fn compress(&self, data: &[u8]) -> io::Result<Vec<u8>> {
        Ok(snap::raw::Encoder::new().compress_vec(data)?)
    }

    fn decompress(&self, data: &[u8]) -> io::Result<Vec<u8>> {
        snap::raw::Decoder::new()
            .decompress_vec(data)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    fn recommended_block_size(&self) -> usize {
        32 * 1024 // 32KB оптимальный блок для Snappy
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl StreamingCompression for SnappyCompressor {
    fn compress_stream<R: Read, W: Write>(&self, mut reader: R, writer: W) -> io::Result<()> {
        let mut encoder = FrameEncoder::new(writer);
        io::copy(&mut reader, &mut encoder)?;
        Ok(())
    }

    fn decompress_stream<R: Read, W: Write>(&self, reader: R, mut writer: W) -> io::Result<()> {
        let mut decoder = FrameDecoder::new(reader);
        io::copy(&mut decoder, &mut writer)?;
        Ok(())
    }
}

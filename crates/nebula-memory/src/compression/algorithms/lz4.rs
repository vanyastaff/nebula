use std::any::Any;
use std::io::{self, ErrorKind, Read, Write};

use lz4_flex::frame::{FrameDecoder, FrameEncoder, FrameInfo};

use crate::compression::{CompressionAlgorithm, CompressionStats, StreamingCompression};

/// Компрессор LZ4 с настраиваемыми параметрами
#[derive(Debug, Clone)]
pub struct Lz4Compressor {
    /// Уровень сжатия (1-12)
    compression_level: u32,
    /// Использовать потоковый режим
    use_frame_format: bool,
}

impl Lz4Compressor {
    /// Создает новый компрессор с настройками по умолчанию
    pub fn new() -> Self {
        Self {
            compression_level: 4, // средний уровень
            use_frame_format: true,
        }
    }

    /// Устанавливает уровень сжатия (1-12)
    #[must_use = "builder methods must be chained or built"]
    pub fn with_compression_level(mut self, level: u32) -> Self {
        self.compression_level = level.clamp(1, 12);
        self
    }

    /// Включает/выключает потоковый формат
    #[must_use = "builder methods must be chained or built"]
    pub fn use_frame_format(mut self, enabled: bool) -> Self {
        self.use_frame_format = enabled;
        self
    }

    fn create_frame_info(&self) -> FrameInfo {
        FrameInfo::new().block_size(lz4_flex::frame::BlockSize::Max64KB)
    }
}

impl CompressionAlgorithm for Lz4Compressor {
    fn compress(&self, data: &[u8]) -> io::Result<Vec<u8>> {
        if self.use_frame_format {
            let mut buffer = Vec::new();
            let info = self.create_frame_info();
            let mut encoder = FrameEncoder::with_frame_info(info, &mut buffer);
            encoder.write_all(data)?;
            encoder.finish()?;
            Ok(buffer)
        } else {
            Ok(lz4_flex::compress_prepend_size(data))
        }
    }

    fn decompress(&self, data: &[u8]) -> io::Result<Vec<u8>> {
        if self.use_frame_format {
            let mut buffer = Vec::new();
            let mut decoder = FrameDecoder::new(data);
            decoder.read_to_end(&mut buffer)?;
            Ok(buffer)
        } else {
            lz4_flex::decompress_size_prepended(data)
                .map_err(|e| io::Error::new(ErrorKind::InvalidData, e))
        }
    }

    fn recommended_block_size(&self) -> usize {
        64 * 1024 // 64KB оптимальный блок для LZ4
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl StreamingCompression for Lz4Compressor {
    fn compress_stream<R: Read, W: Write>(&self, mut reader: R, writer: W) -> io::Result<()> {
        let info = self.create_frame_info();
        let mut encoder = FrameEncoder::with_frame_info(info, writer);
        io::copy(&mut reader.by_ref(), &mut encoder)?;
        encoder.finish()?;
        Ok(())
    }

    fn decompress_stream<R: Read, W: Write>(&self, reader: R, mut writer: W) -> io::Result<()> {
        let mut decoder = FrameDecoder::new(reader);
        io::copy(&mut decoder, &mut writer.by_ref())?;
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct Lz4Stats {
    bytes_processed: u64,
    compression_time: std::time::Duration,
    decompression_time: std::time::Duration,
}

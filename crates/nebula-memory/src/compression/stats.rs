//! Статистика сжатия данных

/// Статистика сжатия
#[derive(Debug, Default, Clone)]
pub struct CompressionStats {
    pub compressed_bytes: u64,
    pub uncompressed_bytes: u64,
    pub compression_time_ns: u64,
    pub decompression_time_ns: u64,
}

impl CompressionStats {
    /// Коэффициент сжатия (0.0 - 1.0)
    pub fn ratio(&self) -> f64 {
        if self.uncompressed_bytes == 0 {
            0.0
        } else {
            self.compressed_bytes as f64 / self.uncompressed_bytes as f64
        }
    }

    /// Экономия места (0.0 - 1.0)
    pub fn savings(&self) -> f64 {
        1.0 - self.ratio()
    }
}

/// Компрессор с отслеживанием статистики
#[derive(Debug)]
pub struct InstrumentedCompressor {
    inner: Box<dyn super::CompressionAlgorithm>,
    stats: std::sync::Arc<parking_lot::Mutex<CompressionStats>>,
}

impl InstrumentedCompressor {
    pub fn new(algorithm: super::Algorithm) -> Self {
        Self { inner: super::new_compressor(algorithm), stats: Default::default() }
    }

    pub fn stats(&self) -> CompressionStats {
        self.stats.lock().clone()
    }
}

impl super::CompressionAlgorithm for InstrumentedCompressor {
    fn compress(&self, data: &[u8]) -> std::io::Result<Vec<u8>> {
        let start = std::time::Instant::now();
        let result = self.inner.compress(data)?;
        let duration = start.elapsed();

        let mut stats = self.stats.lock();
        stats.compressed_bytes += result.len() as u64;
        stats.uncompressed_bytes += data.len() as u64;
        stats.compression_time_ns += duration.as_nanos() as u64;

        Ok(result)
    }

    fn decompress(&self, data: &[u8]) -> std::io::Result<Vec<u8>> {
        let start = std::time::Instant::now();
        let result = self.inner.decompress(data)?;
        let duration = start.elapsed();

        let mut stats = self.stats.lock();
        stats.compressed_bytes += data.len() as u64;
        stats.uncompressed_bytes += result.len() as u64;
        stats.decompression_time_ns += duration.as_nanos() as u64;

        Ok(result)
    }

    fn recommended_block_size(&self) -> usize {
        self.inner.recommended_block_size()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

// Реализация для потокового сжатия, обернутая в условную компиляцию
// чтобы избежать ошибки с dyn-объектом
#[cfg(feature = "compression")]
impl super::StreamingCompression for InstrumentedCompressor {
    fn compress_stream<R: std::io::Read, W: std::io::Write>(
        &self,
        mut reader: R,
        mut writer: W,
    ) -> std::io::Result<()> {
        // Создаем обертку для подсчета прочитанных и записанных байтов
        let mut counted_reader = super::CountingReader { reader, count: 0 };
        let mut counted_writer = super::CountingWriter { writer, count: 0 };

        let start = std::time::Instant::now();

        // Используем реализацию по умолчанию вместо опасного downcast
        super::default_compress_stream(
            self.inner.as_ref(),
            &mut counted_reader,
            &mut counted_writer,
        )?;

        let duration = start.elapsed();

        let mut stats = self.stats.lock();
        stats.uncompressed_bytes += counted_reader.count as u64;
        stats.compressed_bytes += counted_writer.count as u64;
        stats.compression_time_ns += duration.as_nanos() as u64;

        Ok(())
    }

    fn decompress_stream<R: std::io::Read, W: std::io::Write>(
        &self,
        mut reader: R,
        mut writer: W,
    ) -> std::io::Result<()> {
        // Создаем обертку для подсчета прочитанных и записанных байтов
        let mut counted_reader = super::CountingReader { reader, count: 0 };
        let mut counted_writer = super::CountingWriter { writer, count: 0 };

        let start = std::time::Instant::now();

        // Используем реализацию по умолчанию вместо опасного downcast
        super::default_decompress_stream(
            self.inner.as_ref(),
            &mut counted_reader,
            &mut counted_writer,
        )?;

        let duration = start.elapsed();

        let mut stats = self.stats.lock();
        stats.compressed_bytes += counted_reader.count as u64;
        stats.uncompressed_bytes += counted_writer.count as u64;
        stats.decompression_time_ns += duration.as_nanos() as u64;

        Ok(())
    }
}

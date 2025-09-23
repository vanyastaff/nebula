//! Реализация CompressedArena для сжатия данных в арена-аллокаторе
use std::any::Any;
use std::fmt;
use std::io::{self, Read, Write};
use std::sync::Arc;

use super::{Algorithm, CompressionAlgorithm, StreamingCompression};

#[cfg(feature = "arena")]
mod arena_impl {
    use std::io;
    use std::sync::Arc;
    use std::mem::size_of;
    use std::slice::{from_raw_parts, from_raw_parts_mut};
    use std::sync::atomic::Ordering;
    
    // Вместо использования crate::arena напрямую, определим типы-заглушки,
    // которые будут использоваться только при наличии функции "arena"
    #[cfg(feature = "arena")]
    type Arena = crate::arena::Arena;
    
    #[cfg(feature = "arena")]
    type ArenaAllocator = crate::arena::ArenaAllocator;
    
    use super::super::{Algorithm, CompressionAlgorithm};
    use super::{CompressedArenaStats, CompressedBlock};

    /// Arena с поддержкой автоматического сжатия данных
    pub struct CompressedArena {
        pub(super) inner: Arena,
        pub(super) compressor: Box<dyn CompressionAlgorithm>,
        pub(super) threshold: usize,
        pub(super) stats: Arc<CompressedArenaStats>,
    }

    impl CompressedArena {
        /// Создает новую сжатую арену с указанным алгоритмом сжатия
        pub fn new(size: usize, algorithm: Algorithm) -> Self {
            Self::new_with_threshold(size, algorithm, 4096)
        }
        
        /// Создает новую сжатую арену с указанным порогом сжатия
        pub fn new_with_threshold(size: usize, algorithm: Algorithm, threshold: usize) -> Self {
            let compressor = super::super::new_compressor(algorithm);
            Self {
                inner: Arena::new(size),
                compressor,
                threshold,
                stats: Arc::new(CompressedArenaStats::default()),
            }
        }
        
        /// Сжимает данные и сохраняет их в арене
        pub fn allocate_compressed(&mut self, data: &[u8]) -> io::Result<CompressedBlock> {
            if data.len() < self.threshold {
                // Не сжимаем маленькие данные
                let ptr = self.inner.allocate(data.len())?;
                let slice = unsafe { std::slice::from_raw_parts_mut(ptr as *mut u8, data.len()) };
                slice.copy_from_slice(data);
                
                return Ok(CompressedBlock {
                    ptr,
                    original_len: data.len(),
                    compressed_len: data.len(),
                    is_compressed: false,
                });
            }
            
            let compressed = self.compressor.compress(data)?;
            
            // Обновляем статистику
            self.stats.blocks_compressed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.stats.bytes_before_compression.fetch_add(data.len(), std::sync::atomic::Ordering::Relaxed);
            self.stats.bytes_after_compression.fetch_add(compressed.len(), std::sync::atomic::Ordering::Relaxed);
            
            // Если после сжатия размер увеличился, сохраняем исходные данные
            if compressed.len() >= data.len() {
                let ptr = self.inner.allocate(data.len())?;
                let slice = unsafe { std::slice::from_raw_parts_mut(ptr as *mut u8, data.len()) };
                slice.copy_from_slice(data);
                
                return Ok(CompressedBlock {
                    ptr,
                    original_len: data.len(),
                    compressed_len: data.len(),
                    is_compressed: false,
                });
            }
            
            // Сохраняем сжатые данные
            let ptr = self.inner.allocate(compressed.len() + std::mem::size_of::<usize>())?;
            
            // Сохраняем длину исходных данных в начале блока
            let len_ptr = ptr as *mut usize;
            unsafe { *len_ptr = data.len() };
            
            // Сохраняем сжатые данные после длины
            let data_ptr = (ptr as usize + std::mem::size_of::<usize>()) as *mut u8;
            let slice = unsafe { std::slice::from_raw_parts_mut(data_ptr, compressed.len()) };
            slice.copy_from_slice(&compressed);
            
            Ok(CompressedBlock {
                ptr,
                original_len: data.len(),
                compressed_len: compressed.len(),
                is_compressed: true,
            })
        }
        
        /// Декомпрессирует данные из блока
        pub fn decompress_block(&self, block: &CompressedBlock) -> io::Result<Vec<u8>> {
            if !block.is_compressed {
                // Если данные не сжаты, просто копируем
                let slice = unsafe { std::slice::from_raw_parts(block.ptr as *const u8, block.original_len) };
                return Ok(slice.to_vec());
            }
            
            // Получаем сжатые данные
            let data_ptr = (block.ptr as usize + std::mem::size_of::<usize>()) as *const u8;
            let compressed = unsafe { std::slice::from_raw_parts(data_ptr, block.compressed_len) };
            
            // Декомпрессируем
            self.compressor.decompress(compressed)
        }
        
        /// Возвращает статистику сжатия
        pub fn stats(&self) -> Arc<CompressedArenaStats> {
            self.stats.clone()
        }
        
        /// Возвращает порог сжатия в байтах
        pub fn threshold(&self) -> usize {
            self.threshold
        }
        
        /// Устанавливает порог сжатия в байтах
        pub fn set_threshold(&mut self, threshold: usize) {
            self.threshold = threshold;
        }
        
        /// Возвращает используемый алгоритм сжатия
        pub fn compressor(&self) -> &dyn CompressionAlgorithm {
            self.compressor.as_ref()
        }
    }
}

/// Статистика использования сжатой арены
#[derive(Debug)]
pub struct CompressedArenaStats {
    blocks_compressed: std::sync::atomic::AtomicUsize,
    bytes_before_compression: std::sync::atomic::AtomicUsize,
    bytes_after_compression: std::sync::atomic::AtomicUsize,
}

impl Default for CompressedArenaStats {
    fn default() -> Self {
        Self {
            blocks_compressed: std::sync::atomic::AtomicUsize::new(0),
            bytes_before_compression: std::sync::atomic::AtomicUsize::new(0),
            bytes_after_compression: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

impl Clone for CompressedArenaStats {
    fn clone(&self) -> Self {
        Self {
            blocks_compressed: std::sync::atomic::AtomicUsize::new(
                self.blocks_compressed.load(std::sync::atomic::Ordering::Relaxed)
            ),
            bytes_before_compression: std::sync::atomic::AtomicUsize::new(
                self.bytes_before_compression.load(std::sync::atomic::Ordering::Relaxed)
            ),
            bytes_after_compression: std::sync::atomic::AtomicUsize::new(
                self.bytes_after_compression.load(std::sync::atomic::Ordering::Relaxed)
            ),
        }
    }
}

#[cfg(feature = "arena")]
impl CompressedArenaStats {
    /// Возвращает коэффициент сжатия (0.0 - 1.0)
    pub fn compression_ratio(&self) -> f64 {
        let bytes_before = self.bytes_before_compression.load(std::sync::atomic::Ordering::Relaxed);
        let bytes_after = self.bytes_after_compression.load(std::sync::atomic::Ordering::Relaxed);
        
        if bytes_before == 0 {
            1.0
        } else {
            bytes_after as f64 / bytes_before as f64
        }
    }
    
    /// Возвращает экономию памяти в процентах (0.0 - 1.0)
    pub fn memory_savings(&self) -> f64 {
        1.0 - self.compression_ratio()
    }
    
    /// Возвращает количество сжатых блоков
    pub fn compressed_blocks(&self) -> usize {
        self.blocks_compressed.load(std::sync::atomic::Ordering::Relaxed)
    }
    
    /// Возвращает количество байт до сжатия
    pub fn bytes_before_compression(&self) -> usize {
        self.bytes_before_compression.load(std::sync::atomic::Ordering::Relaxed)
    }
    
    /// Возвращает количество байт после сжатия
    pub fn bytes_after_compression(&self) -> usize {
        self.bytes_after_compression.load(std::sync::atomic::Ordering::Relaxed)
    }
}

#[cfg(feature = "arena")]
pub use arena_impl::CompressedArena;

/// Блок сжатых данных в арене
pub struct CompressedBlock {
    ptr: *mut u8,
    original_len: usize,
    compressed_len: usize,
    is_compressed: bool,
}

impl CompressedBlock {
    /// Возвращает размер оригинальных данных
    pub fn original_size(&self) -> usize {
        self.original_len
    }
    
    /// Возвращает размер сжатых данных
    pub fn compressed_size(&self) -> usize {
        self.compressed_len
    }
    
    /// Сжаты ли данные
    pub fn is_compressed(&self) -> bool {
        self.is_compressed
    }
    
    /// Возвращает коэффициент сжатия (0.0 - 1.0)
    pub fn compression_ratio(&self) -> f64 {
        if self.original_len == 0 {
            1.0
        } else {
            self.compressed_len as f64 / self.original_len as f64
        }
    }
    
    /// Возвращает экономию памяти в процентах (0.0 - 1.0)
    pub fn memory_savings(&self) -> f64 {
        1.0 - self.compression_ratio()
    }
}

impl fmt::Debug for CompressedBlock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CompressedBlock")
            .field("original_size", &self.original_len)
            .field("compressed_size", &self.compressed_len)
            .field("is_compressed", &self.is_compressed)
            .field("compression_ratio", &self.compression_ratio())
            .finish()
    }
}

unsafe impl Send for CompressedBlock {}
unsafe impl Sync for CompressedBlock {}

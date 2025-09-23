//! Реализация CompressedCache с поддержкой сжатия кэшированных данных
use std::any::Any;
use std::fmt;
use std::hash::Hash;
use std::sync::{Arc, Mutex, RwLock};
use std::collections::HashMap;
use std::marker::PhantomData;

use super::{Algorithm, CompressionAlgorithm, StreamingCompression};

#[cfg(feature = "cache")]
/// Кэш с поддержкой сжатия данных для экономии памяти
pub struct CompressedCache<K, V>
where
    K: Hash + Eq + Clone,
    V: Clone + Send + Sync + 'static
{
    compressor: Box<dyn CompressionAlgorithm>,
    data: RwLock<HashMap<K, CompressedValue<V>>>,
    stats: Arc<Mutex<CompressedCacheStats>>,
    threshold: usize,
    _marker: PhantomData<V>,
}

/// Статистика использования сжатого кэша
#[derive(Default, Clone, Debug)]
pub struct CompressedCacheStats {
    /// Количество элементов в кэше
    pub items_count: usize,
    /// Количество сжатых элементов
    pub compressed_items: usize,
    /// Количество несжатых элементов
    pub uncompressed_items: usize,
    /// Размер несжатых данных в байтах
    pub uncompressed_bytes: usize,
    /// Размер сжатых данных в байтах
    pub compressed_bytes: usize,
    /// Количество хитов кэша
    pub hits: usize,
    /// Количество промахов кэша
    pub misses: usize,
    /// Количество успешных операций сжатия
    pub compression_successes: usize,
    /// Количество неудачных операций сжатия (размер не уменьшился)
    pub compression_failures: usize,
}

impl CompressedCacheStats {
    /// Возвращает коэффициент сжатия (0.0 - 1.0)
    pub fn compression_ratio(&self) -> f64 {
        if self.uncompressed_bytes == 0 {
            1.0
        } else {
            self.compressed_bytes as f64 / self.uncompressed_bytes as f64
        }
    }

    /// Возвращает экономию памяти в процентах (0.0 - 1.0)
    pub fn memory_savings(&self) -> f64 {
        1.0 - self.compression_ratio()
    }

    /// Возвращает процент хитов кэша (0.0 - 1.0)
    pub fn hit_ratio(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }
}

/// Сжатое значение в кэше
enum CompressedValue<V> {
    /// Несжатые данные (обычно для маленьких объектов)
    Raw(V),
    /// Сжатые данные
    Compressed {
        /// Сжатые данные
        data: Vec<u8>,
        /// Размер оригинальных данных
        original_size: usize,
    }
}

#[cfg(all(feature = "cache", feature = "serde"))]
impl<K, V> CompressedCache<K, V>
where
    K: Hash + Eq + Clone,
    V: Clone + Send + Sync + serde::Serialize + for<'de> serde::Deserialize<'de> + 'static
{
    /// Создает новый сжатый кэш с указанным алгоритмом сжатия
    pub fn new(algorithm: Algorithm) -> Self {
        Self::new_with_threshold(algorithm, 1024)
    }

    /// Создает новый сжатый кэш с указанным порогом сжатия
    pub fn new_with_threshold(algorithm: Algorithm, threshold: usize) -> Self {
        let compressor = super::new_compressor(algorithm);
        Self {
            compressor,
            data: RwLock::new(HashMap::new()),
            stats: Arc::new(Mutex::new(CompressedCacheStats::default())),
            threshold,
            _marker: PhantomData,
        }
    }

    /// Добавляет значение в кэш
    pub fn insert(&self, key: K, value: V) -> std::io::Result<()> {
        // Сериализуем значение для определения размера
        let serialized = bincode::serde::encode_to_vec(&value, bincode::config::standard()).unwrap_or_default();

        let compressed_value = if serialized.len() < self.threshold {
            // Маленькие объекты не сжимаем
            {
                let mut stats = self.stats.lock().unwrap();
                stats.uncompressed_items += 1;
                stats.uncompressed_bytes += serialized.len();
            }
            CompressedValue::Raw(value)
        } else {
            // Сжимаем большие объекты
            let compressed = self.compressor.compress(&serialized)?;

            let mut stats = self.stats.lock().unwrap();
            if compressed.len() < serialized.len() {
                stats.compressed_items += 1;
                stats.compressed_bytes += compressed.len();
                stats.uncompressed_bytes += serialized.len();
                stats.compression_successes += 1;

                CompressedValue::Compressed {
                    data: compressed,
                    original_size: serialized.len(),
                }
            } else {
                stats.uncompressed_items += 1;
                stats.uncompressed_bytes += serialized.len();
                stats.compression_failures += 1;

                CompressedValue::Raw(value)
            }
        };

        // Обновляем кэш
        let mut cache = self.data.write().unwrap();
        cache.insert(key, compressed_value);

        {
            let mut stats = self.stats.lock().unwrap();
            stats.items_count = cache.len();
        }

        Ok(())
    }

    /// Получает значение из кэша
    pub fn get(&self, key: &K) -> Option<V> {
        let cache = self.data.read().unwrap();
        let value = cache.get(key)?;

        let result = match value {
            CompressedValue::Raw(v) => {
                // Просто клонируем несжатое значение
                Some(v.clone())
            }
            CompressedValue::Compressed { data, original_size: _ } => {
                // Распаковываем сжатые данные
                match self.compressor.decompress(data) {
                    Ok(decompressed) => {
                        match bincode::serde::decode_from_slice::<V, _>(&decompressed, bincode::config::standard()) {
                            Ok((value, _)) => Some(value),
                            Err(_) => None,
                        }
                    }
                    Err(_) => None,
                }
            }
        };

        // Обновляем статистику
        {
            let mut stats = self.stats.lock().unwrap();
            if result.is_some() {
                stats.hits += 1;
            } else {
                stats.misses += 1;
            }
        }

        result
    }

    /// Удаляет значение из кэша
    pub fn remove(&self, key: &K) -> bool {
        let mut cache = self.data.write().unwrap();
        let removed = cache.remove(key).is_some();

        if removed {
            let mut stats = self.stats.lock().unwrap();
            stats.items_count = cache.len();
        }

        removed
    }

    /// Очищает кэш
    pub fn clear(&self) {
        let mut cache = self.data.write().unwrap();
        cache.clear();

        let mut stats = self.stats.lock().unwrap();
        *stats = CompressedCacheStats::default();
    }

    /// Возвращает количество элементов в кэше
    pub fn len(&self) -> usize {
        let cache = self.data.read().unwrap();
        cache.len()
    }

    /// Проверяет, пуст ли кэш
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Возвращает статистику кэша
    pub fn stats(&self) -> CompressedCacheStats {
        let stats = self.stats.lock().unwrap();
        stats.clone()
    }

    /// Возвращает порог сжатия в байтах
    pub fn threshold(&self) -> usize {
        self.threshold
    }

    /// Устанавливает порог сжатия в байтах
    pub fn set_threshold(&mut self, threshold: usize) {
        self.threshold = threshold;
    }
}

#[cfg(feature = "cache")]
impl<K, V> fmt::Debug for CompressedCache<K, V>
where
    K: Hash + Eq + Clone + fmt::Debug,
    V: Clone + Send + Sync + 'static
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let stats = self.stats.lock().unwrap();
        f.debug_struct("CompressedCache")
            .field("items_count", &stats.items_count)
            .field("compressed_items", &stats.compressed_items)
            .field("uncompressed_items", &stats.uncompressed_items)
            .field("compression_ratio", &stats.compression_ratio())
            .field("hit_ratio", &stats.hit_ratio())
            .field("threshold", &self.threshold)
            .finish()
    }
}

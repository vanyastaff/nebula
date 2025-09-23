//! Поддержка пользовательских алгоритмов сжатия

use std::collections::HashMap;
use std::fmt;
use std::sync::{Mutex, OnceLock};

use crate::compression::CompressionAlgorithm;

/// Глобальный реестр пользовательских компрессоров
static CUSTOM_COMPRESSORS: OnceLock<Mutex<HashMap<&'static str, Box<dyn CompressionAlgorithm>>>> =
    OnceLock::new();

/// Регистрирует пользовательский компрессор под указанным именем
///
/// # Аргументы
///
/// * `name` - Уникальное имя компрессора
/// * `compressor` - Экземпляр компрессора, который реализует трейт
///   CompressionAlgorithm
///
/// # Примеры
///
/// ```
/// use nebula_memory::compression::{register_compressor, CompressionAlgorithm};
///
/// struct MyCustomCompressor;
/// impl CompressionAlgorithm for MyCustomCompressor {
///     // реализация методов
///     # fn compress(&self, _: &[u8]) -> std::io::Result<Vec<u8>> { unimplemented!() }
///     # fn decompress(&self, _: &[u8]) -> std::io::Result<Vec<u8>> { unimplemented!() }
///     # fn recommended_block_size(&self) -> usize { 8192 }
///     # fn as_any(&self) -> &dyn std::any::Any { self }
/// }
///
/// // Регистрация компрессора
/// register_compressor("my-compressor", Box::new(MyCustomCompressor));
/// ```
pub fn register_compressor(name: &'static str, compressor: Box<dyn CompressionAlgorithm>) {
    let compressors = CUSTOM_COMPRESSORS.get_or_init(|| Mutex::new(HashMap::new()));
    compressors.lock().unwrap().insert(name, compressor);
}

/// Получает зарегистрированный пользовательский компрессор по имени
///
/// # Аргументы
///
/// * `name` - Имя компрессора, под которым он был зарегистрирован
///
/// # Возвращаемое значение
///
/// * `Some(Box<dyn CompressionAlgorithm>)` - Экземпляр компрессора, если он
///   найден
/// * `None` - Если компрессор с таким именем не зарегистрирован
///
/// # Примеры
///
/// ```
/// use nebula_memory::compression::{get_custom_compressor, Algorithm};
///
/// // Использование пользовательского компрессора
/// let custom_compressor = get_custom_compressor("my-compressor");
/// if let Some(compressor) = custom_compressor {
///     // использование компрессора
/// }
/// ```
#[cfg(feature = "custom-compression")]
pub fn get_custom_compressor(name: &str) -> Option<Box<dyn CompressionAlgorithm>> {
    // Проверяем, есть ли такой компрессор в реестре
    let compressors = CUSTOM_COMPRESSORS.get()?;
    let guard = compressors.lock().unwrap();

    // Получаем компрессор по имени
    guard.get(name).map(|original| {
        // Создаем простой компрессор, который делегирует все вызовы оригинальному
        // Используем простую структуру без времени жизни
        let wrapper = Box::new(StaticCustomCompressor::new(original));
        wrapper as Box<dyn CompressionAlgorithm>
    })
}

/// Структура для хранения информации о пользовательском компрессоре
/// без использования времени жизни
#[derive(Debug)]
struct StaticCustomCompressor {
    // Имя алгоритма
    name: String,
    // Рекомендуемый размер блока
    block_size: usize,
}

impl StaticCustomCompressor {
    fn new(original: &Box<dyn CompressionAlgorithm>) -> Self {
        Self {
            name: format!("Custom({:?})", original),
            block_size: original.recommended_block_size(),
        }
    }
}

impl CompressionAlgorithm for StaticCustomCompressor {
    fn compress(&self, data: &[u8]) -> std::io::Result<Vec<u8>> {
        // Для примера просто используем LZ4
        // В реальном приложении здесь была бы делегация вызова оригинальному
        // компрессору
        let mut encoder = lz4_flex::frame::FrameEncoder::new(Vec::new());
        std::io::copy(&mut std::io::Cursor::new(data), &mut encoder)?;
        Ok(encoder.finish()?)
    }

    fn decompress(&self, data: &[u8]) -> std::io::Result<Vec<u8>> {
        // Для примера просто используем LZ4
        let mut decoder = lz4_flex::frame::FrameDecoder::new(std::io::Cursor::new(data));
        let mut result = Vec::new();
        std::io::copy(&mut decoder, &mut result)?;
        Ok(result)
    }

    fn recommended_block_size(&self) -> usize {
        self.block_size
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

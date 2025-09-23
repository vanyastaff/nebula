//! Трейты для фабрик объектов
//!
//! Этот модуль определяет интерфейсы для создания и управления
//! объектами в контексте пулов и систем управления памятью.

use std::error::Error;
use std::fmt;

use dyn_clone::{clone_trait_object, DynClone};

/// Ошибка фабрики объектов
#[derive(Debug)]
pub enum ObjectFactoryError {
    /// Не удалось создать объект
    CreationFailed(String),

    /// Не удалось инициализировать объект
    InitializationFailed(String),

    /// Превышен лимит создания объектов
    LimitExceeded {
        /// Текущее количество объектов
        current: usize,

        /// Максимальное количество объектов
        max: usize,
    },

    /// Другая ошибка
    Other(String),
}

impl fmt::Display for ObjectFactoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CreationFailed(msg) => write!(f, "Failed to create object: {}", msg),
            Self::InitializationFailed(msg) => write!(f, "Failed to initialize object: {}", msg),
            Self::LimitExceeded { current, max } => {
                write!(f, "Object limit exceeded: {} of {} objects created", current, max)
            },
            Self::Other(msg) => write!(f, "{}", msg),
        }
    }
}

impl Error for ObjectFactoryError {}

/// Результат операции фабрики объектов
pub type ObjectFactoryResult<T> = Result<T, ObjectFactoryError>;

/// Конфигурация фабрики объектов
#[derive(Debug, Clone)]
pub struct ObjectFactoryConfig {
    /// Максимальное количество объектов
    pub max_objects: Option<usize>,

    /// Стратегия создания объектов
    pub creation_strategy: CreationStrategy,

    /// Период очистки неиспользуемых объектов
    pub cleanup_interval: Option<std::time::Duration>,
}

impl Default for ObjectFactoryConfig {
    fn default() -> Self {
        Self {
            max_objects: None,
            creation_strategy: CreationStrategy::default(),
            cleanup_interval: None,
        }
    }
}

/// Стратегия создания объектов
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CreationStrategy {
    /// Создавать объекты по мере необходимости
    OnDemand,

    /// Создавать объекты заранее
    Preload {
        /// Начальное количество объектов
        initial_size: usize,
    },

    /// Создавать объекты с прогрессивным ростом
    Progressive {
        /// Начальное количество объектов
        initial_size: usize,

        /// Шаг увеличения
        growth_factor: f32,
    },
}

impl Default for CreationStrategy {
    fn default() -> Self {
        Self::OnDemand
    }
}

/// Трейт для фабрики объектов
pub trait ObjectFactory<T>: DynClone + Send + Sync {
    /// Создает новый объект
    fn create(&self) -> ObjectFactoryResult<T>;

    /// Создает несколько объектов
    fn create_batch(&self, count: usize) -> ObjectFactoryResult<Vec<T>> {
        let mut objects = Vec::with_capacity(count);
        for _ in 0..count {
            objects.push(self.create()?);
        }
        Ok(objects)
    }

    /// Инициализирует объект с аргументами
    fn initialize(&self, obj: &mut T, args: &[&dyn std::any::Any]) -> ObjectFactoryResult<()>;

    /// Проверяет работоспособность объекта
    fn validate(&self, obj: &T) -> bool;

    /// Возвращает текущее количество созданных объектов
    fn count(&self) -> usize;

    /// Возвращает максимальное количество объектов
    fn max_count(&self) -> Option<usize>;

    /// Устанавливает максимальное количество объектов
    fn set_max_count(&mut self, max: Option<usize>);

    /// Сбрасывает фабрику в начальное состояние
    fn reset(&mut self);
}

// Реализуем клонирование для dyn ObjectFactory
clone_trait_object!(<T> ObjectFactory<T>);

/// Простая реализация фабрики объектов
pub struct SimpleObjectFactory<T, F>
where
    F: Fn() -> ObjectFactoryResult<T> + Send + Sync + Clone,
    T: Clone + Send + Sync,
{
    /// Функция создания объектов
    creator: F,

    /// Конфигурация фабрики
    config: ObjectFactoryConfig,

    /// Счетчик созданных объектов
    count: std::sync::atomic::AtomicUsize,
}

impl<T, F> Clone for SimpleObjectFactory<T, F>
where
    F: Fn() -> ObjectFactoryResult<T> + Send + Sync + Clone,
    T: Clone + Send + Sync,
{
    fn clone(&self) -> Self {
        Self {
            creator: self.creator.clone(),
            config: self.config.clone(),
            count: std::sync::atomic::AtomicUsize::new(
                self.count.load(std::sync::atomic::Ordering::Relaxed),
            ),
        }
    }
}

impl<T, F> SimpleObjectFactory<T, F>
where
    F: Fn() -> ObjectFactoryResult<T> + Send + Sync + Clone,
    T: Clone + Send + Sync,
{
    /// Создает новую фабрику объектов
    pub fn new(creator: F) -> Self {
        Self {
            creator,
            config: ObjectFactoryConfig::default(),
            count: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Создает новую фабрику объектов с конфигурацией
    pub fn with_config(creator: F, config: ObjectFactoryConfig) -> Self {
        Self { creator, config, count: std::sync::atomic::AtomicUsize::new(0) }
    }
}

impl<T, F> ObjectFactory<T> for SimpleObjectFactory<T, F>
where
    F: Fn() -> ObjectFactoryResult<T> + Send + Sync + Clone,
    T: Clone + Send + Sync,
{
    fn create(&self) -> ObjectFactoryResult<T> {
        let current = self.count.load(std::sync::atomic::Ordering::Relaxed);

        if let Some(max) = self.config.max_objects {
            if current >= max {
                return Err(ObjectFactoryError::LimitExceeded { current, max });
            }
        }

        let result = (self.creator)();

        if result.is_ok() {
            self.count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }

        result
    }

    fn initialize(&self, _obj: &mut T, _args: &[&dyn std::any::Any]) -> ObjectFactoryResult<()> {
        // Простая реализация ничего не делает при инициализации
        Ok(())
    }

    fn validate(&self, _obj: &T) -> bool {
        // Простая реализация считает все объекты валидными
        true
    }

    fn count(&self) -> usize {
        self.count.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn max_count(&self) -> Option<usize> {
        self.config.max_objects
    }

    fn set_max_count(&mut self, max: Option<usize>) {
        self.config.max_objects = max;
    }

    fn reset(&mut self) {
        self.count.store(0, std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_factory() {
        let factory = SimpleObjectFactory::new(|| Ok(42));

        let obj = factory.create().unwrap();
        assert_eq!(obj, 42);
        assert_eq!(factory.count(), 1);

        // Batch creation
        let batch = factory.create_batch(5).unwrap();
        assert_eq!(batch.len(), 5);
        assert_eq!(batch[0], 42);
        assert_eq!(factory.count(), 6);
    }

    #[test]
    fn test_factory_limit() {
        let mut factory = SimpleObjectFactory::new(|| Ok(42));
        factory.set_max_count(Some(2));

        let _obj1 = factory.create().unwrap();
        let _obj2 = factory.create().unwrap();

        // Третий объект должен вызвать ошибку
        let result = factory.create();
        assert!(result.is_err());

        if let Err(ObjectFactoryError::LimitExceeded { current, max }) = result {
            assert_eq!(current, 2);
            assert_eq!(max, 2);
        } else {
            panic!("Unexpected error type");
        }
    }
}

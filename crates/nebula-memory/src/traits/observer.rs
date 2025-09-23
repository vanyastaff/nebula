//! Трейты для наблюдения за использованием памяти
//!
//! Этот модуль определяет интерфейсы для мониторинга и наблюдения
//! за использованием памяти в различных компонентах системы.

use std::fmt;
use std::sync::Arc;
use std::time::Instant;

use dyn_clone::{clone_trait_object, DynClone};

/// Уровни давления памяти
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MemoryPressure {
    /// Нормальное использование памяти
    Normal,

    /// Повышенное использование памяти
    Elevated,

    /// Высокое использование памяти
    High,

    /// Критически высокое использование памяти
    Critical,
}

impl Default for MemoryPressure {
    fn default() -> Self {
        Self::Normal
    }
}

impl fmt::Display for MemoryPressure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Normal => write!(f, "Normal"),
            Self::Elevated => write!(f, "Elevated"),
            Self::High => write!(f, "High"),
            Self::Critical => write!(f, "Critical"),
        }
    }
}

/// Тип события памяти
#[derive(Debug, Clone)]
pub enum MemoryEvent {
    /// Выделение памяти
    Allocation {
        /// Размер выделения
        size: usize,

        /// Контекст выделения
        context: String,

        /// Время события
        timestamp: Instant,
    },

    /// Освобождение памяти
    Deallocation {
        /// Размер освобождения
        size: usize,

        /// Контекст освобождения
        context: String,

        /// Время события
        timestamp: Instant,
    },

    /// Изменение давления памяти
    PressureChange {
        /// Новое значение давления
        pressure: MemoryPressure,

        /// Время события
        timestamp: Instant,
    },

    /// Оптимизация памяти
    Optimization {
        /// Описание оптимизации
        description: String,

        /// Сэкономленная память (если применимо)
        bytes_saved: Option<usize>,

        /// Время события
        timestamp: Instant,
    },

    /// Ошибка памяти
    Error {
        /// Описание ошибки
        message: String,

        /// Контекст ошибки
        context: String,

        /// Время события
        timestamp: Instant,
    },
}

impl MemoryEvent {
    /// Получает время события
    pub fn timestamp(&self) -> Instant {
        match self {
            Self::Allocation { timestamp, .. } => *timestamp,
            Self::Deallocation { timestamp, .. } => *timestamp,
            Self::PressureChange { timestamp, .. } => *timestamp,
            Self::Optimization { timestamp, .. } => *timestamp,
            Self::Error { timestamp, .. } => *timestamp,
        }
    }

    /// Создает событие выделения памяти
    pub fn allocation(size: usize, context: impl Into<String>) -> Self {
        Self::Allocation { size, context: context.into(), timestamp: Instant::now() }
    }

    /// Создает событие освобождения памяти
    pub fn deallocation(size: usize, context: impl Into<String>) -> Self {
        Self::Deallocation { size, context: context.into(), timestamp: Instant::now() }
    }

    /// Создает событие изменения давления памяти
    pub fn pressure_change(pressure: MemoryPressure) -> Self {
        Self::PressureChange { pressure, timestamp: Instant::now() }
    }

    /// Создает событие оптимизации памяти
    pub fn optimization(description: impl Into<String>, bytes_saved: Option<usize>) -> Self {
        Self::Optimization {
            description: description.into(),
            bytes_saved,
            timestamp: Instant::now(),
        }
    }

    /// Создает событие ошибки памяти
    pub fn error(message: impl Into<String>, context: impl Into<String>) -> Self {
        Self::Error { message: message.into(), context: context.into(), timestamp: Instant::now() }
    }
}

/// Трейт для наблюдателя за использованием памяти
pub trait MemoryObserver: DynClone + Send + Sync + 'static {
    /// Обрабатывает событие памяти
    fn on_memory_event(&self, event: MemoryEvent);

    /// Обрабатывает выделение памяти
    fn on_allocation(&self, size: usize, context: String) {
        self.on_memory_event(MemoryEvent::allocation(size, context));
    }

    /// Обрабатывает освобождение памяти
    fn on_deallocation(&self, size: usize, context: String) {
        self.on_memory_event(MemoryEvent::deallocation(size, context));
    }

    /// Обрабатывает изменение давления памяти
    fn on_pressure_change(&self, pressure: MemoryPressure) {
        self.on_memory_event(MemoryEvent::pressure_change(pressure));
    }

    /// Обрабатывает оптимизацию памяти
    fn on_optimization(&self, description: String, bytes_saved: Option<usize>) {
        self.on_memory_event(MemoryEvent::optimization(description, bytes_saved));
    }

    /// Обрабатывает ошибку памяти
    fn on_error(&self, message: String, context: String) {
        self.on_memory_event(MemoryEvent::error(message, context));
    }
}

// Реализуем клонирование для dyn MemoryObserver
clone_trait_object!(MemoryObserver);

/// Пустой наблюдатель, который ничего не делает
#[derive(Debug, Default, Clone, Copy)]
pub struct NoOpObserver;

impl MemoryObserver for NoOpObserver {
    fn on_memory_event(&self, _event: MemoryEvent) {
        // Ничего не делаем
    }
}

/// Трейт для объектов, которые могут быть наблюдаемыми
pub trait Observable {
    /// Добавляет наблюдателя и возвращает его идентификатор
    fn add_observer(&mut self, observer: Arc<dyn MemoryObserver>) -> usize;

    /// Удаляет наблюдателя
    fn remove_observer(&mut self, id: usize) -> Option<Arc<dyn MemoryObserver>>;

    /// Получает список идентификаторов наблюдателей
    fn observer_ids(&self) -> Vec<usize>;
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Clone)]
    struct TestObserver {
        events: Arc<Mutex<Vec<MemoryEvent>>>,
    }

    impl TestObserver {
        fn new() -> (Self, Arc<Mutex<Vec<MemoryEvent>>>) {
            let events = Arc::new(Mutex::new(Vec::new()));
            (Self { events: events.clone() }, events)
        }
    }

    impl MemoryObserver for TestObserver {
        fn on_memory_event(&self, event: MemoryEvent) {
            self.events.lock().unwrap().push(event);
        }
    }

    #[test]
    fn test_memory_event_creation() {
        let event = MemoryEvent::allocation(1024, "test");
        match &event {
            MemoryEvent::Allocation { size, context, .. } => {
                assert_eq!(*size, 1024);
                assert_eq!(context, "test");
            },
            _ => panic!("Unexpected event type"),
        }
    }

    #[test]
    fn test_observer() {
        let (observer, events) = TestObserver::new();

        observer.on_allocation(1024, "alloc_test".to_string());
        observer.on_deallocation(512, "dealloc_test".to_string());
        observer.on_pressure_change(MemoryPressure::High);

        let events = events.lock().unwrap();
        assert_eq!(events.len(), 3);

        match &events[0] {
            MemoryEvent::Allocation { size, context, .. } => {
                assert_eq!(*size, 1024);
                assert_eq!(context, "alloc_test");
            },
            _ => panic!("Unexpected event type at index 0"),
        }

        match &events[1] {
            MemoryEvent::Deallocation { size, context, .. } => {
                assert_eq!(*size, 512);
                assert_eq!(context, "dealloc_test");
            },
            _ => panic!("Unexpected event type at index 1"),
        }

        match &events[2] {
            MemoryEvent::PressureChange { pressure, .. } => {
                assert_eq!(*pressure, MemoryPressure::High);
            },
            _ => panic!("Unexpected event type at index 2"),
        }
    }
}

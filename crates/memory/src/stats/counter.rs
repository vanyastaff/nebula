// Типы счетчиков для статистики и мониторинга
use core::sync::atomic::{AtomicI64, Ordering};

/// Тип счетчика определяет его поведение и интерпретацию значения
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CounterType {
    /// Счетчик (увеличивается/уменьшается, отражает количество)
    Counter,

    /// Счетчик-указатель (отражает текущее состояние, например размер памяти)
    Gauge,

    /// Гистограмма (распределение значений)
    Histogram,

    /// Счетчик времени выполнения (для профилирования)
    Timer,
}

/// Атомарный счетчик для сбора метрик и статистики
#[derive(Debug)]
pub struct Counter {
    /// Значение счетчика
    value: AtomicI64,

    /// Тип счетчика
    counter_type: CounterType,
}

impl Counter {
    /// Создает новый счетчик указанного типа
    pub fn new(counter_type: CounterType) -> Self {
        Self {
            value: AtomicI64::new(0),
            counter_type,
        }
    }

    /// Создает новый счетчик указанного типа с начальным значением
    pub fn with_value(counter_type: CounterType, initial_value: i64) -> Self {
        Self {
            value: AtomicI64::new(initial_value),
            counter_type,
        }
    }

    /// Возвращает текущее значение счетчика
    pub fn value(&self) -> i64 {
        self.value.load(Ordering::Acquire)
    }

    /// Устанавливает значение счетчика
    pub fn set(&self, value: i64) {
        self.value.store(value, Ordering::Release);
    }

    /// Увеличивает значение счетчика на указанную величину
    pub fn increment(&self, delta: i64) {
        self.value.fetch_add(delta, Ordering::AcqRel);
    }

    /// Уменьшает значение счетчика на указанную величину
    pub fn decrement(&self, delta: i64) {
        self.value.fetch_sub(delta, Ordering::AcqRel);
    }

    /// Возвращает тип счетчика
    pub fn counter_type(&self) -> CounterType {
        self.counter_type
    }

    /// Сбрасывает значение счетчика в 0
    pub fn reset(&self) {
        self.value.store(0, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_counter_operations() {
        let counter = Counter::new(CounterType::Counter);

        // Проверка начального значения
        assert_eq!(counter.value(), 0);

        // Проверка инкремента
        counter.increment(5);
        assert_eq!(counter.value(), 5);

        // Проверка декремента
        counter.decrement(2);
        assert_eq!(counter.value(), 3);

        // Проверка сброса
        counter.reset();
        assert_eq!(counter.value(), 0);

        // Проверка установки значения
        counter.set(10);
        assert_eq!(counter.value(), 10);
    }

    #[test]
    fn test_counter_with_initial_value() {
        let counter = Counter::with_value(CounterType::Gauge, 100);

        // Проверка начального значения
        assert_eq!(counter.value(), 100);

        // Проверка типа счетчика
        assert_eq!(counter.counter_type(), CounterType::Gauge);
    }
}

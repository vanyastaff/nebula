//! Трейты для управления приоритетами
//!
//! Этот модуль определяет интерфейсы для приоритезации операций с памятью,
//! которые позволяют более гибко управлять ресурсами в условиях ограничений.

use std::cmp::Ordering;
use std::fmt::Debug;

use dyn_clone::{clone_trait_object, DynClone};

/// Базовый тип для значений приоритета
pub trait PriorityValue: PartialOrd + Clone + Send + Sync + Debug + 'static {}

// Автоматическая реализация для всех подходящих типов
impl<T> PriorityValue for T where T: PartialOrd + Clone + Send + Sync + Debug + 'static {}

/// Трейт для объектов с приоритетом
///
/// Этот трейт может быть реализован для любого типа, который имеет
/// понятие приоритета, что позволяет системе управления памятью
/// принимать решения о выделении и освобождении ресурсов.
pub trait Priority: DynClone + Send + Sync + 'static {
    /// Получение приоритета объекта
    fn priority(&self) -> u8;

    /// Установка нового приоритета
    fn set_priority(&mut self, priority: u8);

    /// Сравнение приоритета с другим объектом того же типа приоритета
    fn compare(&self, other: &dyn Priority) -> Ordering {
        self.priority().cmp(&other.priority())
    }
}

// Реализуем клонирование для dyn Priority
clone_trait_object!(Priority);

/// Трейт для объектов с возможностью изменения приоритета
pub trait DynamicPriority: Priority {
    /// Повышение приоритета
    fn increase_priority(&mut self);

    /// Понижение приоритета
    fn decrease_priority(&mut self);
}

// Реализуем клонирование для dyn DynamicPriority
clone_trait_object!(DynamicPriority);

/// Приоритет на основе численного значения
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct NumericPriority {
    value: u8,
}

impl NumericPriority {
    /// Создание нового приоритета
    pub fn new(value: u8) -> Self {
        Self { value }
    }

    /// Получение значения приоритета
    pub fn value(&self) -> u8 {
        self.value
    }
}

impl Priority for NumericPriority {
    fn priority(&self) -> u8 {
        self.value
    }

    fn set_priority(&mut self, priority: u8) {
        self.value = priority;
    }
}

/// Реализация DynamicPriority для NumericPriority
impl DynamicPriority for NumericPriority {
    fn increase_priority(&mut self) {
        if self.value < u8::MAX {
            self.value += 1;
        }
    }

    fn decrease_priority(&mut self) {
        if self.value > 0 {
            self.value -= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_numeric_priority() {
        let mut p1 = NumericPriority::new(10);
        let p2 = NumericPriority::new(20);

        assert_eq!(p1.priority(), 10);
        assert_eq!(p1.compare(&p2), Ordering::Less);

        p1.set_priority(30);
        assert_eq!(p1.priority(), 30);
        assert_eq!(p1.compare(&p2), Ordering::Greater);

        p1.increase_priority();
        assert_eq!(p1.priority(), 31);

        p1.decrease_priority();
        assert_eq!(p1.priority(), 30);
    }
}

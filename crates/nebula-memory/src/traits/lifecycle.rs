//! Трейты для управления жизненным циклом объектов в памяти
//!
//! Этот модуль определяет интерфейсы для контроля жизненного цикла объектов,
//! что важно для оптимизации управления памятью в пулах и аренах.

use dyn_clone::{clone_trait_object, DynClone};

/// Трейт для объектов, поддерживающих сброс в начальное состояние
///
/// Этот трейт должны реализовывать типы, которые могут быть сброшены
/// в начальное состояние без необходимости полной деструкции и реконструкции.
/// Это особенно полезно для пулов объектов.
pub trait Resetable: DynClone {
    /// Сбрасывает объект в начальное состояние
    fn reset(&mut self);

    /// Проверяет, находится ли объект в сброшенном состоянии
    fn is_reset(&self) -> bool {
        false
    }
}

// Реализуем клонирование для dyn Resetable
clone_trait_object!(Resetable);

/// Трейт для объектов, которые могут быть использованы в пулах
///
/// Расширяет `Resetable` дополнительными методами для работы в пулах объектов.
pub trait Poolable: Resetable + Send + Sync + 'static {
    /// Получает размер объекта в байтах
    fn size_bytes(&self) -> usize {
        std::mem::size_of_val(self)
    }

    /// Возвращает приоритет объекта (для эвристики вытеснения)
    fn pool_priority(&self) -> u8 {
        128 // Средний приоритет по умолчанию
    }

    /// Вызывается при добавлении объекта в пул
    fn on_return_to_pool(&mut self) {
        self.reset();
    }

    /// Вызывается при извлечении объекта из пула
    fn on_get_from_pool(&mut self) {}
}

/// Трейт для арендованных объектов с отслеживанием времени жизни
pub trait Recyclable: DynClone + Send + Sync + 'static {
    /// Метка времени последнего использования
    fn last_used(&self) -> std::time::Instant;

    /// Обновление метки времени использования
    fn mark_used(&mut self);

    /// Проверяет, может ли объект быть переработан
    fn can_recycle(&self) -> bool;

    /// Подготавливает объект к переработке
    fn prepare_for_recycling(&mut self);
}

// Реализуем клонирование для dyn Recyclable
clone_trait_object!(Recyclable);

/// Трейт для управления жизненным циклом объекта
pub trait ObjectLifecycle: DynClone + Send + Sync {
    /// Тип управляемого объекта
    type Object: Send + Sync + 'static;

    /// Создает новый объект
    fn create(&self) -> Self::Object;

    /// Сбрасывает объект в начальное состояние
    fn reset(&self, obj: &mut Self::Object);

    /// Уничтожает объект
    fn destroy(&self, obj: Self::Object);

    /// Проверяет работоспособность объекта
    fn validate(&self, obj: &Self::Object) -> bool;

    /// Обрабатывает неиспользуемый объект
    fn on_idle(&self, obj: &mut Self::Object);
}

// Реализуем клонирование для dyn ObjectLifecycle
clone_trait_object!(<T> ObjectLifecycle<Object = T>);

/// Простая реализация жизненного цикла для типов с реализацией Default и Reset
#[derive(Debug, Clone)]
pub struct DefaultLifecycle<T>
where T: Default + Resetable + Clone + Send + Sync + 'static
{
    // Используем PhantomData с правильной вариацией, показывая что мы не владеем T
    _phantom: std::marker::PhantomData<fn() -> T>,
}

impl<T> DefaultLifecycle<T>
where T: Default + Resetable + Clone + Send + Sync + 'static
{
    /// Создает новый менеджер жизненного цикла
    #[inline]
    pub fn new() -> Self {
        Self { _phantom: std::marker::PhantomData }
    }
}

impl<T> Default for DefaultLifecycle<T>
where T: Default + Resetable + Clone + Send + Sync + 'static
{
    fn default() -> Self {
        Self::new()
    }
}

impl<T> ObjectLifecycle for DefaultLifecycle<T>
where T: Default + Resetable + Clone + Send + Sync + 'static
{
    type Object = T;

    #[inline]
    fn create(&self) -> Self::Object {
        T::default()
    }

    #[inline]
    fn reset(&self, obj: &mut Self::Object) {
        obj.reset();
    }

    #[inline]
    fn destroy(&self, _obj: Self::Object) {
        // Объект будет уничтожен автоматически при выходе из области видимости
        // благодаря системе владения Rust
    }

    #[inline]
    fn validate(&self, _obj: &Self::Object) -> bool {
        // По умолчанию все объекты считаются валидными
        true
    }

    #[inline]
    fn on_idle(&self, _obj: &mut Self::Object) {
        // По умолчанию не выполняем никаких действий для объектов в простое
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default, Clone)]
    struct TestObject {
        value: i32,
        reset_called: bool,
    }

    impl Resetable for TestObject {
        fn reset(&mut self) {
            self.value = 0;
            self.reset_called = true;
        }

        fn is_reset(&self) -> bool {
            self.value == 0 && self.reset_called
        }
    }

    #[test]
    fn test_default_lifecycle() {
        let lifecycle = DefaultLifecycle::<TestObject>::new();

        let mut obj = lifecycle.create();
        obj.value = 42;

        assert!(!obj.is_reset());

        lifecycle.reset(&mut obj);

        assert!(obj.is_reset());
        assert_eq!(obj.value, 0);
    }
}

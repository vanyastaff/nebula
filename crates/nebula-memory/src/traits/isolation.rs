//! Трейты для изоляции памяти
//!
//! Этот модуль определяет трейты для изоляции памяти между
//! различными компонентами и подсистемами.

use std::error::Error;
use std::fmt;

use dyn_clone::{clone_trait_object, DynClone};

use super::context::MemoryContext;

/// Ошибка изоляции памяти
#[derive(Debug)]
pub enum MemoryIsolationError {
    /// Превышение лимита памяти
    MemoryLimitExceeded { requested: usize, available: usize, context: String },

    /// Нехватка памяти в системе
    SystemMemoryExhausted { requested: usize, context: String },

    /// Отказано в доступе из-за приоритета
    PriorityTooLow { context: String },

    /// Другие ошибки
    Other(String),
}

impl fmt::Display for MemoryIsolationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MemoryLimitExceeded { requested, available, context } => {
                write!(
                    f,
                    "Превышение лимита памяти: запрошено {}, доступно {} в контексте '{}'",
                    requested, available, context
                )
            },
            Self::SystemMemoryExhausted { requested, context } => {
                write!(
                    f,
                    "Нехватка системной памяти: запрошено {} в контексте '{}'",
                    requested, context
                )
            },
            Self::PriorityTooLow { context } => {
                write!(f, "Отказано в доступе: приоритет слишком низкий в контексте '{}'", context)
            },
            Self::Other(msg) => write!(f, "{}", msg),
        }
    }
}

impl Error for MemoryIsolationError {}

/// Результат операции с изоляцией памяти
pub type MemoryIsolationResult<T> = Result<T, MemoryIsolationError>;

/// Трейт для изоляции памяти
///
/// Этот трейт обеспечивает абстракцию для изоляции памяти,
/// позволяя ограничивать и контролировать использование памяти
/// различными компонентами системы.
pub trait MemoryIsolation: DynClone + Send + Sync + 'static {
    /// Тип контекста, используемый этой изоляцией
    type Context: MemoryContext + ?Sized;

    /// Запрашивает выделение памяти в указанном контексте
    fn request_memory(
        &self,
        size: usize,
        context: &Self::Context,
    ) -> MemoryIsolationResult<MemoryAllocation>;

    /// Возвращает текущее использование памяти для контекста
    fn memory_usage(&self, context: &Self::Context) -> usize;

    /// Возвращает доступную память для контекста
    fn available_memory(&self, context: &Self::Context) -> Option<usize>;

    /// Регистрирует использование памяти без выделения
    fn register_memory_usage(
        &self,
        size: usize,
        context: &Self::Context,
    ) -> MemoryIsolationResult<()>;

    /// Освобождает использование памяти без фактического освобождения
    fn unregister_memory_usage(&self, size: usize, context: &Self::Context);
}

// Реализуем клонирование для dyn MemoryIsolation
clone_trait_object!(<C> MemoryIsolation<Context = C>);

/// Токен выделенной памяти
///
/// Этот объект представляет выделенную память в контексте изоляции.
/// При удалении токена память автоматически возвращается в пул.
pub struct MemoryAllocation {
    /// Размер выделенной памяти
    pub size: usize,

    /// Контекст, в котором была выделена память
    pub context_id: String,

    /// Обработчик освобождения памяти
    release_handler: Option<Box<dyn FnOnce() + Send + Sync>>,
}

impl MemoryAllocation {
    /// Создает новый токен выделения памяти
    pub fn new<F>(size: usize, context_id: String, release_handler: F) -> Self
    where F: FnOnce() + Send + Sync + 'static {
        Self { size, context_id, release_handler: Some(Box::new(release_handler)) }
    }

    /// Ручное освобождение памяти
    pub fn release(mut self) {
        if let Some(handler) = self.release_handler.take() {
            handler();
        }
    }
}

impl Drop for MemoryAllocation {
    fn drop(&mut self) {
        if let Some(handler) = self.release_handler.take() {
            handler();
        }
    }
}

/// Простая реализация изоляции памяти
#[cfg(feature = "budget")]
pub struct SimpleMemoryIsolation {
    /// Бюджет памяти для этой изоляции
    budget: std::sync::Arc<crate::budget::MemoryBudget>,

    /// Отслеживание использования памяти по контекстам
    context_usage: std::sync::Mutex<std::collections::HashMap<String, usize>>,

    /// Сбор статистики
    #[cfg(feature = "stats")]
    stats: Option<std::sync::Arc<crate::stats::MemoryStats>>,
}

#[cfg(feature = "budget")]
impl SimpleMemoryIsolation {
    /// Создает новую изоляцию памяти с указанным бюджетом
    pub fn new(budget: std::sync::Arc<crate::budget::MemoryBudget>) -> Self {
        Self {
            budget,
            context_usage: std::sync::Mutex::new(std::collections::HashMap::new()),
            #[cfg(feature = "stats")]
            stats: None,
        }
    }

    /// Создает новую изоляцию памяти с указанным бюджетом и статистикой
    #[cfg(feature = "stats")]
    pub fn with_stats(budget: std::sync::Arc<crate::budget::MemoryBudget>, stats: std::sync::Arc<crate::stats::MemoryStats>) -> Self {
        Self {
            budget,
            context_usage: std::sync::Mutex::new(std::collections::HashMap::new()),
            stats: Some(stats),
        }
    }

    /// Возвращает бюджет памяти
    pub fn budget(&self) -> &std::sync::Arc<crate::budget::MemoryBudget> {
        &self.budget
    }
}

#[cfg(feature = "budget")]
impl dyn_clone::DynClone for SimpleMemoryIsolation {}

#[cfg(feature = "budget")]
impl Clone for SimpleMemoryIsolation {
    fn clone(&self) -> Self {
        Self {
            budget: self.budget.clone(),
            context_usage: std::sync::Mutex::new(std::collections::HashMap::new()),
            #[cfg(feature = "stats")]
            stats: self.stats.clone(),
        }
    }
}

#[cfg(feature = "budget")]
impl<C: MemoryContext> MemoryIsolation for SimpleMemoryIsolation {
    type Context = C;

    fn request_memory(
        &self,
        size: usize,
        context: &Self::Context,
    ) -> MemoryIsolationResult<MemoryAllocation> {
        // Проверяем, что контекст может выделить память
        if !context.can_allocate(size) {
            return Err(MemoryIsolationError::MemoryLimitExceeded {
                requested: size,
                available: context.memory_limit().unwrap_or(0),
                context: context.identifier().to_string(),
            });
        }

        // Запрашиваем память из бюджета
        match self.budget.request_memory(size) {
            Ok(()) => {
                // Обновляем использование памяти для контекста
                let context_id = context.identifier().to_string();
                let mut context_usage = self.context_usage.lock().unwrap();
                let current = context_usage.get(&context_id).copied().unwrap_or(0);
                context_usage.insert(context_id.clone(), current + size);

                // Обновляем статистику, если она включена
                #[cfg(feature = "stats")]
                if let Some(stats) = &self.stats {
                    stats.record_allocation(size);
                }

                // Создаем токен выделения
                let allocation = MemoryAllocation::new(
                    size,
                    context_id,
                    {
                        let budget = self.budget.clone();
                        let context_usage = self.context_usage.clone();
                        #[cfg(feature = "stats")]
                        let stats = self.stats.clone();
                        let context_id = context.identifier().to_string();

                        move || {
                            // Освобождаем память в бюджете
                            budget.release_memory(size);

                            // Обновляем использование памяти для контекста
                            let mut context_usage = context_usage.lock().unwrap();
                            let current = context_usage.get(&context_id).copied().unwrap_or(0);
                            context_usage.insert(context_id, current.saturating_sub(size));

                            // Обновляем статистику, если она включена
                            #[cfg(feature = "stats")]
                            if let Some(stats) = &stats {
                                stats.record_deallocation(size);
                            }
                        }
                    },
                );

                Ok(allocation)
            },
            Err(_) => Err(MemoryIsolationError::SystemMemoryExhausted {
                requested: size,
                context: context.identifier().to_string(),
            }),
        }
    }

    fn memory_usage(&self, context: &Self::Context) -> usize {
        let context_id = context.identifier().to_string();
        let context_usage = self.context_usage.lock().unwrap();
        context_usage.get(&context_id).copied().unwrap_or(0)
    }

    fn available_memory(&self, context: &Self::Context) -> Option<usize> {
        let context_limit = context.memory_limit();
        let context_usage = self.memory_usage(context);

        match context_limit {
            Some(limit) => Some(limit.saturating_sub(context_usage)),
            None => {
                // Если у контекста нет лимита, используем доступную память бюджета
                let budget_used = self.budget.used();
                let budget_limit = self.budget.limit();
                Some(budget_limit.saturating_sub(budget_used))
            }
        }
    }

    fn register_memory_usage(
        &self,
        size: usize,
        context: &Self::Context,
    ) -> MemoryIsolationResult<()> {
        // Проверяем, что контекст может выделить память
        if !context.can_allocate(size) {
            return Err(MemoryIsolationError::MemoryLimitExceeded {
                requested: size,
                available: context.memory_limit().unwrap_or(0),
                context: context.identifier().to_string(),
            });
        }

        // Обновляем использование памяти для контекста
        let context_id = context.identifier().to_string();
        let mut context_usage = self.context_usage.lock().unwrap();
        let current = context_usage.get(&context_id).copied().unwrap_or(0);
        context_usage.insert(context_id, current + size);

        // Обновляем статистику, если она включена
        #[cfg(feature = "stats")]
        if let Some(stats) = &self.stats {
            stats.record_allocation(size);
        }

        Ok(())
    }

    fn unregister_memory_usage(&self, size: usize, context: &Self::Context) {
        // Обновляем использование памяти для контекста
        let context_id = context.identifier().to_string();
        let mut context_usage = self.context_usage.lock().unwrap();
        let current = context_usage.get(&context_id).copied().unwrap_or(0);
        context_usage.insert(context_id, current.saturating_sub(size));

        // Обновляем статистику, если она включена
        #[cfg(feature = "stats")]
        if let Some(stats) = &self.stats {
            stats.record_deallocation(size);
        }
    }
}

#[cfg(not(feature = "budget"))]
pub struct SimpleMemoryIsolation {
    // Реализация будет добавлена позже
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use super::*;

    #[test]
    fn test_memory_allocation() {
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();

        {
            let allocation = MemoryAllocation::new(1024, "test".to_string(), move || {
                counter_clone.fetch_add(1, Ordering::SeqCst);
            });

            assert_eq!(allocation.size, 1024);
            assert_eq!(allocation.context_id, "test");
        }

        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }
}

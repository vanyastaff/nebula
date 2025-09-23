//! Трейты для контекста памяти
//!
//! Этот модуль определяет интерфейс для контекстов памяти,
//! которые используются для изоляции и управления ресурсами.

use std::hash::Hash;
use std::sync::Arc;

use dyn_clone::{clone_trait_object, DynClone};

/// Уровни приоритета для контекстов памяти
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MemoryPriority {
    Critical = 0,
    High = 1,
    Normal = 2,
    Low = 3,
    Background = 4,
}

impl Default for MemoryPriority {
    fn default() -> Self {
        Self::Normal
    }
}

/// Контекст памяти для изоляции и управления ресурсами
///
/// Этот трейт обеспечивает абстракцию для изоляции памяти между
/// различными компонентами системы. Контексты могут быть организованы
/// в иерархию, где дочерние контексты наследуют ограничения родительских.
pub trait MemoryContext: DynClone + Send + Sync {
    /// Тип приоритета, используемый в контексте
    type Priority: PartialOrd + Clone + Send + Sync;

    /// Тип идентификатора контекста
    type Identifier: Hash + Eq + Clone + Send + Sync;

    /// Создает дочерний контекст с указанным идентификатором
    fn create_child_context(
        &self,
        id: Self::Identifier,
    ) -> Box<dyn MemoryContext<Priority = Self::Priority, Identifier = Self::Identifier>>;

    /// Возвращает ограничение памяти для контекста (если установлено)
    fn memory_limit(&self) -> Option<usize>;

    /// Возвращает приоритет контекста
    fn priority(&self) -> Self::Priority;

    /// Возвращает идентификатор контекста
    fn identifier(&self) -> &Self::Identifier;

    /// Возвращает родительский контекст (если есть)
    fn parent(
        &self,
    ) -> Option<Arc<dyn MemoryContext<Priority = Self::Priority, Identifier = Self::Identifier>>>;

    /// Проверяет, соответствует ли запрос памяти ограничениям контекста
    fn can_allocate(&self, size: usize) -> bool {
        match self.memory_limit() {
            Some(limit) => size <= limit,
            None => true,
        }
    }
}

// Реализуем клонирование для dyn MemoryContext
clone_trait_object!(<P, I> MemoryContext<Priority = P, Identifier = I>);

// Простой тип для хранения данных контекста
#[derive(Debug, Clone)]
struct ContextData<P, I>
where
    P: PartialOrd + Clone + Send + Sync + 'static,
    I: Hash + Eq + Clone + Send + Sync + 'static,
{
    /// Идентификатор контекста
    pub id: I,

    /// Приоритет контекста
    pub priority: P,

    /// Ограничение памяти (если установлено)
    pub memory_limit: Option<usize>,
}

/// Базовая реализация контекста памяти
pub struct SimpleMemoryContext<P, I>
where
    P: PartialOrd + Clone + Send + Sync + 'static,
    I: Hash + Eq + Clone + Send + Sync + 'static,
{
    /// Данные контекста
    data: ContextData<P, I>,

    /// Родительский контекст (если есть)
    parent: Option<Arc<dyn MemoryContext<Priority = P, Identifier = I>>>,
}

impl<P, I> Clone for SimpleMemoryContext<P, I>
where
    P: PartialOrd + Clone + Send + Sync + 'static,
    I: Hash + Eq + Clone + Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self { data: self.data.clone(), parent: self.parent.clone() }
    }
}

impl<P, I> std::fmt::Debug for SimpleMemoryContext<P, I>
where
    P: PartialOrd + Clone + Send + Sync + std::fmt::Debug + 'static,
    I: Hash + Eq + Clone + Send + Sync + std::fmt::Debug + 'static,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimpleMemoryContext")
            .field("id", &self.data.id)
            .field("priority", &self.data.priority)
            .field("memory_limit", &self.data.memory_limit)
            .field("has_parent", &self.parent.is_some())
            .finish()
    }
}

impl<P, I> SimpleMemoryContext<P, I>
where
    P: PartialOrd + Clone + Send + Sync + 'static,
    I: Hash + Eq + Clone + Send + Sync + 'static,
{
    /// Создает новый контекст
    pub fn new(id: I, priority: P, memory_limit: Option<usize>) -> Self {
        Self { data: ContextData { id, priority, memory_limit }, parent: None }
    }

    /// Создает новый контекст с родителем
    pub fn with_parent(
        id: I,
        priority: P,
        memory_limit: Option<usize>,
        parent: Arc<dyn MemoryContext<Priority = P, Identifier = I>>,
    ) -> Self {
        Self { data: ContextData { id, priority, memory_limit }, parent: Some(parent) }
    }
}

/// Простая строковая реализация контекста памяти с числовым приоритетом
pub type StringContext = SimpleMemoryContext<u8, String>;

impl StringContext {
    /// Создает новый строковый контекст
    pub fn create(id: impl Into<String>, priority: u8, memory_limit: Option<usize>) -> Self {
        SimpleMemoryContext::new(id.into(), priority, memory_limit)
    }
}

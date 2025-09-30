//! Общие трейты для интеграции компонентов nebula-memory
//!
//! Этот модуль содержит трейты, которые обеспечивают интеграцию между
//! различными компонентами nebula-memory и другими крейтами без прямых
//! зависимостей.

mod context;
mod factory;
mod isolation;
mod lifecycle;
mod observer;
mod priority;

pub use context::*;
pub use factory::*;
pub use isolation::*;
pub use lifecycle::*;
pub use observer::*;
pub use priority::*;

// Core memory management traits
use core::alloc::Layout;
use crate::{AllocResult, MemoryResult};

/// Core memory management trait for allocators and pools
pub trait MemoryManager {
    /// Allocate memory with the given layout
    unsafe fn allocate(&mut self, layout: Layout) -> AllocResult<*mut u8>;

    /// Deallocate previously allocated memory
    unsafe fn deallocate(&mut self, ptr: *mut u8, layout: Layout);

    /// Reset the memory manager to initial state
    fn reset(&mut self) -> MemoryResult<()>;

    /// Get the name of this memory manager
    fn name(&self) -> &'static str;
}

/// Memory usage tracking trait
pub trait MemoryUsage {
    /// Get current memory usage in bytes
    fn current_usage(&self) -> usize;

    /// Get peak memory usage in bytes
    fn peak_usage(&self) -> usize;

    /// Get total allocations performed
    fn total_allocations(&self) -> u64;

    /// Get total deallocations performed
    fn total_deallocations(&self) -> u64;

    /// Reset usage statistics
    fn reset_stats(&mut self);
}

/// Reexport всех трейтов через преамбулу
pub mod prelude {
    pub use super::context::*;
    pub use super::factory::*;
    pub use super::isolation::*;
    pub use super::lifecycle::*;
    pub use super::observer::*;
    pub use super::priority::*;
    pub use super::{MemoryManager, MemoryUsage};
}

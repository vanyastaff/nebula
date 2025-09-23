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

/// Reexport всех трейтов через преамбулу
pub mod prelude {
    pub use super::context::*;
    pub use super::factory::*;
    pub use super::isolation::*;
    pub use super::lifecycle::*;
    pub use super::observer::*;
    pub use super::priority::*;
}

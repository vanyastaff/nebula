//! Реализации backends для [Storage](crate::Storage).

mod memory;

pub use memory::{MemoryStorage, MemoryStorageTyped};

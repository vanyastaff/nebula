//! Storage backend implementations for credential state persistence

#[cfg(feature = "storage-postgres")]
pub mod postgres;

pub mod memory;

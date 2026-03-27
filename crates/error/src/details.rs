//! TypeId-keyed extensible detail storage.
//!
//! Placeholder types — full implementation in a later task.

/// Marker trait for types that can be stored in [`ErrorDetails`].
pub trait ErrorDetail: std::any::Any + Send + Sync + std::fmt::Debug {}

/// TypeId-keyed bag of [`ErrorDetail`] values.
///
/// Placeholder — full implementation in a later task.
#[derive(Debug)]
pub struct ErrorDetails;

//! Re-exports the standalone error types for backward compatibility.
//!
//! The error types are now organized by logical concern:
//! - `ValueError` - Core value operations (in `crate::error`)
//! - `ConversionError` - Type conversions (in `crate::core::conversions`)
//! - `SerdeError` - Serialization (in `crate::core::serde`)

// Re-export standalone error types
pub use crate::error::*;

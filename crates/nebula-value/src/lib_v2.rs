#![warn(missing_docs)]
#![warn(clippy::all)]
#![cfg_attr(docsrs, feature(doc_cfg))]

//! # Nebula Value v2.0
//!
//! World-class value type system for workflow engines.
//!
//! ## Features
//!
//! - 🚀 **Performance**: O(log n) operations with persistent data structures
//! - 🛡️ **Type Safety**: No panics, comprehensive error handling
//! - 🎯 **Workflow-Optimized**: Designed for n8n-like use cases
//!
//! ## Quick Start
//!
//! ```rust
//! use nebula_value::prelude::*;
//!
//! let value = Value::from(42);
//! assert!(value.is_integer());
//! ```

// Module declarations (order matters)
pub mod core;
pub mod scalar;
pub mod collections;

// Conditional modules
#[cfg(feature = "temporal")]
#[cfg_attr(docsrs, doc(cfg(feature = "temporal")))]
pub mod temporal;

#[cfg(feature = "serde")]
#[cfg_attr(docsrs, doc(cfg(feature = "serde")))]
pub mod serde;

pub mod validation;
pub mod conversion;
pub mod operations;
pub mod hash;
pub mod display;

#[cfg(feature = "memory-pooling")]
#[cfg_attr(docsrs, doc(cfg(feature = "memory-pooling")))]
pub mod memory;

pub mod security;
pub mod observability;

// Re-exports
pub use crate::core::Value;

/// Prelude module with commonly used items
pub mod prelude {
    pub use crate::core::{Value, ValueKind};
    pub use crate::collections::{Array, Object};

    // Re-export ecosystem
    pub use nebula_error::{NebulaError, NebulaResult};
}
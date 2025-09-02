//! Core functionality for nebula-validator
//! 
//! This module contains the fundamental types for validation:
//! - `Valid<T>` and `Invalid<T>` for type-safe validation results
//! - `Validated<T>` enum for handling both cases
//! - `ValidationProof` for validation evidence
//! - Core error types

mod validity;
mod validated;
mod proof;
mod error;

// Re-export all core types
pub use validity::{Valid, Invalid};
pub use validated::Validated;
pub use proof::{ValidationProof, ProofType, ProofBuilder};
pub use error::{CoreError, CoreResult};

// Re-export commonly used items
pub use validated::ValidatedExt;
pub use proof::ProofExt;
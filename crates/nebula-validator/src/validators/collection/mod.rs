//! Collection validators
//!
//! Validators for arrays, vectors, and other collections.

pub mod elements;
pub mod size;
pub mod structure;

// Re-export size validators
pub use size::{exact_size, max_size, min_size, not_empty_collection, ExactSize, MaxSize, MinSize, NotEmptyCollection};

// Re-export element validators
pub use elements::{all, any, contains_element, unique, All, Any, ContainsElement, Unique};

// Re-export structure validators
pub use structure::{has_key, HasKey};

pub mod prelude {
    pub use super::elements::*;
    pub use super::size::*;
    pub use super::structure::*;
}
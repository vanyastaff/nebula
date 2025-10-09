//! Collection validators
//!
//! Validators for arrays, vectors, and other collections.

pub mod elements;
pub mod size;
pub mod structure;

// Re-export size validators
pub use size::{
    ExactSize, MaxSize, MinSize, NotEmptyCollection, exact_size, max_size, min_size,
    not_empty_collection,
};

// Re-export element validators
pub use elements::{All, Any, ContainsElement, Unique, all, any, contains_element, unique};

// Re-export structure validators
pub use structure::{HasKey, has_key};

pub mod prelude {
    pub use super::elements::*;
    pub use super::size::*;
    pub use super::structure::*;
}

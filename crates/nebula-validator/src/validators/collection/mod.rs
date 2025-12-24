//! Collection validators
//!
//! Validators for arrays, vectors, and other collections.

pub mod elements;
pub mod size;
pub mod structure;

// Re-export size validators
pub use size::{
    ExactSize, MaxSize, MinSize, NotEmptyCollection, SizeRange, exact_size, max_size, min_size,
    not_empty_collection, size_range,
};

// Re-export element validators
pub use elements::{
    All, Any, AtLeastCount, AtMostCount, ContainsAll, ContainsAny, ContainsElement, Count, First,
    Last, None, Nth, Sorted, SortedDescending, Unique, all, any, at_least_count, at_most_count,
    contains_all, contains_any, contains_element, count, first, last, none, nth, sorted,
    sorted_descending, unique,
};

// Re-export structure validators
pub use structure::{HasKey, has_key};

pub mod prelude {
    pub use super::elements::*;
    pub use super::size::*;
    pub use super::structure::*;
}

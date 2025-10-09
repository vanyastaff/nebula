//! String validators
//!
//! This module provides validators for string types.

pub mod content;
pub mod length;
pub mod pattern;

// Re-export length validators
pub use length::{
    ExactLength, LengthRange, MaxLength, MinLength, NotEmpty, exact_length, length_range,
    max_length, min_length, not_empty,
};

// Re-export pattern validators
pub use pattern::{
    Alphabetic, Alphanumeric, Contains, EndsWith, Lowercase, Numeric, StartsWith, Uppercase,
    alphabetic, alphanumeric, contains, ends_with, lowercase, numeric, starts_with, uppercase,
};

// Re-export content validators
pub use content::{Email, MatchesRegex, Url, Uuid, email, matches_regex, url, uuid};

/// Prelude for string validators.
pub mod prelude {
    pub use super::content::*;
    pub use super::length::*;
    pub use super::pattern::*;
}

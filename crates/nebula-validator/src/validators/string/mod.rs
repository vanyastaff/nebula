//! String validators
//!
//! This module provides validators for string types.

pub mod content;
pub mod length;
pub mod pattern;

// Re-export length validators
pub use length::{
    exact_length, length_range, max_length, min_length, not_empty, ExactLength, LengthRange,
    MaxLength, MinLength, NotEmpty,
};

// Re-export pattern validators
pub use pattern::{
    alphabetic, alphanumeric, contains, ends_with, lowercase, numeric, starts_with, uppercase,
    Alphabetic, Alphanumeric, Contains, EndsWith, Lowercase, Numeric, StartsWith, Uppercase,
};

// Re-export content validators
pub use content::{email, matches_regex, url, uuid, Email, MatchesRegex, Url, Uuid};

/// Prelude for string validators.
pub mod prelude {
    pub use super::content::*;
    pub use super::length::*;
    pub use super::pattern::*;
}
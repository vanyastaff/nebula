//! String validators
//!
//! This module provides validators for string types, including:
//! - Length validators (min, max, exact, range)
//! - Pattern validators (contains, starts_with, alphanumeric, etc.)
//! - Content validators (email, URL, regex)
//! - Format validators (UUID, JSON, Base64, Hex, DateTime, etc.)
//! - Domain validators (phone, credit card, IBAN, semver, password, etc.)

// Core string validators
pub mod content;
pub mod length;
pub mod pattern;

// Format validators
mod base64;
mod credit_card;
mod datetime;
mod hex;
mod iban;
mod json;
mod password;
mod phone;
mod semver;
mod slug;
mod uuid;

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
pub use content::{Email, MatchesRegex, Url, email, matches_regex, url};

// Re-export format validators
pub use base64::{Base64, Base64Alphabet};
pub use credit_card::{CardType, CardTypes, CreditCard};
pub use datetime::DateTime;
pub use hex::{Hex, HexCase, RequirePrefixHex};
pub use iban::Iban;
pub use json::Json;
pub use password::Password;
pub use phone::{Phone, PhoneMode};
pub use semver::Semver;
pub use slug::Slug;
pub use uuid::Uuid;

/// Prelude for string validators.
pub mod prelude {
    pub use super::content::*;
    pub use super::length::*;
    pub use super::pattern::*;

    // Format validators
    pub use super::{
        Base64, Base64Alphabet, CardType, CardTypes, CreditCard, DateTime, Hex, HexCase, Iban,
        Json, Password, Phone, PhoneMode, RequirePrefixHex, Semver, Slug, Uuid,
    };
}

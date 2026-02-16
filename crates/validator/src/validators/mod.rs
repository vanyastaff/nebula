//! Built-in validators
//!
//! This module provides a comprehensive set of ready-to-use validators
//! for common validation scenarios.
//!
//! # Categories
//!
//! - **String**: Length, patterns, formats (email, URL, UUID, phone, IBAN, etc.)
//! - **Numeric**: Range, properties (even, odd, positive)
//! - **Collection**: Size, elements, structure
//! - **Logical**: Boolean, nullable
//! - **Network**: IP address, MAC address, port
//!
//! # Examples
//!
//! ```rust,ignore
//! use nebula_validator::prelude::*;
//!
//! // String validation
//! let username = min_length(3).and(max_length(20)).and(alphanumeric());
//!
//! // Numeric validation
//! let age = in_range(18, 100);
//!
//! // Collection validation
//! let tags = min_size(1).and(max_size(10));
//!
//! // Composition
//! let email_validator = not_empty().and(email());
//! ```

// String validators
pub mod base64;
pub mod content;
pub mod credit_card;
pub mod datetime;
pub mod hex;
pub mod iban;
pub mod json_string;
pub mod length;
pub mod password;
pub mod pattern;
pub mod phone;
pub mod semver;
pub mod slug;
pub mod uuid;

// Numeric validators
pub mod divisibility;
pub mod float;
pub mod percentage;
pub mod properties;
pub mod range;

// Collection validators
pub mod elements;
pub mod size;
pub mod structure;

// Network validators
pub mod hostname;
pub mod ip_address;
pub mod mac_address;
pub mod port;

// Time validators
pub mod time;

// Logical validators
pub mod boolean;
pub mod nullable;

// ============================================================================
// RE-EXPORTS: String validators
// ============================================================================

pub use length::{
    ExactLength, LengthRange, MaxLength, MinLength, NotEmpty, exact_length, length_range,
    max_length, min_length, not_empty,
};

pub use pattern::{
    Alphabetic, Alphanumeric, Contains, EndsWith, Lowercase, Numeric, StartsWith, Uppercase,
    alphabetic, alphanumeric, contains, ends_with, lowercase, numeric, starts_with, uppercase,
};

pub use content::{Email, MatchesRegex, Url, email, matches_regex, url};

pub use base64::{Base64, Base64Alphabet};
pub use credit_card::{CardType, CardTypes, CreditCard};
pub use datetime::DateTime;
pub use hex::{Hex, HexCase, RequirePrefixHex};
pub use iban::Iban;
pub use json_string::Json;
pub use password::Password;
pub use phone::{Phone, PhoneMode};
pub use semver::Semver;
pub use slug::Slug;
pub use uuid::Uuid;

// ============================================================================
// RE-EXPORTS: Numeric validators
// ============================================================================

pub use divisibility::{DivisibleBy, divisible_by, multiple_of};

pub use float::{
    DecimalPlaces, Finite, FiniteF32, NotNaN, NotNaNF32, decimal_places, finite, finite_f32,
    not_nan, not_nan_f32,
};

pub use percentage::{
    Percentage, Percentage100, Percentage100F64, PercentageF32, percentage, percentage_100,
    percentage_100_f64, percentage_f32,
};

pub use properties::{
    Even, Negative, NonZero, Odd, Positive, PowerOfTwo, PowerOfTwoU64, even, negative, non_zero,
    odd, positive, power_of_two, power_of_two_u64,
};

pub use range::{
    ExclusiveRange, GreaterThan, InRange, LessThan, Max, Min, exclusive_range, greater_than,
    in_range, less_than, max, min,
};

// ============================================================================
// RE-EXPORTS: Collection validators
// ============================================================================

pub use size::{
    ExactSize, MaxSize, MinSize, NotEmptyCollection, SizeRange, exact_size, max_size, min_size,
    not_empty_collection, size_range,
};

#[allow(deprecated)]
pub use elements::{
    All, Any, AtLeastCount, AtMostCount, ContainsAll, ContainsAny, ContainsElement, Count, First,
    Last, NoneOf, Nth, Sorted, SortedDescending, Unique, all, any, at_least_count, at_most_count,
    contains_all, contains_any, contains_element, count, first, last, none, none_of, nth, sorted,
    sorted_descending, unique,
};

pub use structure::{HasKey, has_key};

// ============================================================================
// RE-EXPORTS: Network validators
// ============================================================================

pub use hostname::{Hostname, hostname};
pub use ip_address::{IpAddress, Ipv4, Ipv6};
pub use mac_address::MacAddress;
pub use port::Port;
pub use time::{TimeOnly, time_only};

// ============================================================================
// RE-EXPORTS: Logical validators
// ============================================================================

pub use boolean::{IsFalse, IsTrue, is_false, is_true};
pub use nullable::{NotNull, Required, not_null, required};

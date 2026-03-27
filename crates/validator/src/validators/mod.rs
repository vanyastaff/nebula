//! Built-in validators
//!
//! This module provides a set of ready-to-use validators for common
//! validation scenarios.
//!
//! # Categories
//!
//! - **String** ([`length`](crate::validators::length), [`pattern`](crate::validators::pattern), [`content`](crate::validators::content)): length bounds, character patterns,
//!   email/URL/regex matching
//! - **Numeric** ([`range`](crate::validators::range)): min, max, in_range, greater_than, less_than
//! - **Collection** ([`size`](crate::validators::size)): size bounds for `Vec`, slices, etc.
//! - **Logical** ([`boolean`](crate::validators::boolean), [`nullable`](crate::validators::nullable)): boolean checks, required/not-null
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
pub mod content;
pub mod length;
pub mod pattern;

// Numeric validators
pub mod range;

// Collection validators
pub mod size;

// Logical validators
pub mod boolean;
pub mod nullable;

// Network validators
pub mod network;

// Temporal validators (date, time, datetime, uuid)
pub mod temporal;

pub use length::{
    ExactLength, LengthRange, MaxLength, MinLength, NotEmpty, exact_length, exact_length_bytes,
    length_range, length_range_bytes, max_length, max_length_bytes, min_length, min_length_bytes,
    not_empty,
};

pub use pattern::{
    Alphabetic, Alphanumeric, Contains, EndsWith, Lowercase, Numeric, StartsWith, Uppercase,
    alphabetic, alphanumeric, contains, ends_with, lowercase, numeric, starts_with, uppercase,
};

pub use content::{Email, MatchesRegex, Url, email, matches_regex, url};

pub use range::{
    ExclusiveRange, GreaterThan, InRange, LessThan, Max, Min, exclusive_range, greater_than,
    in_range, in_range_f64, in_range_i64, less_than, max, max_f64, max_i64, min, min_f64, min_i64,
};

pub use size::{
    ExactSize, MaxSize, MinSize, NotEmptyCollection, SizeRange, exact_size, max_size, min_size,
    not_empty_collection, size_range,
};

pub use boolean::{IsFalse, IsTrue, is_false, is_true};
pub use nullable::{NotNull, Required, not_null, required};

pub use network::{Hostname, IpAddr, Ipv4, Ipv6, hostname, ip_addr, ipv4, ipv6};
pub use temporal::{Date, DateTime, Time, Uuid, date, date_time, time, uuid};

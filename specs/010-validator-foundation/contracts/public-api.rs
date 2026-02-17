// ============================================================================
// PUBLIC API CONTRACT: nebula-validator (after Phase 0 restructuring)
// ============================================================================
// This file documents the target public API surface.
// It is NOT compiled — it serves as the contract for implementation.
// ============================================================================

// ---------------------------------------------------------------------------
// 1. PRELUDE — single import for consumers
// ---------------------------------------------------------------------------
// use nebula_validator::prelude::*;

pub mod prelude {
    // Core traits
    pub use crate::foundation::{
        AsValidatable, Validate, ValidateExt,
        ValidationError, ValidationErrors, ErrorSeverity,
        ValidatorMetadata, ValidationComplexity,
    };

    // String validators (factory functions)
    pub use crate::validators::{
        min_length, max_length, exact_length, length_range, not_empty,
        contains, starts_with, ends_with,
        alphabetic, alphanumeric, numeric, lowercase, uppercase,
        email, url, matches_regex,
        hostname, time_only,
        DateTime, Uuid, Semver, Slug, Hex, Base64,
        Password, Phone, CreditCard, Iban, Json,
    };

    // Numeric validators
    pub use crate::validators::{
        min, max, in_range, greater_than, less_than, exclusive_range,
        divisible_by, multiple_of,
        positive, negative, non_zero, even, odd, power_of_two,
        finite, not_nan, decimal_places,
        percentage, percentage_100,
    };

    // Collection validators
    pub use crate::validators::{
        min_size, max_size, exact_size, size_range, not_empty_collection,
        all, any, none_of, count, unique, sorted, contains_element,
        has_key,
    };

    // Network validators
    pub use crate::validators::{
        IpAddress, Ipv4, Ipv6, Port, MacAddress, Hostname,
    };

    // Logical validators
    pub use crate::validators::{
        is_true, is_false, required, not_null,
    };

    // JSON convenience (serde feature)
    #[cfg(feature = "serde")]
    pub use crate::json::*;

    // Combinators
    pub use crate::combinators::{
        and, or, not, optional, when, unless, each, lazy,
        with_message, with_code,
        field, named_field,
    };

    #[cfg(feature = "serde")]
    pub use crate::combinators::{json_field, json_field_optional};
}

// ---------------------------------------------------------------------------
// 2. JSON MODULE — turbofish-free collection validators
// ---------------------------------------------------------------------------
// #[cfg(feature = "serde")]
pub mod json {
    pub type JsonMinSize = crate::validators::MinSize<serde_json::Value>;
    pub type JsonMaxSize = crate::validators::MaxSize<serde_json::Value>;
    pub type JsonExactSize = crate::validators::ExactSize<serde_json::Value>;
    pub type JsonSizeRange = crate::validators::SizeRange<serde_json::Value>;

    pub fn json_min_size(min: usize) -> JsonMinSize { todo!() }
    pub fn json_max_size(max: usize) -> JsonMaxSize { todo!() }
    pub fn json_exact_size(size: usize) -> JsonExactSize { todo!() }
    pub fn json_size_range(min: usize, max: usize) -> JsonSizeRange { todo!() }
}

// ---------------------------------------------------------------------------
// 3. NEW VALIDATORS
// ---------------------------------------------------------------------------

/// RFC 1123 hostname validator.
pub struct Hostname;
pub fn hostname() -> Hostname { Hostname }

/// Time-only validator (HH:MM:SS with optional ms and timezone).
pub struct TimeOnly {
    allow_milliseconds: bool,
    require_timezone: bool,
}

impl TimeOnly {
    pub fn new() -> Self { todo!() }
    pub fn require_timezone(self) -> Self { todo!() }
}

pub fn time_only() -> TimeOnly { TimeOnly::new() }

/// DateTime extension — date-only mode.
impl DateTime {
    pub fn date_only() -> Self { todo!() }
}

// ---------------------------------------------------------------------------
// 4. FEATURE FLAGS (Cargo.toml)
// ---------------------------------------------------------------------------
// [features]
// default = ["serde"]
// serde = []
// caching = ["dep:moka"]
// optimizer = []
// full = ["serde", "caching", "optimizer"]
//
// [dependencies]
// moka = { version = "0.12", features = ["sync"], optional = true }

// ---------------------------------------------------------------------------
// 5. REMOVED FROM PUBLIC API
// ---------------------------------------------------------------------------
// - AsyncValidate trait                    (deleted)
// - Refined<T, V> type                    (deleted)
// - Parameter<T, S> type-state            (deleted)
// - Unvalidated / Validated<V> markers    (deleted)
// - ParameterBuilder<T, S>               (deleted)
// - Map<V, F> combinator                 (deleted)
// - NonEmptyString, EmailAddress, etc.   (deleted, were Refined aliases)
// - pub mod core (alias)                 (no alias, clean break)

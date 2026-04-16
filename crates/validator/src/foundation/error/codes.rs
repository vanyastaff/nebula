//! Canonical error codes used by built-in validators and combinators.

/// Value is required but was missing or empty.
pub const REQUIRED: &str = "required";
/// Value is shorter than the minimum allowed length.
pub const MIN_LENGTH: &str = "min_length";
/// Value exceeds the maximum allowed length.
pub const MAX_LENGTH: &str = "max_length";
/// Value does not match the expected format.
pub const INVALID_FORMAT: &str = "invalid_format";
/// Value has an unexpected type.
pub const TYPE_MISMATCH: &str = "type_mismatch";
/// Numeric value is outside the allowed range.
pub const OUT_OF_RANGE: &str = "out_of_range";
/// Value does not have the exact required length.
pub const EXACT_LENGTH: &str = "exact_length";
/// Value length falls outside the allowed range.
pub const LENGTH_RANGE: &str = "length_range";
/// Custom validation error.
pub const CUSTOM: &str = "custom";

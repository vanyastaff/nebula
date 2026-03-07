//! Core traits for the subtype system.
//!
//! Inspired by paramdef's trait-based architecture, these traits provide:
//! - Compile-time type safety
//! - Zero-cost abstractions
//! - Flexible composition

use std::fmt::Debug;

use serde::Serialize;
use serde::de::DeserializeOwned;

/// Marker trait for numeric types that can be used with number parameters.
///
/// This provides compile-time safety for numeric operations.
pub trait Numeric:
    Copy + PartialOrd + Debug + Send + Sync + Serialize + DeserializeOwned + 'static
{
    /// Convert from f64 for legacy interop.
    fn from_f64(v: f64) -> Self;

    /// Convert to f64 for legacy interop.
    fn to_f64(self) -> f64;

    /// Parse this numeric type from a JSON value without losing integer semantics.
    fn from_json(value: &serde_json::Value) -> Option<Self>;

    /// Check if this is an integer type.
    fn is_integer() -> bool;
}

impl Numeric for f64 {
    #[inline]
    fn from_f64(v: f64) -> Self {
        v
    }

    #[inline]
    fn to_f64(self) -> f64 {
        self
    }

    #[inline]
    fn from_json(value: &serde_json::Value) -> Option<Self> {
        value.as_f64()
    }

    #[inline]
    fn is_integer() -> bool {
        false
    }
}

impl Numeric for i64 {
    #[inline]
    #[allow(clippy::cast_possible_truncation)]
    fn from_f64(v: f64) -> Self {
        v as i64
    }

    #[inline]
    fn to_f64(self) -> f64 {
        self as f64
    }

    #[inline]
    fn from_json(value: &serde_json::Value) -> Option<Self> {
        value
            .as_i64()
            .or_else(|| value.as_u64().and_then(|v| i64::try_from(v).ok()))
    }

    #[inline]
    fn is_integer() -> bool {
        true
    }
}

impl Numeric for u16 {
    #[inline]
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    fn from_f64(v: f64) -> Self {
        v as u16
    }

    #[inline]
    fn to_f64(self) -> f64 {
        f64::from(self)
    }

    #[inline]
    fn from_json(value: &serde_json::Value) -> Option<Self> {
        value.as_u64().and_then(|v| u16::try_from(v).ok())
    }

    #[inline]
    fn is_integer() -> bool {
        true
    }
}

/// Marker trait for integer-only subtypes.
///
/// Used for compile-time constraints on subtypes like `Port` or `Index`.
pub trait IntegerSubtype {}

/// Marker trait for float-only subtypes.
///
/// Used for compile-time constraints on subtypes like `Percentage` or `Angle`.
pub trait FloatSubtype {}

/// Core trait for boolean/checkbox subtypes.
///
/// Boolean subtypes provide semantic meaning and defaults for checkbox-style
/// parameters.
pub trait BooleanSubtype: Debug + Clone + Copy + Default + Send + Sync + 'static {
    /// The canonical subtype name (for example, `toggle` or `consent`).
    fn name() -> &'static str;

    /// Human-readable description.
    fn description() -> &'static str;

    /// Optional inline label for checkbox UI.
    fn label() -> Option<&'static str> {
        None
    }

    /// Optional helper text for checkbox UI.
    fn help_text() -> Option<&'static str> {
        None
    }

    /// Optional default value for this semantic subtype.
    fn default_value() -> Option<bool> {
        None
    }
}

/// Core trait for text subtypes.
///
/// Text subtypes provide semantic meaning and metadata for text parameters.
pub trait TextSubtype: Debug + Clone + Copy + Default + Send + Sync + 'static {
    /// The name of this subtype (e.g., "email", "url")
    fn name() -> &'static str;

    /// Human-readable description
    fn description() -> &'static str;

    /// Optional regex pattern for validation
    fn pattern() -> Option<&'static str> {
        None
    }

    /// Whether this subtype represents sensitive data
    fn is_sensitive() -> bool {
        false
    }

    /// Whether this is a code/markup type
    fn is_code() -> bool {
        false
    }

    /// Placeholder text for UI
    fn placeholder() -> Option<&'static str> {
        None
    }

    /// Whether this should use multiline input
    fn is_multiline() -> bool {
        false
    }
}

/// Core trait for number subtypes.
///
/// Number subtypes provide semantic meaning and constraints for numeric parameters.
pub trait NumberSubtype: Debug + Clone + Copy + Default + Send + Sync + 'static {
    /// The numeric type this subtype works with (for compile-time constraints)
    type Value: Numeric;

    /// The name of this subtype (e.g., "port", "percentage")
    fn name() -> &'static str;

    /// Human-readable description
    fn description() -> &'static str;

    /// Default range constraints, if any
    fn default_range() -> Option<(Self::Value, Self::Value)> {
        None
    }

    /// Default step for UI sliders
    fn default_step() -> Option<Self::Value> {
        None
    }

    /// Whether this is a percentage (needs % display)
    fn is_percentage() -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_numeric_f64() {
        assert_eq!(f64::from_f64(3.14), 3.14);
        assert_eq!(3.14f64.to_f64(), 3.14);
        assert!(!f64::is_integer());
    }

    #[test]
    fn test_numeric_i64() {
        assert_eq!(i64::from_f64(42.7), 42);
        assert_eq!(42i64.to_f64(), 42.0);
        assert_eq!(i64::from_json(&serde_json::json!(42)), Some(42));
        assert!(i64::is_integer());
    }

    #[test]
    fn test_numeric_u16() {
        assert_eq!(u16::from_f64(8080.0), 8080);
        assert_eq!(8080u16.to_f64(), 8080.0);
        assert_eq!(u16::from_json(&serde_json::json!(8080)), Some(8080));
        assert_eq!(u16::from_json(&serde_json::json!(-1)), None);
        assert!(u16::is_integer());
    }
}

//! Sealed category traits for type-safe Value operations
//!
//! This module provides sealed traits that categorize Value types at the type level.
//! Using sealed traits prevents external crates from implementing these categories,
//! ensuring type-level guarantees about value categories.
//!
//! ## Categories
//!
//! - [`ScalarValue`] - Primitive types (Boolean, Integer, Float, Decimal, Text, Bytes)
//! - [`NumericValue`] - Numeric types (Integer, Float, Decimal)
//! - [`CollectionValue`] - Container types (Array, Object)
//! - [`TemporalValue`] - Date/time types (Date, Time, DateTime, Duration)
//!
//! ## Usage
//!
//! These traits enable generic functions that operate only on specific categories:
//!
//! ```ignore
//! use nebula_value::core::category::NumericValue;
//!
//! fn sum_numerics<T: NumericValue>(values: &[T]) -> f64 {
//!     // Only numeric types can be passed here
//!     values.iter().map(|v| v.to_f64()).sum()
//! }
//! ```

use crate::Boolean;
use crate::collections::{Array, Object};
use crate::scalar::{Bytes, Float, Integer, Text};

#[cfg(feature = "temporal")]
use crate::temporal::{Date, DateTime, Duration, Time};

// ============================================================================
// Sealed module - prevents external implementations
// ============================================================================

mod sealed {
    pub trait Sealed {}

    // Scalar implementations
    impl Sealed for super::Boolean {}
    impl Sealed for super::Integer {}
    impl Sealed for super::Float {}
    impl Sealed for rust_decimal::Decimal {}
    impl Sealed for super::Text {}
    impl Sealed for super::Bytes {}

    // Collection implementations
    impl Sealed for super::Array {}
    impl Sealed for super::Object {}

    // Temporal implementations
    #[cfg(feature = "temporal")]
    impl Sealed for super::Date {}
    #[cfg(feature = "temporal")]
    impl Sealed for super::Time {}
    #[cfg(feature = "temporal")]
    impl Sealed for super::DateTime {}
    #[cfg(feature = "temporal")]
    impl Sealed for super::Duration {}
}

// ============================================================================
// Category Traits
// ============================================================================

/// Marker trait for scalar (primitive) value types
///
/// Scalar values are simple, non-container types:
/// - Boolean
/// - Integer
/// - Float
/// - Decimal
/// - Text
/// - Bytes
///
/// This trait is sealed and cannot be implemented outside this crate.
pub trait ScalarValue: sealed::Sealed {
    /// Get the category name for this scalar type
    fn category_name() -> &'static str {
        "scalar"
    }
}

impl ScalarValue for Boolean {}
impl ScalarValue for Integer {}
impl ScalarValue for Float {}
impl ScalarValue for rust_decimal::Decimal {}
impl ScalarValue for Text {}
impl ScalarValue for Bytes {}

/// Marker trait for numeric value types
///
/// Numeric values support arithmetic operations:
/// - Integer (i64)
/// - Float (f64)
/// - Decimal (arbitrary precision)
///
/// This trait is sealed and cannot be implemented outside this crate.
pub trait NumericValue: ScalarValue {
    /// Convert to f64 for arithmetic operations
    fn to_f64(&self) -> f64;

    /// Check if this is an integer type
    fn is_integer(&self) -> bool {
        false
    }

    /// Check if this is a floating-point type
    fn is_float(&self) -> bool {
        false
    }

    /// Check if this is a decimal type
    fn is_decimal(&self) -> bool {
        false
    }
}

impl NumericValue for Integer {
    fn to_f64(&self) -> f64 {
        self.value() as f64
    }

    fn is_integer(&self) -> bool {
        true
    }
}

impl NumericValue for Float {
    fn to_f64(&self) -> f64 {
        self.value()
    }

    fn is_float(&self) -> bool {
        true
    }
}

impl NumericValue for rust_decimal::Decimal {
    fn to_f64(&self) -> f64 {
        use rust_decimal::prelude::ToPrimitive;
        ToPrimitive::to_f64(self).unwrap_or(f64::NAN)
    }

    fn is_decimal(&self) -> bool {
        true
    }
}

/// Marker trait for collection value types
///
/// Collection values contain other values:
/// - Array (ordered list)
/// - Object (key-value map)
///
/// This trait is sealed and cannot be implemented outside this crate.
pub trait CollectionValue: sealed::Sealed {
    /// Get the number of elements in the collection
    fn collection_len(&self) -> usize;

    /// Check if the collection is empty
    fn collection_is_empty(&self) -> bool {
        self.collection_len() == 0
    }

    /// Get the category name for this collection type
    fn category_name() -> &'static str {
        "collection"
    }
}

impl CollectionValue for Array {
    fn collection_len(&self) -> usize {
        self.len()
    }
}

impl CollectionValue for Object {
    fn collection_len(&self) -> usize {
        self.len()
    }
}

/// Marker trait for temporal (date/time) value types
///
/// Temporal values represent points in time or durations:
/// - Date
/// - Time
/// - DateTime
/// - Duration
///
/// This trait is sealed and cannot be implemented outside this crate.
#[cfg(feature = "temporal")]
pub trait TemporalValue: sealed::Sealed {
    /// Get the category name for this temporal type
    fn category_name() -> &'static str {
        "temporal"
    }
}

#[cfg(feature = "temporal")]
impl TemporalValue for Date {}

#[cfg(feature = "temporal")]
impl TemporalValue for Time {}

#[cfg(feature = "temporal")]
impl TemporalValue for DateTime {}

#[cfg(feature = "temporal")]
impl TemporalValue for Duration {}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scalar_category() {
        fn accepts_scalar<T: ScalarValue>(_value: &T) {}

        let int = Integer::new(42);
        let float = Float::new(3.14);
        let text = Text::from("hello");

        accepts_scalar(&int);
        accepts_scalar(&float);
        accepts_scalar(&text);
    }

    #[test]
    fn test_numeric_category() {
        fn sum_numerics<T: NumericValue>(values: &[T]) -> f64 {
            values.iter().map(|v| v.to_f64()).sum()
        }

        let integers = vec![Integer::new(1), Integer::new(2), Integer::new(3)];
        assert_eq!(sum_numerics(&integers), 6.0);

        let floats = vec![Float::new(1.5), Float::new(2.5)];
        assert_eq!(sum_numerics(&floats), 4.0);
    }

    #[test]
    fn test_numeric_type_checks() {
        let int = Integer::new(42);
        let float = Float::new(3.14);
        let decimal = rust_decimal::Decimal::new(100, 2); // 1.00

        assert!(NumericValue::is_integer(&int));
        assert!(!NumericValue::is_float(&int));
        assert!(!NumericValue::is_decimal(&int));

        assert!(!NumericValue::is_integer(&float));
        assert!(NumericValue::is_float(&float));
        assert!(!NumericValue::is_decimal(&float));

        // Use explicit trait method syntax to avoid collision with Decimal::is_integer()
        assert!(!NumericValue::is_integer(&decimal));
        assert!(!NumericValue::is_float(&decimal));
        assert!(NumericValue::is_decimal(&decimal));
    }

    #[test]
    fn test_collection_category() {
        fn collection_size<T: CollectionValue>(value: &T) -> usize {
            value.collection_len()
        }

        let array = Array::from_vec(vec![crate::Value::integer(1), crate::Value::integer(2)]);
        let object = Object::from_iter(vec![("a".to_string(), crate::Value::integer(1))]);

        assert_eq!(collection_size(&array), 2);
        assert_eq!(collection_size(&object), 1);
    }

    // This test verifies that the sealed trait pattern works
    // by ensuring compile-time errors for external implementations.
    // Uncomment to verify the pattern:
    //
    // ```compile_fail
    // struct MyType;
    // impl super::sealed::Sealed for MyType {} // ERROR: module is private
    // impl ScalarValue for MyType {} // ERROR: Sealed is not implemented
    // ```
}

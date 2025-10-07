//! Trait-based Value type extraction
//!
//! This module provides automatic type extraction from nebula-value::Value
//! using Rust's trait system.

use crate::core::{ValidationError, TypedValidator};
use nebula_value::Value;

// ============================================================================
// EXTRACT TRAIT - Automatic type extraction from Value
// ============================================================================

/// Trait for types that can be extracted from a Value.
///
/// This allows automatic conversion from Value to the target type
/// without manual matching or wrapping.
///
/// Note: This trait doesn't require `Sized` to support unsized types like `str`.
pub trait Extract {
    /// Extract a reference to Self from a Value.
    ///
    /// Returns an error if the Value contains a different type.
    fn extract(value: &Value) -> Result<&Self, ValidationError>;
}

// ============================================================================
// IMPLEMENTATIONS FOR PRIMITIVE TYPES
// ============================================================================

impl Extract for str {
    fn extract(value: &Value) -> Result<&Self, ValidationError> {
        match value {
            Value::Text(s) => Ok(s.as_str()),
            _ => Err(ValidationError::type_mismatch(
                "",
                "string",
                value.kind().name(),
            )),
        }
    }
}

impl Extract for bool {
    fn extract(value: &Value) -> Result<&Self, ValidationError> {
        match value {
            Value::Boolean(b) => Ok(b),
            _ => Err(ValidationError::type_mismatch(
                "",
                "boolean",
                value.kind().name(),
            )),
        }
    }
}

// For numeric types, we need to handle the wrapper types
impl Extract for i64 {
    fn extract(value: &Value) -> Result<&Self, ValidationError> {
        match value {
            Value::Integer(i) => {
                // Integer wraps i64, so we need a way to get &i64
                // This is a limitation - we'll need to extract by value instead
                // For now, return error pointing to the need for owned extraction
                Err(ValidationError::new(
                    "extract_unsupported",
                    "i64 extraction requires owned value, use extract_owned instead",
                ))
            }
            _ => Err(ValidationError::type_mismatch(
                "",
                "integer",
                value.kind().name(),
            )),
        }
    }
}

// ============================================================================
// OWNED EXTRACTION - For types that need owned values
// ============================================================================

/// Trait for extracting owned values from Value.
///
/// Use this when the type is wrapped and can't provide a direct reference.
pub trait ExtractOwned: Sized {
    /// Extract an owned Self from a Value.
    fn extract_owned(value: &Value) -> Result<Self, ValidationError>;
}

impl ExtractOwned for i64 {
    fn extract_owned(value: &Value) -> Result<Self, ValidationError> {
        match value {
            Value::Integer(i) => Ok(i.value()),
            Value::Float(f) => Ok(f.value() as i64),
            Value::Decimal(d) => d
                .to_string()
                .parse()
                .map_err(|_| ValidationError::new("parse_error", "Cannot convert Decimal to i64")),
            _ => Err(ValidationError::type_mismatch(
                "",
                "number",
                value.kind().name(),
            )),
        }
    }
}

impl ExtractOwned for f64 {
    fn extract_owned(value: &Value) -> Result<Self, ValidationError> {
        match value {
            Value::Float(f) => Ok(f.value()),
            Value::Integer(i) => Ok(i.value() as f64),
            Value::Decimal(d) => d
                .to_string()
                .parse()
                .map_err(|_| ValidationError::new("parse_error", "Cannot convert Decimal to f64")),
            _ => Err(ValidationError::type_mismatch(
                "",
                "number",
                value.kind().name(),
            )),
        }
    }
}

impl ExtractOwned for bool {
    fn extract_owned(value: &Value) -> Result<Self, ValidationError> {
        match value {
            Value::Boolean(b) => Ok(*b),
            _ => Err(ValidationError::type_mismatch(
                "",
                "boolean",
                value.kind().name(),
            )),
        }
    }
}

// ============================================================================
// ARRAY EXTRACTION
// ============================================================================

impl Extract for nebula_value::Array {
    fn extract(value: &Value) -> Result<&Self, ValidationError> {
        match value {
            Value::Array(arr) => Ok(arr),
            _ => Err(ValidationError::type_mismatch(
                "",
                "array",
                value.kind().name(),
            )),
        }
    }
}

// ============================================================================
// AUTOMATIC VALUE ADAPTER
// ============================================================================

/// Wrapper that automatically adapts a validator to work with Value.
///
/// This uses the Extract trait to automatically extract the correct type.
pub struct ValueAdapter<V> {
    inner: V,
}

impl<V> ValueAdapter<V> {
    pub fn new(inner: V) -> Self {
        Self { inner }
    }
}

// For validators with extractable Input types
impl<V> TypedValidator for ValueAdapter<V>
where
    V: TypedValidator,
    V::Input: Extract,
    V::Error: Into<ValidationError>,
{
    type Input = Value;
    type Output = V::Output;
    type Error = ValidationError;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        let extracted = <V::Input as Extract>::extract(input)?;
        self.inner.validate(extracted).map_err(Into::into)
    }

    fn metadata(&self) -> crate::core::ValidatorMetadata {
        self.inner.metadata()
    }
}

// ============================================================================
// EXTENSION TRAIT - Ergonomic API
// ============================================================================

/// Extension trait that adds `.for_value()` to any validator.
pub trait ValueValidatorExt: TypedValidator + Sized {
    /// Wrap this validator to work with nebula-value::Value.
    ///
    /// # Example
    /// ```rust
    /// use nebula_validator::validators::string::min_length;
    /// use nebula_validator::bridge::ValueValidatorExt;
    ///
    /// let validator = min_length(5).for_value();
    /// // Now validator accepts Value instead of &str
    /// ```
    fn for_value(self) -> ValueAdapter<Self>
    where
        Self::Input: Extract,
        Self::Error: Into<ValidationError>,
    {
        ValueAdapter::new(self)
    }
}

// Blanket implementation for all validators
impl<T: TypedValidator> ValueValidatorExt for T {}

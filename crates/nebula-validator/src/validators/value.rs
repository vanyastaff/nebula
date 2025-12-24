//! Value type wrappers for nebula-value::Value
//!
//! These wrappers extract typed values from `Value` and delegate to typed validators.

use crate::core::{TypedValidator, ValidationError};
use nebula_value::Value;

/// Wrapper that extracts `&str` from `Value::Text` and validates it
pub struct ValueString<V> {
    validator: V,
}

impl<V> ValueString<V> {
    /// Create a new ValueString wrapper
    pub fn new(validator: V) -> Self {
        Self { validator }
    }

    /// Get reference to inner validator
    pub fn inner(&self) -> &V {
        &self.validator
    }
}

impl<V> TypedValidator for ValueString<V>
where
    V: TypedValidator<Input = str, Output = (), Error = ValidationError>,
{
    type Input = Value;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Value) -> Result<(), ValidationError> {
        let s = input
            .as_str()
            .ok_or_else(|| ValidationError::new("type_error", "Expected text value"))?;
        self.validator.validate(s)
    }
}

/// Convenience function to create a ValueString validator
pub fn value_string<V>(validator: V) -> ValueString<V>
where
    V: TypedValidator<Input = str, Output = (), Error = ValidationError>,
{
    ValueString::new(validator)
}

/// Wrapper that extracts `i64` from `Value::Integer` and validates it
pub struct ValueInteger<V> {
    validator: V,
}

impl<V> ValueInteger<V> {
    /// Create a new ValueInteger wrapper
    pub fn new(validator: V) -> Self {
        Self { validator }
    }

    /// Get reference to inner validator
    pub fn inner(&self) -> &V {
        &self.validator
    }
}

impl<V> TypedValidator for ValueInteger<V>
where
    V: TypedValidator<Input = i64, Output = (), Error = ValidationError>,
{
    type Input = Value;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Value) -> Result<(), ValidationError> {
        let n = input
            .as_integer()
            .ok_or_else(|| ValidationError::new("type_error", "Expected integer value"))?;
        self.validator.validate(&n.value())
    }
}

/// Convenience function to create a ValueInteger validator
pub fn value_integer<V>(validator: V) -> ValueInteger<V>
where
    V: TypedValidator<Input = i64, Output = (), Error = ValidationError>,
{
    ValueInteger::new(validator)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validators::string::min_length;

    #[test]
    fn test_value_string_valid() {
        let validator = value_string(min_length(3));
        assert!(validator.validate(&Value::text("hello")).is_ok());
    }

    #[test]
    fn test_value_string_invalid() {
        let validator = value_string(min_length(3));
        assert!(validator.validate(&Value::text("hi")).is_err());
    }

    #[test]
    fn test_value_string_wrong_type() {
        let validator = value_string(min_length(3));
        assert!(
            validator
                .validate(&Value::Integer(nebula_value::Integer::new(42)))
                .is_err()
        );
    }

    #[test]
    fn test_value_integer_valid() {
        use crate::validators::numeric::min;
        let validator = value_integer(min(18i64));
        assert!(validator.validate(&Value::integer(25)).is_ok());
    }

    #[test]
    fn test_value_integer_invalid() {
        use crate::validators::numeric::min;
        let validator = value_integer(min(18i64));
        assert!(validator.validate(&Value::integer(15)).is_err());
    }

    #[test]
    fn test_value_integer_wrong_type() {
        use crate::validators::numeric::min;
        let validator = value_integer(min(18i64));
        assert!(validator.validate(&Value::text("hello")).is_err());
    }
}

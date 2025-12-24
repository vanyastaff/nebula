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

/// Wrapper that extracts `f64` from `Value::Float` (with lossy conversion) and validates it
pub struct ValueFloat<V> {
    validator: V,
}

impl<V> ValueFloat<V> {
    /// Create a new ValueFloat wrapper
    pub fn new(validator: V) -> Self {
        Self { validator }
    }

    /// Get reference to inner validator
    pub fn inner(&self) -> &V {
        &self.validator
    }
}

impl<V> TypedValidator for ValueFloat<V>
where
    V: TypedValidator<Input = f64, Output = (), Error = ValidationError>,
{
    type Input = Value;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Value) -> Result<(), ValidationError> {
        let n = input
            .as_float_lossy()
            .ok_or_else(|| ValidationError::new("type_error", "Expected numeric value"))?;
        self.validator.validate(&n.value())
    }
}

/// Convenience function to create a ValueFloat validator
pub fn value_float<V>(validator: V) -> ValueFloat<V>
where
    V: TypedValidator<Input = f64, Output = (), Error = ValidationError>,
{
    ValueFloat::new(validator)
}

/// Wrapper that extracts `bool` from `Value::Boolean` and validates it
pub struct ValueBoolean<V> {
    validator: V,
}

impl<V> ValueBoolean<V> {
    /// Create a new ValueBoolean wrapper
    pub fn new(validator: V) -> Self {
        Self { validator }
    }

    /// Get reference to inner validator
    pub fn inner(&self) -> &V {
        &self.validator
    }
}

impl<V> TypedValidator for ValueBoolean<V>
where
    V: TypedValidator<Input = bool, Output = (), Error = ValidationError>,
{
    type Input = Value;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Value) -> Result<(), ValidationError> {
        let b = input
            .as_boolean()
            .ok_or_else(|| ValidationError::new("type_error", "Expected boolean value"))?;
        self.validator.validate(&b)
    }
}

/// Convenience function to create a ValueBoolean validator
pub fn value_boolean<V>(validator: V) -> ValueBoolean<V>
where
    V: TypedValidator<Input = bool, Output = (), Error = ValidationError>,
{
    ValueBoolean::new(validator)
}

/// Wrapper that extracts `&Array` from `Value::Array` and validates it
pub struct ValueArray<V> {
    validator: V,
}

impl<V> ValueArray<V> {
    /// Create a new ValueArray wrapper
    pub fn new(validator: V) -> Self {
        Self { validator }
    }

    /// Get reference to inner validator
    pub fn inner(&self) -> &V {
        &self.validator
    }
}

impl<V> TypedValidator for ValueArray<V>
where
    V: TypedValidator<Input = nebula_value::Array, Output = (), Error = ValidationError>,
{
    type Input = Value;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Value) -> Result<(), ValidationError> {
        let arr = input
            .as_array()
            .ok_or_else(|| ValidationError::new("type_error", "Expected array value"))?;
        self.validator.validate(arr)
    }
}

/// Convenience function to create a ValueArray validator
pub fn value_array<V>(validator: V) -> ValueArray<V>
where
    V: TypedValidator<Input = nebula_value::Array, Output = (), Error = ValidationError>,
{
    ValueArray::new(validator)
}

/// Wrapper that extracts `&Object` from `Value::Object` and validates it
pub struct ValueObject<V> {
    validator: V,
}

impl<V> ValueObject<V> {
    /// Create a new ValueObject wrapper
    pub fn new(validator: V) -> Self {
        Self { validator }
    }

    /// Get reference to inner validator
    pub fn inner(&self) -> &V {
        &self.validator
    }
}

impl<V> TypedValidator for ValueObject<V>
where
    V: TypedValidator<Input = nebula_value::Object, Output = (), Error = ValidationError>,
{
    type Input = Value;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Value) -> Result<(), ValidationError> {
        let obj = input
            .as_object()
            .ok_or_else(|| ValidationError::new("type_error", "Expected object value"))?;
        self.validator.validate(obj)
    }
}

/// Convenience function to create a ValueObject validator
pub fn value_object<V>(validator: V) -> ValueObject<V>
where
    V: TypedValidator<Input = nebula_value::Object, Output = (), Error = ValidationError>,
{
    ValueObject::new(validator)
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

    #[test]
    fn test_value_float_valid() {
        use crate::validators::numeric::min;
        let validator = value_float(min(0.0f64));
        assert!(validator.validate(&Value::float(3.14)).is_ok());
    }

    #[test]
    fn test_value_float_invalid() {
        use crate::validators::numeric::min;
        let validator = value_float(min(10.0f64));
        assert!(validator.validate(&Value::float(5.0)).is_err());
    }

    #[test]
    fn test_value_float_from_integer() {
        // ValueFloat accepts integers with lossy conversion
        use crate::validators::numeric::min;
        let validator = value_float(min(0.0f64));
        assert!(validator.validate(&Value::integer(42)).is_ok());
    }

    #[test]
    fn test_value_float_wrong_type() {
        use crate::validators::numeric::min;
        let validator = value_float(min(0.0f64));
        assert!(validator.validate(&Value::text("hello")).is_err());
    }

    // Simple test validator for booleans - validates that value is true
    struct MustBeTrue;

    impl TypedValidator for MustBeTrue {
        type Input = bool;
        type Output = ();
        type Error = ValidationError;

        fn validate(&self, input: &bool) -> Result<(), ValidationError> {
            if *input {
                Ok(())
            } else {
                Err(ValidationError::new("must_be_true", "Value must be true"))
            }
        }
    }

    #[test]
    fn test_value_boolean_valid() {
        let validator = value_boolean(MustBeTrue);
        assert!(validator.validate(&Value::Boolean(true)).is_ok());
    }

    #[test]
    fn test_value_boolean_invalid() {
        let validator = value_boolean(MustBeTrue);
        assert!(validator.validate(&Value::Boolean(false)).is_err());
    }

    #[test]
    fn test_value_boolean_wrong_type() {
        let validator = value_boolean(MustBeTrue);
        assert!(validator.validate(&Value::text("true")).is_err());
    }

    // Simple test validator for arrays - validates minimum length
    struct MinArrayLen {
        min: usize,
    }

    impl TypedValidator for MinArrayLen {
        type Input = nebula_value::Array;
        type Output = ();
        type Error = ValidationError;

        fn validate(&self, input: &nebula_value::Array) -> Result<(), ValidationError> {
            if input.len() >= self.min {
                Ok(())
            } else {
                Err(ValidationError::new(
                    "min_array_len",
                    format!("Array must have at least {} elements", self.min),
                ))
            }
        }
    }

    #[test]
    fn test_value_array_valid() {
        let validator = value_array(MinArrayLen { min: 2 });
        let arr = nebula_value::Array::from_iter([Value::integer(1), Value::integer(2)]);
        assert!(validator.validate(&Value::Array(arr)).is_ok());
    }

    #[test]
    fn test_value_array_invalid() {
        let validator = value_array(MinArrayLen { min: 3 });
        let arr = nebula_value::Array::from_iter([Value::integer(1)]);
        assert!(validator.validate(&Value::Array(arr)).is_err());
    }

    #[test]
    fn test_value_array_wrong_type() {
        let validator = value_array(MinArrayLen { min: 1 });
        assert!(validator.validate(&Value::text("not an array")).is_err());
    }

    // Simple test validator for objects - validates required key
    struct RequiredKey {
        key: &'static str,
    }

    impl TypedValidator for RequiredKey {
        type Input = nebula_value::Object;
        type Output = ();
        type Error = ValidationError;

        fn validate(&self, input: &nebula_value::Object) -> Result<(), ValidationError> {
            if input.contains_key(self.key) {
                Ok(())
            } else {
                Err(ValidationError::new(
                    "required_key",
                    format!("Object must contain key '{}'", self.key),
                ))
            }
        }
    }

    #[test]
    fn test_value_object_valid() {
        let validator = value_object(RequiredKey { key: "name" });
        let obj = nebula_value::Object::from_iter([("name".to_string(), Value::text("Alice"))]);
        assert!(validator.validate(&Value::Object(obj)).is_ok());
    }

    #[test]
    fn test_value_object_invalid() {
        let validator = value_object(RequiredKey { key: "email" });
        let obj = nebula_value::Object::from_iter([("name".to_string(), Value::text("Alice"))]);
        assert!(validator.validate(&Value::Object(obj)).is_err());
    }

    #[test]
    fn test_value_object_wrong_type() {
        let validator = value_object(RequiredKey { key: "name" });
        assert!(validator.validate(&Value::text("not an object")).is_err());
    }
}

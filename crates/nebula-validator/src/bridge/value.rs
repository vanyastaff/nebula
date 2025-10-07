//! Bridge to nebula-value for legacy support
//!
//! This module provides adapters to use v2 validators with nebula-value::Value.

use crate::core::{TypedValidator, ValidationError, ValidatorMetadata};
use nebula_value::Value;

// ============================================================================
// VALUE VALIDATOR WRAPPER
// ============================================================================

/// Wraps a typed validator to work with nebula-value::Value.
///
/// This allows using v2 type-safe validators with the old Value type.
///
/// # Examples
///
/// ```rust
/// use nebula_validator::bridge::ValueValidator;
/// use nebula_validator::validators::string::MinLength;
/// use nebula_value::Value;
///
/// let validator = ValueValidator::new(MinLength { min: 5 });
/// let value = Value::text("hello");
/// assert!(validator.validate(&value).is_ok());
/// ```
#[derive(Debug, Clone)]
pub struct ValueValidator<V> {
    inner: V,
}

impl<V> ValueValidator<V> {
    /// Creates a new Value validator wrapper.
    pub fn new(inner: V) -> Self {
        Self { inner }
    }

    /// Returns a reference to the inner validator.
    pub fn inner(&self) -> &V {
        &self.inner
    }

    /// Extracts the inner validator.
    pub fn into_inner(self) -> V {
        self.inner
    }
}

// ============================================================================
// STRING VALIDATOR BRIDGE
// ============================================================================

impl<V> TypedValidator for ValueValidator<V>
where
    V: TypedValidator<Input = str, Output = ()>,
    V::Error: Into<ValidationError>,
{
    type Input = Value;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        match input {
            Value::Text(s) => self.inner.validate(s).map_err(|e| e.into()),
            _ => Err(ValidationError::type_mismatch("", "string", input.kind().name())),
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        self.inner.metadata()
    }
}

// ============================================================================
// NUMERIC VALIDATOR BRIDGES
// ============================================================================

/// Wraps an i64 validator for Value.
#[derive(Debug, Clone)]
pub struct ValueI64Validator<V> {
    inner: V,
}

impl<V> ValueI64Validator<V> {
    pub fn new(inner: V) -> Self {
        Self { inner }
    }
}

impl<V> TypedValidator for ValueI64Validator<V>
where
    V: TypedValidator<Input = i64, Output = ()>,
    V::Error: Into<ValidationError>,
{
    type Input = Value;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        match input {
            Value::Integer(n) => {
                let val = n.value();
                self.inner.validate(&val).map_err(|e| e.into())
            }
            // For Float/Decimal, try converting to i64
            Value::Float(f) => {
                let i = f.value() as i64;
                self.inner.validate(&i).map_err(|e| e.into())
            }
            Value::Decimal(d) => {
                // Best effort conversion via string parsing
                let i = d.to_string().parse::<i64>().unwrap_or(0);
                self.inner.validate(&i).map_err(|e| e.into())
            }
            _ => Err(ValidationError::type_mismatch("", "number", input.kind().name())),
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        self.inner.metadata()
    }
}

/// Wraps an f64 validator for Value.
#[derive(Debug, Clone)]
pub struct ValueF64Validator<V> {
    inner: V,
}

impl<V> ValueF64Validator<V> {
    pub fn new(inner: V) -> Self {
        Self { inner }
    }
}

impl<V> TypedValidator for ValueF64Validator<V>
where
    V: TypedValidator<Input = f64, Output = ()>,
    V::Error: Into<ValidationError>,
{
    type Input = Value;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        match input {
            Value::Float(f) => {
                let val = f.value();
                self.inner.validate(&val).map_err(|e| e.into())
            }
            Value::Integer(i) => {
                let val = i.value() as f64;
                self.inner.validate(&val).map_err(|e| e.into())
            }
            Value::Decimal(d) => {
                // Best effort conversion via string parsing
                let val = d.to_string().parse::<f64>().unwrap_or(0.0);
                self.inner.validate(&val).map_err(|e| e.into())
            }
            _ => Err(ValidationError::type_mismatch("", "number", input.kind().name())),
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        self.inner.metadata()
    }
}

// ============================================================================
// ARRAY VALIDATOR BRIDGE
// ============================================================================

/// Wraps an array validator for Value.
#[derive(Debug, Clone)]
pub struct ValueArrayValidator<V> {
    inner: V,
}

impl<V> ValueArrayValidator<V> {
    pub fn new(inner: V) -> Self {
        Self { inner }
    }
}

impl<V> TypedValidator for ValueArrayValidator<V>
where
    V: TypedValidator<Input = nebula_value::Array, Output = ()>,
    V::Error: Into<ValidationError>,
{
    type Input = Value;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        match input {
            Value::Array(arr) => self.inner.validate(arr).map_err(|e| e.into()),
            _ => Err(ValidationError::type_mismatch("", "array", input.kind().name())),
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        self.inner.metadata()
    }
}

// ============================================================================
// BOOLEAN VALIDATOR BRIDGE
// ============================================================================

/// Wraps a boolean validator for Value.
#[derive(Debug, Clone)]
pub struct ValueBoolValidator<V> {
    inner: V,
}

impl<V> ValueBoolValidator<V> {
    pub fn new(inner: V) -> Self {
        Self { inner }
    }
}

impl<V> TypedValidator for ValueBoolValidator<V>
where
    V: TypedValidator<Input = bool, Output = ()>,
    V::Error: Into<ValidationError>,
{
    type Input = Value;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        match input {
            Value::Boolean(b) => {
                self.inner.validate(b).map_err(|e| e.into())
            }
            _ => Err(ValidationError::type_mismatch("", "boolean", input.kind().name())),
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        self.inner.metadata()
    }
}

// ============================================================================
// CONVENIENCE FUNCTIONS
// ============================================================================

/// Wraps a string validator for use with Value.
///
/// # Examples
///
/// ```rust
/// use nebula_validator::bridge::for_string;
/// use nebula_validator::validators::string::min_length;
///
/// let validator = for_string(min_length(5));
/// ```
pub fn for_string<V>(validator: V) -> ValueValidator<V>
where
    V: TypedValidator<Input = str, Output = ()>,
{
    ValueValidator::new(validator)
}

/// Wraps an i64 validator for use with Value.
pub fn for_i64<V>(validator: V) -> ValueI64Validator<V>
where
    V: TypedValidator<Input = i64, Output = ()>,
{
    ValueI64Validator::new(validator)
}

/// Wraps an f64 validator for use with Value.
pub fn for_f64<V>(validator: V) -> ValueF64Validator<V>
where
    V: TypedValidator<Input = f64, Output = ()>,
{
    ValueF64Validator::new(validator)
}

/// Wraps an array validator for use with Value.
pub fn for_array<V>(validator: V) -> ValueArrayValidator<V>
where
    V: TypedValidator<Input = [Value], Output = ()>,
{
    ValueArrayValidator::new(validator)
}

/// Wraps a boolean validator for use with Value.
pub fn for_bool<V>(validator: V) -> ValueBoolValidator<V>
where
    V: TypedValidator<Input = bool, Output = ()>,
{
    ValueBoolValidator::new(validator)
}

// ============================================================================
// EXTENSION TRAIT
// ============================================================================

/// Extension trait for converting validators to Value validators.
pub trait ValueValidatorExt: Sized {
    /// Converts a string validator to work with Value.
    fn for_value(self) -> ValueValidator<Self>;
}

impl<V> ValueValidatorExt for V
where
    V: TypedValidator<Input = str, Output = ()>,
{
    fn for_value(self) -> ValueValidator<Self> {
        ValueValidator::new(self)
    }
}

// ============================================================================
// LEGACY V1 API COMPATIBILITY
// ============================================================================

/// Legacy validator trait from v1 for backwards compatibility.
#[async_trait::async_trait]
pub trait LegacyValidator: Send + Sync {
    async fn validate(
        &self,
        value: &Value,
        context: Option<&ValidationContext>,
    ) -> Result<Valid<()>, Invalid<()>>;
    
    fn name(&self) -> &str;
}

/// Legacy Valid type from v1.
#[derive(Debug, Clone)]
pub struct Valid<T> {
    _data: std::marker::PhantomData<T>,
}

impl<T> Valid<T> {
    pub fn new(_data: T) -> Self {
        Self {
            _data: std::marker::PhantomData,
        }
    }
}

/// Legacy Invalid type from v1.
#[derive(Debug, Clone)]
pub struct Invalid<T> {
    pub errors: Vec<String>,
    _data: std::marker::PhantomData<T>,
}

impl<T> Invalid<T> {
    pub fn new(errors: Vec<String>) -> Self {
        Self {
            errors,
            _data: std::marker::PhantomData,
        }
    }
    
    pub fn simple(message: impl Into<String>) -> Self {
        Self::new(vec![message.into()])
    }
}

/// Legacy validation context from v1.
#[derive(Debug, Clone, Default)]
pub struct ValidationContext {
    // Empty for now - can be populated if needed
}

/// Adapter to convert v2 validators to v1 API.
pub struct V1Adapter<V> {
    validator: ValueValidator<V>,
    name: String,
}

impl<V> V1Adapter<V> {
    pub fn new(validator: V) -> Self
    where
        V: TypedValidator<Input = str, Output = ()>,
        V::Error: Into<ValidationError>,
    {
        let name = validator.metadata().name.clone();
        Self {
            validator: ValueValidator::new(validator),
            name,
        }
    }
}

#[async_trait::async_trait]
impl<V> LegacyValidator for V1Adapter<V>
where
    V: TypedValidator<Input = str, Output = ()> + Send + Sync,
    V::Error: Into<ValidationError>,
{
    async fn validate(
        &self,
        value: &Value,
        _context: Option<&ValidationContext>,
    ) -> Result<Valid<()>, Invalid<()>> {
        match self.validator.validate(value) {
            Ok(_) => Ok(Valid::new(())),
            Err(e) => Err(Invalid::simple(e.message)),
        }
    }
    
    fn name(&self) -> &str {
        &self.name
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validators::string::{min_length, MinLength};

    #[test]
    fn test_value_validator_string() {
        let validator = ValueValidator::new(MinLength { min: 5 });
        
        assert!(validator.validate(&Value::text("hello")).is_ok());
        assert!(validator.validate(&Value::text("hi")).is_err());
        assert!(validator.validate(&Value::number(42.0)).is_err());
    }

    #[test]
    fn test_for_string_helper() {
        let validator = for_string(min_length(5));
        
        assert!(validator.validate(&Value::text("hello")).is_ok());
        assert!(validator.validate(&Value::text("hi")).is_err());
    }

    #[test]
    fn test_value_i64_validator() {
        use crate::validators::numeric::min;
        
        let validator = for_i64(min(10));
        
        assert!(validator.validate(&Value::integer(15)).is_ok());
        assert!(validator.validate(&Value::integer(5)).is_err());
        assert!(validator.validate(&Value::text("hello")).is_err());
    }

    #[test]
    fn test_value_bool_validator() {
        use crate::validators::logical::is_true;
        
        let validator = for_bool(is_true());
        
        assert!(validator.validate(&Value::boolean(true)).is_ok());
        assert!(validator.validate(&Value::boolean(false)).is_err());
    }

    #[test]
    fn test_value_array_validator() {
        use nebula_value::Array;

        // Create a simple test validator for arrays
        struct MinArraySize { min: usize }
        impl TypedValidator for MinArraySize {
            type Input = nebula_value::Array;
            type Output = ();
            type Error = ValidationError;

            fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
                if input.len() >= self.min {
                    Ok(())
                } else {
                    Err(ValidationError::new("min_size", format!("Array too small: {} < {}", input.len(), self.min)))
                }
            }

            fn metadata(&self) -> ValidatorMetadata {
                ValidatorMetadata::simple("MinArraySize")
            }
        }

        let validator = for_array(MinArraySize { min: 2 });

        let mut arr = Array::new();
        arr.push(Value::Integer(nebula_value::Integer::new(1)));
        arr.push(Value::Integer(nebula_value::Integer::new(2)));
        assert!(validator.validate(&Value::Array(arr)).is_ok());

        let mut arr = Array::new();
        arr.push(Value::Integer(nebula_value::Integer::new(1)));
        assert!(validator.validate(&Value::Array(arr)).is_err());
    }

    #[test]
    fn test_extension_trait() {
        let validator = min_length(5).for_value();
        
        assert!(validator.validate(&Value::text("hello")).is_ok());
        assert!(validator.validate(&Value::text("hi")).is_err());
    }

    #[tokio::test]
    async fn test_v1_adapter() {
        let validator = V1Adapter::new(min_length(5));
        
        let result = validator.validate(&Value::text("hello"), None).await;
        assert!(result.is_ok());
        
        let result = validator.validate(&Value::text("hi"), None).await;
        assert!(result.is_err());
    }
}

pub trait ValueType: Sized + Clone + fmt::Debug + PartialEq {
    /// The error type returned when validation or conversion fails.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Returns the name of this value type.
    ///
    /// This is used for error messages, debugging, and schema generation.
    fn type_name() -> &'static str;

    /// Validates the current value.
    ///
    /// This method should check that the value meets all constraints
    /// and business rules for this type.
    fn validate(&self) -> ValidationResult;

    /// Converts this value to the universal `Value` enum.
    ///
    /// This enables interoperability with all other nebula-value types.
    fn to_value(&self) -> Value;

    /// Attempts to create an instance of this type from a `Value`.
    ///
    /// This should validate the input and return an error if the
    /// conversion is not possible or the value is invalid.
    fn from_value(value: Value) -> Result<Self, Self::Error>;

    /// Returns the JSON schema for this type (when schema feature is enabled).
    ///
    /// The default implementation returns a basic string schema.
    #[cfg(feature = "schema")]
    #[cfg_attr(docsrs, doc(cfg(feature = "schema")))]
    fn json_schema() -> schemars::schema::Schema {
        use schemars::schema::*;
        Schema::Object(SchemaObject {
            metadata: Some(Box::new(Metadata {
                title: Some(Self::type_name().to_string()),
                ..Default::default()
            })),
            instance_type: Some(InstanceType::String.into()),
            ..Default::default()
        })
    }

    /// Returns whether this type supports null values.
    ///
    /// The default implementation returns `false`.
    fn is_nullable() -> bool {
        false
    }

    /// Returns the default value for this type, if any.
    ///
    /// The default implementation returns `None`.
    fn default_value() -> Option<Self> {
        None
    }

    /// Returns a human-readable description of this type.
    ///
    /// This is used for documentation and error messages.
    fn description() -> Option<&'static str> {
        None
    }

    /// Returns examples of valid values for this type.
    ///
    /// This is used for documentation and testing.
    fn examples() -> Vec<Self> {
        Vec::new()
    }
}

/// A trait for types that can be validated.
///
/// This is automatically implemented for all `ValueType` implementations,
/// but can also be implemented independently for types that need validation
/// but don't need full `ValueType` integration.
pub trait Validatable {
    /// Validates the value and returns a result.
    fn validate(&self) -> ValidationResult;

    /// Returns whether the value is valid.
    fn is_valid(&self) -> bool {
        self.validate().is_ok()
    }
}

// Automatic implementation of Validatable for all ValueType implementations
impl<T: ValueType> Validatable for T {
    fn validate(&self) -> ValidationResult {
        ValueType::validate(self)
    }
}

/// A trait for types that can be converted to and from JSON values.
///
/// This is used internally for serialization and provides a bridge
/// between custom types and standard JSON representations.
pub trait JsonConvertible: Sized {
    /// The error type returned when JSON conversion fails.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Converts this value to a JSON value.
    fn to_json(&self) -> serde_json::Value;

    /// Attempts to create an instance from a JSON value.
    fn from_json(json: serde_json::Value) -> Result<Self, Self::Error>;
}

// Automatic implementation of JsonConvertible for ValueType implementations
impl<T: ValueType> JsonConvertible for T {
    type Error = T::Error;

    fn to_json(&self) -> serde_json::Value {
        // Convert through Value enum for consistency
        let value = self.to_value();
        serde_json::to_value(value).unwrap_or(serde_json::Value::Null)
    }

    fn from_json(json: serde_json::Value) -> Result<Self, Self::Error> {
        // Convert through Value enum for consistency
        let value: Value = serde_json::from_value(json)
            .map_err(|_| {
                // This is a bit of a hack since we can't convert serde_json::Error
                // to T::Error generically. In practice, this should be handled
                // by the specific ValueType implementation.
                panic!("JSON conversion error - should be handled by ValueType implementation")
            })?;
        Self::from_value(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Text;

    // Test implementation of ValueType
    #[derive(Debug, Clone, PartialEq)]
    struct TestType {
        value: String,
    }

    #[derive(Debug, Clone, PartialEq)]
    enum TestTypeError {
        Empty,
        TooLong,
    }

    impl fmt::Display for TestTypeError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                TestTypeError::Empty => write!(f, "Value cannot be empty"),
                TestTypeError::TooLong => write!(f, "Value is too long"),
            }
        }
    }

    impl std::error::Error for TestTypeError {}

    impl ValueType for TestType {
        type Error = TestTypeError;

        fn type_name() -> &'static str {
            "TestType"
        }

        fn validate(&self) -> ValidationResult {
            if self.value.is_empty() {
                return Err(ValidationError::new("empty", "Value cannot be empty"));
            }
            if self.value.len() > 100 {
                return Err(ValidationError::new("too_long", "Value is too long"));
            }
            Ok(())
        }

        fn to_value(&self) -> Value {
            Value::Text(Text::new(&self.value))
        }

        fn from_value(value: Value) -> Result<Self, Self::Error> {
            match value {
                Value::Text(text) => {
                    let value = text.as_str().to_string();
                    if value.is_empty() {
                        return Err(TestTypeError::Empty);
                    }
                    if value.len() > 100 {
                        return Err(TestTypeError::TooLong);
                    }
                    Ok(TestType { value })
                }
                _ => Err(TestTypeError::Empty), // Simplified for test
            }
        }
    }

    #[test]
    fn test_value_type_validation() {
        let valid = TestType { value: "hello".to_string() };
        assert!(valid.validate().is_ok());
        assert!(valid.is_valid());

        let empty = TestType { value: String::new() };
        assert!(empty.validate().is_err());
        assert!(!empty.is_valid());
    }

    #[test]
    fn test_value_type_conversion() {
        let test_type = TestType { value: "hello".to_string() };
        let value = test_type.to_value();

        match value {
            Value::Text(text) => assert_eq!(text.as_str(), "hello"),
            _ => panic!("Expected Text value"),
        }

        let converted_back = TestType::from_value(value).unwrap();
        assert_eq!(converted_back, test_type);
    }

    #[test]
    fn test_type_name() {
        assert_eq!(TestType::type_name(), "TestType");
    }
}
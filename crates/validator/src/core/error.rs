//! Error types for validation failures
//!
//! This module provides a rich, structured error type that supports
//! nested errors, field paths, error codes, and parameterized messages.
//!
//! All string fields use `Cow<'static, str>` for zero-allocation in the
//! common case of static error codes and messages.

use std::borrow::Cow;
use std::fmt;

// ============================================================================
// VALIDATION ERROR
// ============================================================================

/// A structured validation error with support for nested errors and metadata.
///
/// Uses `Cow<'static, str>` for zero-allocation when error codes and messages
/// are known at compile time (the common case).
///
/// # Examples
///
/// ## Simple error
///
/// ```rust,ignore
/// use nebula_validator::core::ValidationError;
///
/// let error = ValidationError::new("min_length", "String is too short");
/// ```
///
/// ## Error with parameters
///
/// ```rust,ignore
/// use nebula_validator::core::ValidationError;
///
/// let error = ValidationError::new("min_length", "String is too short")
///     .with_param("min", "5")
///     .with_param("actual", "3");
/// ```
///
/// ## Nested errors
///
/// ```rust,ignore
/// use nebula_validator::core::ValidationError;
///
/// let error = ValidationError::new("object_validation", "Object validation failed")
///     .with_field("user.email")
///     .with_nested(vec![
///         ValidationError::new("email_invalid", "Invalid email format"),
///     ]);
/// ```
#[derive(Debug, Clone)]
pub struct ValidationError {
    /// Error code for programmatic handling and i18n.
    ///
    /// Examples: "min_length", "email_invalid", "required"
    pub code: Cow<'static, str>,

    /// Human-readable error message in English.
    ///
    /// This is the default message. Use `code` and `params` for i18n.
    pub message: Cow<'static, str>,

    /// Optional field path for nested object validation.
    ///
    /// Examples: "user.email", "address.zipcode", "items[0].name"
    pub field: Option<Cow<'static, str>>,

    /// Parameters for the error message template.
    ///
    /// Stored as ordered key-value pairs (typically 0-3 params).
    /// Example: `[("min", "5"), ("actual", "3")]`
    pub params: Vec<(Cow<'static, str>, Cow<'static, str>)>,

    /// Nested validation errors for complex objects.
    ///
    /// Used when validating objects with multiple fields that can each fail.
    pub nested: Vec<ValidationError>,

    /// Optional severity level.
    pub severity: ErrorSeverity,

    /// Optional help text or suggestion for fixing the error.
    pub help: Option<Cow<'static, str>>,
}

/// Severity level of a validation error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ErrorSeverity {
    /// Error that must be fixed (default).
    #[default]
    Error,
    /// Warning that should be addressed but doesn't block validation.
    Warning,
    /// Informational message.
    Info,
}

impl ValidationError {
    /// Creates a new validation error with a code and message.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use nebula_validator::core::ValidationError;
    ///
    /// // Static strings — zero allocation:
    /// let error = ValidationError::new("min_length", "String is too short");
    ///
    /// // Dynamic strings — allocates only when needed:
    /// let error = ValidationError::new("min_length", format!("Must be at least {} chars", 5));
    /// ```
    pub fn new(code: impl Into<Cow<'static, str>>, message: impl Into<Cow<'static, str>>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            field: None,
            params: Vec::new(),
            nested: Vec::new(),
            severity: ErrorSeverity::Error,
            help: None,
        }
    }

    /// Sets the field path for this error.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_field(mut self, field: impl Into<Cow<'static, str>>) -> Self {
        self.field = Some(field.into());
        self
    }

    /// Adds a parameter to the error.
    ///
    /// Parameters are used for message templating and i18n.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_param(
        mut self,
        key: impl Into<Cow<'static, str>>,
        value: impl Into<Cow<'static, str>>,
    ) -> Self {
        self.params.push((key.into(), value.into()));
        self
    }

    /// Adds nested validation errors.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_nested(mut self, errors: Vec<ValidationError>) -> Self {
        self.nested = errors;
        self
    }

    /// Adds a single nested error.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_nested_error(mut self, error: ValidationError) -> Self {
        self.nested.push(error);
        self
    }

    /// Sets the severity level.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_severity(mut self, severity: ErrorSeverity) -> Self {
        self.severity = severity;
        self
    }

    /// Adds help text or a suggestion.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_help(mut self, help: impl Into<Cow<'static, str>>) -> Self {
        self.help = Some(help.into());
        self
    }

    /// Looks up a parameter value by key.
    #[must_use]
    pub fn param(&self, key: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(k, _)| k.as_ref() == key)
            .map(|(_, v)| v.as_ref())
    }

    /// Returns true if this error has nested errors.
    #[must_use]
    pub fn has_nested(&self) -> bool {
        !self.nested.is_empty()
    }

    /// Returns the number of errors (including nested).
    #[must_use]
    pub fn total_error_count(&self) -> usize {
        1 + self
            .nested
            .iter()
            .map(ValidationError::total_error_count)
            .sum::<usize>()
    }

    /// Flattens all errors into a single list (depth-first).
    #[must_use]
    pub fn flatten(&self) -> Vec<&ValidationError> {
        let mut result = vec![self];
        for nested in &self.nested {
            result.extend(nested.flatten());
        }
        result
    }

    /// Converts the error to a JSON-like structure (for serialization).
    #[cfg(feature = "serde")]
    pub fn to_json_value(&self) -> serde_json::Value {
        use serde_json::json;

        let params: serde_json::Map<String, serde_json::Value> = self
            .params
            .iter()
            .map(|(k, v)| (k.to_string(), serde_json::Value::String(v.to_string())))
            .collect();

        json!({
            "code": self.code,
            "message": self.message,
            "field": self.field,
            "params": params,
            "severity": format!("{:?}", self.severity),
            "help": self.help,
            "nested": self.nested.iter().map(|e| e.to_json_value()).collect::<Vec<_>>(),
        })
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(field) = &self.field {
            write!(f, "[{}] {}: {}", field, self.code, self.message)?;
        } else {
            write!(f, "{}: {}", self.code, self.message)?;
        }

        if !self.params.is_empty() {
            write!(f, " (params: [")?;
            for (i, (k, v)) in self.params.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write!(f, "{k}={v}")?;
            }
            write!(f, "])")?;
        }

        if let Some(help) = &self.help {
            write!(f, "\n  Help: {help}")?;
        }

        if !self.nested.is_empty() {
            write!(f, "\n  Nested errors:")?;
            for (i, error) in self.nested.iter().enumerate() {
                write!(f, "\n    {}. {}", i + 1, error)?;
            }
        }

        Ok(())
    }
}

impl std::error::Error for ValidationError {}

// ============================================================================
// CONVENIENCE CONSTRUCTORS
// ============================================================================

impl ValidationError {
    /// Creates a "required" error.
    pub fn required(field: impl Into<Cow<'static, str>>) -> Self {
        Self::new("required", "This field is required").with_field(field)
    }

    /// Creates a "min_length" error.
    pub fn min_length(field: impl Into<Cow<'static, str>>, min: usize, actual: usize) -> Self {
        Self::new("min_length", format!("Must be at least {min} characters"))
            .with_field(field)
            .with_param("min", min.to_string())
            .with_param("actual", actual.to_string())
    }

    /// Creates a "max_length" error.
    pub fn max_length(field: impl Into<Cow<'static, str>>, max: usize, actual: usize) -> Self {
        Self::new("max_length", format!("Must be at most {max} characters"))
            .with_field(field)
            .with_param("max", max.to_string())
            .with_param("actual", actual.to_string())
    }

    /// Creates an "invalid_format" error.
    pub fn invalid_format(
        field: impl Into<Cow<'static, str>>,
        expected: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self::new("invalid_format", "Invalid format")
            .with_field(field)
            .with_param("expected", expected)
    }

    /// Creates a "type_mismatch" error.
    pub fn type_mismatch(
        field: impl Into<Cow<'static, str>>,
        expected: impl Into<Cow<'static, str>>,
        actual: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self::new("type_mismatch", "Type mismatch")
            .with_field(field)
            .with_param("expected", expected)
            .with_param("actual", actual)
    }

    /// Creates a "range" error.
    pub fn out_of_range<T: fmt::Display>(
        field: impl Into<Cow<'static, str>>,
        min: T,
        max: T,
        actual: T,
    ) -> Self {
        Self::new(
            "out_of_range",
            format!("Value must be between {min} and {max}"),
        )
        .with_field(field)
        .with_param("min", min.to_string())
        .with_param("max", max.to_string())
        .with_param("actual", actual.to_string())
    }

    /// Creates a "custom" error with a message.
    pub fn custom(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new("custom", message)
    }
}

// ============================================================================
// ERROR COLLECTION
// ============================================================================

/// A collection of validation errors.
///
/// Useful for collecting multiple validation errors before returning them.
#[derive(Debug, Clone, Default)]
pub struct ValidationErrors {
    errors: Vec<ValidationError>,
}

impl ValidationErrors {
    /// Creates a new empty error collection.
    #[must_use]
    pub fn new() -> Self {
        Self { errors: Vec::new() }
    }

    /// Adds an error to the collection.
    pub fn add(&mut self, error: ValidationError) {
        self.errors.push(error);
    }

    /// Adds multiple errors to the collection.
    pub fn extend(&mut self, errors: Vec<ValidationError>) {
        self.errors.extend(errors);
    }

    /// Returns true if there are any errors.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    /// Returns the number of errors.
    #[must_use]
    pub fn len(&self) -> usize {
        self.errors.len()
    }

    /// Returns true if empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.errors.is_empty()
    }

    /// Returns all errors.
    #[must_use]
    pub fn errors(&self) -> &[ValidationError] {
        &self.errors
    }

    /// Converts to a single error with nested errors.
    pub fn into_single_error(self, message: impl Into<Cow<'static, str>>) -> ValidationError {
        ValidationError::new("validation_errors", message).with_nested(self.errors)
    }

    /// Converts to a Result.
    #[must_use = "result must be used"]
    pub fn into_result<T>(self, ok_value: T) -> Result<T, ValidationErrors> {
        if self.is_empty() {
            Ok(ok_value)
        } else {
            Err(self)
        }
    }
}

impl FromIterator<ValidationError> for ValidationErrors {
    fn from_iter<I: IntoIterator<Item = ValidationError>>(iter: I) -> Self {
        Self {
            errors: iter.into_iter().collect(),
        }
    }
}

impl fmt::Display for ValidationErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Validation failed with {} error(s):", self.errors.len())?;
        for (i, error) in self.errors.iter().enumerate() {
            writeln!(f, "  {}. {}", i + 1, error)?;
        }
        Ok(())
    }
}

impl std::error::Error for ValidationErrors {}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_error() {
        let error = ValidationError::new("test", "Test error");
        assert_eq!(error.code, "test");
        assert_eq!(error.message, "Test error");
    }

    #[test]
    fn test_error_with_field() {
        let error = ValidationError::new("required", "Field is required").with_field("email");
        assert_eq!(error.field.as_deref(), Some("email"));
    }

    #[test]
    fn test_error_with_params() {
        let error = ValidationError::new("min", "Too small")
            .with_param("min", "5")
            .with_param("actual", "3");

        assert_eq!(error.param("min"), Some("5"));
        assert_eq!(error.param("actual"), Some("3"));
    }

    #[test]
    fn test_nested_errors() {
        let error = ValidationError::new("object", "Object validation failed").with_nested(vec![
            ValidationError::new("email", "Invalid email").with_field("email"),
            ValidationError::new("age", "Too young").with_field("age"),
        ]);

        assert_eq!(error.nested.len(), 2);
        assert_eq!(error.total_error_count(), 3); // 1 parent + 2 nested
    }

    #[test]
    fn test_error_collection() {
        let mut errors = ValidationErrors::new();
        errors.add(ValidationError::new("error1", "First error"));
        errors.add(ValidationError::new("error2", "Second error"));

        assert_eq!(errors.len(), 2);
        assert!(errors.has_errors());
    }

    #[test]
    fn test_flatten() {
        let error = ValidationError::new("root", "Root error").with_nested(vec![
            ValidationError::new("child1", "Child 1")
                .with_nested(vec![ValidationError::new("grandchild", "Grandchild")]),
            ValidationError::new("child2", "Child 2"),
        ]);

        let flattened = error.flatten();
        assert_eq!(flattened.len(), 4); // root + 2 children + 1 grandchild
    }

    #[test]
    fn test_zero_alloc_static_strings() {
        let error = ValidationError::new("required", "This field is required");
        // Both should be borrowed (no allocation)
        assert!(matches!(error.code, Cow::Borrowed(_)));
        assert!(matches!(error.message, Cow::Borrowed(_)));
    }

    #[test]
    fn test_dynamic_strings() {
        let code = format!("error_{}", 42);
        let error = ValidationError::new(code, "Dynamic error");
        assert!(matches!(error.code, Cow::Owned(_)));
        assert!(matches!(error.message, Cow::Borrowed(_)));
    }
}

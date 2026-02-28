//! Error types for validation failures
//!
//! This module provides a rich, structured error type that supports
//! nested errors, field paths, error codes, and parameterized messages.
//!
//! # Memory Optimization
//!
//! `ValidationError` is optimized for the common case (80 bytes):
//! - `code`, `message`, `field` are inline (most errors only use these)
//! - `params`, `nested`, `severity`, `help` are boxed in `ErrorExtras` (lazy allocated)
//!
//! This reduces stack size by ~47% compared to inlining all fields.

use smallvec::SmallVec;
use std::borrow::Cow;
use std::fmt;

// Canonical error codes used by built-in validators and combinators.
pub mod codes {
    pub const REQUIRED: &str = "required";
    pub const MIN_LENGTH: &str = "min_length";
    pub const MAX_LENGTH: &str = "max_length";
    pub const INVALID_FORMAT: &str = "invalid_format";
    pub const TYPE_MISMATCH: &str = "type_mismatch";
    pub const OUT_OF_RANGE: &str = "out_of_range";
    pub const EXACT_LENGTH: &str = "exact_length";
    pub const LENGTH_RANGE: &str = "length_range";
    pub const CUSTOM: &str = "custom";
}

// ============================================================================
// ERROR EXTRAS (Boxed for rare fields)
// ============================================================================

/// Extended error data, boxed to reduce `ValidationError` size.
///
/// Most validation errors only need `code`, `message`, and `field`.
/// This struct holds rarely-used fields that are lazily allocated.
#[derive(Debug, Clone, PartialEq)]
struct ErrorExtras {
    /// Parameters for the error message template.
    /// SmallVec optimizes for 0-2 params inline (covers ~95% of cases).
    params: SmallVec<[(Cow<'static, str>, Cow<'static, str>); 2]>,

    /// Nested validation errors for complex objects.
    nested: Vec<ValidationError>,

    /// Severity level (defaults to Error).
    severity: ErrorSeverity,

    /// Help text or suggestion for fixing the error.
    help: Option<Cow<'static, str>>,
}

impl Default for ErrorExtras {
    fn default() -> Self {
        Self {
            params: SmallVec::new(),
            nested: Vec::new(),
            severity: ErrorSeverity::Error,
            help: None,
        }
    }
}

// ============================================================================
// VALIDATION ERROR
// ============================================================================

/// A structured validation error with support for nested errors and metadata.
///
/// Uses `Cow<'static, str>` for zero-allocation when error codes and messages
/// are known at compile time (the common case).
///
/// # Memory Layout (80 bytes)
///
/// - `code`: 24 bytes (Cow<'static, str>)
/// - `message`: 24 bytes (Cow<'static, str>)
/// - `field`: 24 bytes (Option<Cow<'static, str>>)
/// - `extras`: 8 bytes (`Option<Box<ErrorExtras>>`)
///
/// # Examples
///
/// ## Simple error
///
/// ```rust
/// use nebula_validator::foundation::ValidationError;
///
/// let error = ValidationError::new("min_length", "String is too short");
/// ```
///
/// ## Error with parameters
///
/// ```rust
/// use nebula_validator::foundation::ValidationError;
///
/// let error = ValidationError::new("min_length", "String is too short")
///     .with_param("min", "5")
///     .with_param("actual", "3");
/// ```
///
/// ## Nested errors
///
/// ```rust
/// use nebula_validator::foundation::ValidationError;
///
/// let error = ValidationError::new("object_validation", "Object validation failed")
///     .with_field("user.email")
///     .with_nested(vec![
///         ValidationError::new("email_invalid", "Invalid email format"),
///     ]);
/// ```
#[derive(Debug, Clone, PartialEq)]
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
    /// Examples: "user.email", "address.zipcode", "items\[0\].name"
    pub field: Option<Cow<'static, str>>,

    /// Extended error data (params, nested, severity, help).
    /// Boxed to reduce struct size; lazily allocated on first use.
    extras: Option<Box<ErrorExtras>>,
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
    /// ```rust
    /// use nebula_validator::foundation::ValidationError;
    ///
    /// // Static strings - zero allocation:
    /// let error = ValidationError::new("min_length", "String is too short");
    ///
    /// // Dynamic strings - allocates only when needed:
    /// let error = ValidationError::new("min_length", format!("Must be at least {} chars", 5));
    /// ```
    #[inline]
    pub fn new(code: impl Into<Cow<'static, str>>, message: impl Into<Cow<'static, str>>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            field: None,
            extras: None,
        }
    }

    /// Sets the field path for this error.
    ///
    /// Empty strings are treated as "no field" and leave `field` as `None`.
    #[must_use = "builder methods must be chained or built"]
    #[inline]
    pub fn with_field(mut self, field: impl Into<Cow<'static, str>>) -> Self {
        let field = field.into();
        if !field.is_empty() {
            self.field = Some(field);
        }
        self
    }

    /// Adds a parameter to the error.
    ///
    /// Parameters are used for message templating and i18n.
    #[must_use = "builder methods must be chained or built"]
    #[inline]
    pub fn with_param(
        mut self,
        key: impl Into<Cow<'static, str>>,
        value: impl Into<Cow<'static, str>>,
    ) -> Self {
        let key = key.into();
        let value = redact_if_sensitive(&key, value.into());
        self.extras_mut().params.push((key, value));
        self
    }

    /// Adds nested validation errors.
    #[must_use = "builder methods must be chained or built"]
    #[inline]
    pub fn with_nested(mut self, errors: Vec<ValidationError>) -> Self {
        if !errors.is_empty() {
            self.extras_mut().nested = errors;
        }
        self
    }

    /// Adds a single nested error.
    #[must_use = "builder methods must be chained or built"]
    #[inline]
    pub fn with_nested_error(mut self, error: ValidationError) -> Self {
        self.extras_mut().nested.push(error);
        self
    }

    /// Sets the severity level.
    #[must_use = "builder methods must be chained or built"]
    #[inline]
    pub fn with_severity(mut self, severity: ErrorSeverity) -> Self {
        self.extras_mut().severity = severity;
        self
    }

    /// Adds help text or a suggestion.
    #[must_use = "builder methods must be chained or built"]
    #[inline]
    pub fn with_help(mut self, help: impl Into<Cow<'static, str>>) -> Self {
        self.extras_mut().help = Some(help.into());
        self
    }

    // ========================================================================
    // ACCESSORS
    // ========================================================================

    /// Looks up a parameter value by key.
    #[must_use]
    #[inline]
    pub fn param(&self, key: &str) -> Option<&str> {
        self.params()
            .iter()
            .find(|(k, _)| k.as_ref() == key)
            .map(|(_, v)| v.as_ref())
    }

    /// Returns all parameters.
    #[must_use]
    #[inline]
    pub fn params(&self) -> &[(Cow<'static, str>, Cow<'static, str>)] {
        self.extras
            .as_ref()
            .map(|e| e.params.as_slice())
            .unwrap_or(&[])
    }

    /// Returns nested errors.
    #[must_use]
    #[inline]
    pub fn nested(&self) -> &[ValidationError] {
        self.extras
            .as_ref()
            .map(|e| e.nested.as_slice())
            .unwrap_or(&[])
    }

    /// Returns true if this error has nested errors.
    #[must_use]
    #[inline]
    pub fn has_nested(&self) -> bool {
        self.extras.as_ref().is_some_and(|e| !e.nested.is_empty())
    }

    /// Returns the severity level.
    #[must_use]
    #[inline]
    pub fn severity(&self) -> ErrorSeverity {
        self.extras
            .as_ref()
            .map(|e| e.severity)
            .unwrap_or(ErrorSeverity::Error)
    }

    /// Returns help text if available.
    #[must_use]
    #[inline]
    pub fn help(&self) -> Option<&str> {
        self.extras.as_ref()?.help.as_deref()
    }

    /// Returns the number of errors (including nested).
    #[must_use]
    pub fn total_error_count(&self) -> usize {
        1 + self
            .nested()
            .iter()
            .map(ValidationError::total_error_count)
            .sum::<usize>()
    }

    /// Flattens all errors into a single list (depth-first).
    #[must_use]
    pub fn flatten(&self) -> Vec<&ValidationError> {
        let mut result = vec![self];
        for nested in self.nested() {
            result.extend(nested.flatten());
        }
        result
    }

    /// Converts the error to a JSON-like structure (for serialization).
    pub fn to_json_value(&self) -> serde_json::Value {
        use serde_json::json;

        let params: serde_json::Map<String, serde_json::Value> = self
            .params()
            .iter()
            .map(|(k, v)| (k.to_string(), serde_json::Value::String(v.to_string())))
            .collect();

        json!({
            "code": self.code,
            "message": self.message,
            "field": self.field,
            "params": params,
            "severity": format!("{:?}", self.severity()),
            "help": self.help(),
            "nested": self.nested().iter().map(|e| e.to_json_value()).collect::<Vec<_>>(),
        })
    }

    // ========================================================================
    // INTERNAL HELPERS
    // ========================================================================

    /// Gets mutable reference to extras, creating if needed.
    #[inline]
    fn extras_mut(&mut self) -> &mut ErrorExtras {
        self.extras
            .get_or_insert_with(|| Box::new(ErrorExtras::default()))
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(field) = &self.field {
            write!(f, "[{}] {}: {}", field, self.code, self.message)?;
        } else {
            write!(f, "{}: {}", self.code, self.message)?;
        }

        let params = self.params();
        if !params.is_empty() {
            write!(f, " (params: [")?;
            for (i, (k, v)) in params.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write!(f, "{k}={v}")?;
            }
            write!(f, "])")?;
        }

        if let Some(help) = self.help() {
            write!(f, "\n  Help: {help}")?;
        }

        let nested = self.nested();
        if !nested.is_empty() {
            write!(f, "\n  Nested errors:")?;
            for (i, error) in nested.iter().enumerate() {
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
    #[inline]
    pub fn required(field: impl Into<Cow<'static, str>>) -> Self {
        Self::new(codes::REQUIRED, "This field is required").with_field(field)
    }

    /// Creates a "min_length" error.
    #[inline]
    pub fn min_length(field: impl Into<Cow<'static, str>>, min: usize, actual: usize) -> Self {
        Self::new(
            codes::MIN_LENGTH,
            format!("Must be at least {min} characters"),
        )
        .with_field(field)
        .with_param("min", min.to_string())
        .with_param("actual", actual.to_string())
    }

    /// Creates a "max_length" error.
    #[inline]
    pub fn max_length(field: impl Into<Cow<'static, str>>, max: usize, actual: usize) -> Self {
        Self::new(
            codes::MAX_LENGTH,
            format!("Must be at most {max} characters"),
        )
        .with_field(field)
        .with_param("max", max.to_string())
        .with_param("actual", actual.to_string())
    }

    /// Creates an "invalid_format" error.
    #[inline]
    pub fn invalid_format(
        field: impl Into<Cow<'static, str>>,
        expected: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self::new(codes::INVALID_FORMAT, "Invalid format")
            .with_field(field)
            .with_param("expected", expected)
    }

    /// Creates a "type_mismatch" error.
    #[inline]
    pub fn type_mismatch(
        field: impl Into<Cow<'static, str>>,
        expected: impl Into<Cow<'static, str>>,
        actual: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self::new(codes::TYPE_MISMATCH, "Type mismatch")
            .with_field(field)
            .with_param("expected", expected)
            .with_param("actual", actual)
    }

    /// Creates a "range" error.
    #[inline]
    pub fn out_of_range<T: fmt::Display>(
        field: impl Into<Cow<'static, str>>,
        min: T,
        max: T,
        actual: T,
    ) -> Self {
        Self::new(
            codes::OUT_OF_RANGE,
            format!("Value must be between {min} and {max}"),
        )
        .with_field(field)
        .with_param("min", min.to_string())
        .with_param("max", max.to_string())
        .with_param("actual", actual.to_string())
    }

    /// Creates an "exact_length" error.
    #[inline]
    pub fn exact_length(
        field: impl Into<Cow<'static, str>>,
        expected: usize,
        actual: usize,
    ) -> Self {
        Self::new(
            codes::EXACT_LENGTH,
            format!("Must be exactly {expected} characters"),
        )
        .with_field(field)
        .with_param("expected", expected.to_string())
        .with_param("actual", actual.to_string())
    }

    /// Creates a "length_range" error.
    #[inline]
    pub fn length_range(
        field: impl Into<Cow<'static, str>>,
        min: usize,
        max: usize,
        actual: usize,
    ) -> Self {
        Self::new(
            codes::LENGTH_RANGE,
            format!("Must be between {min} and {max} characters"),
        )
        .with_field(field)
        .with_param("min", min.to_string())
        .with_param("max", max.to_string())
        .with_param("actual", actual.to_string())
    }

    /// Creates a "custom" error with a message.
    #[inline]
    pub fn custom(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(codes::CUSTOM, message)
    }
}

#[inline]
fn redact_if_sensitive(key: &str, value: Cow<'static, str>) -> Cow<'static, str> {
    let lowered = key.to_ascii_lowercase();
    let sensitive = [
        "password",
        "secret",
        "token",
        "api_key",
        "apikey",
        "credential",
    ];
    if sensitive.iter().any(|pattern| lowered.contains(pattern)) {
        Cow::Borrowed("[REDACTED]")
    } else {
        value
    }
}

// ============================================================================
// ERROR COLLECTION
// ============================================================================

/// A collection of validation errors.
///
/// Useful for collecting multiple validation errors before returning them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ValidationErrors {
    errors: Vec<ValidationError>,
}

impl ValidationErrors {
    /// Creates a new empty error collection.
    #[must_use]
    #[inline]
    pub fn new() -> Self {
        Self { errors: Vec::new() }
    }

    /// Adds an error to the collection.
    #[inline]
    pub fn add(&mut self, error: ValidationError) {
        self.errors.push(error);
    }

    /// Adds multiple errors to the collection.
    #[inline]
    pub fn extend(&mut self, errors: impl IntoIterator<Item = ValidationError>) {
        self.errors.extend(errors);
    }

    /// Returns true if there are any errors.
    #[must_use]
    #[inline]
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    /// Returns the number of errors.
    #[must_use]
    #[inline]
    pub fn len(&self) -> usize {
        self.errors.len()
    }

    /// Returns true if empty.
    #[must_use]
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.errors.is_empty()
    }

    /// Returns all errors.
    #[must_use]
    #[inline]
    pub fn errors(&self) -> &[ValidationError] {
        &self.errors
    }

    /// Converts to a single error with nested errors.
    #[inline]
    pub fn into_single_error(self, message: impl Into<Cow<'static, str>>) -> ValidationError {
        ValidationError::new("validation_errors", message).with_nested(self.errors)
    }

    /// Converts to a Result.
    #[must_use = "result must be used"]
    #[inline]
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

impl IntoIterator for ValidationErrors {
    type Item = ValidationError;
    type IntoIter = std::vec::IntoIter<ValidationError>;

    fn into_iter(self) -> Self::IntoIter {
        self.errors.into_iter()
    }
}

impl<'a> IntoIterator for &'a ValidationErrors {
    type Item = &'a ValidationError;
    type IntoIter = std::slice::Iter<'a, ValidationError>;

    fn into_iter(self) -> Self::IntoIter {
        self.errors.iter()
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validation_error_size() {
        // Ensure our optimized struct is <= 80 bytes
        let size = std::mem::size_of::<ValidationError>();
        assert!(
            size <= 80,
            "ValidationError size is {size} bytes, expected <= 80"
        );
    }

    #[test]
    fn test_simple_error_no_allocation() {
        let error = ValidationError::new("test", "Test error");
        assert_eq!(error.code, "test");
        assert_eq!(error.message, "Test error");
        // Simple error should not allocate extras
        assert!(error.extras.is_none());
    }

    #[test]
    fn test_error_with_field() {
        let error = ValidationError::new("required", "Field is required").with_field("email");
        assert_eq!(error.field.as_deref(), Some("email"));
        // Field is inline, should not allocate extras
        assert!(error.extras.is_none());
    }

    #[test]
    fn test_error_with_params() {
        let error = ValidationError::new("min", "Too small")
            .with_param("min", "5")
            .with_param("actual", "3");

        assert_eq!(error.param("min"), Some("5"));
        assert_eq!(error.param("actual"), Some("3"));
        // Params trigger extras allocation
        assert!(error.extras.is_some());
    }

    #[test]
    fn test_nested_errors() {
        let error = ValidationError::new("object", "Object validation failed").with_nested(vec![
            ValidationError::new("email", "Invalid email").with_field("email"),
            ValidationError::new("age", "Too young").with_field("age"),
        ]);

        assert_eq!(error.nested().len(), 2);
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

    #[test]
    fn test_severity_default() {
        let error = ValidationError::new("test", "Test");
        assert_eq!(error.severity(), ErrorSeverity::Error);
    }

    #[test]
    fn test_severity_custom() {
        let error = ValidationError::new("test", "Test").with_severity(ErrorSeverity::Warning);
        assert_eq!(error.severity(), ErrorSeverity::Warning);
    }

    #[test]
    fn test_help_text() {
        let error = ValidationError::new("test", "Test").with_help("Try using a longer password");
        assert_eq!(error.help(), Some("Try using a longer password"));
    }

    #[test]
    fn test_empty_field_ignored() {
        let error = ValidationError::new("test", "Test").with_field("");
        assert!(error.field.is_none());
    }

    #[test]
    fn test_params_accessor() {
        let error = ValidationError::new("test", "Test")
            .with_param("a", "1")
            .with_param("b", "2");

        let params = error.params();
        assert_eq!(params.len(), 2);
        assert_eq!(params[0], (Cow::Borrowed("a"), Cow::Borrowed("1")));
    }

    #[test]
    fn test_has_nested() {
        let error_without = ValidationError::new("test", "Test");
        assert!(!error_without.has_nested());

        let error_with = ValidationError::new("test", "Test")
            .with_nested(vec![ValidationError::new("child", "Child")]);
        assert!(error_with.has_nested());
    }

    #[test]
    fn test_sensitive_params_are_redacted() {
        let error = ValidationError::new("auth", "Authentication failed")
            .with_param("password", "super-secret")
            .with_param("token", "api-token-123")
            .with_param("username", "alice");

        assert_eq!(error.param("password"), Some("[REDACTED]"));
        assert_eq!(error.param("token"), Some("[REDACTED]"));
        assert_eq!(error.param("username"), Some("alice"));
    }
}

//! [`ValidationError`] — the core structured error type.
//!
//! # Memory Optimization
//!
//! `ValidationError` is optimized for the common case (80 bytes):
//! - `code`, `message`, `field` are inline (most errors only use these)
//! - `params`, `nested`, `severity`, `help` are boxed in [`ErrorExtras`] (lazy allocated)

use std::{borrow::Cow, fmt};

use smallvec::SmallVec;

use super::{
    codes,
    pointer::{normalize_pointer, to_json_pointer},
    severity::ErrorSeverity,
};

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
///     .with_nested(vec![ValidationError::new(
///         "email_invalid",
///         "Invalid email format",
///     )]);
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
        if let Some(pointer) = to_json_pointer(field.as_ref()) {
            self.field = Some(Cow::Owned(pointer));
        }
        self
    }

    /// Sets the field path from a typed [`FieldPath`](super::super::field_path::FieldPath).
    ///
    /// This is the preferred way to set field paths when you have a
    /// pre-validated `FieldPath`.
    #[must_use = "builder methods must be chained or built"]
    #[inline]
    pub fn with_field_path(mut self, path: super::super::field_path::FieldPath) -> Self {
        self.field = Some(path.into_inner());
        self
    }

    /// Sets the field path using JSON Pointer (RFC 6901).
    ///
    /// Accepts pointers in `/a/b` format and URI fragment form `#/a/b`.
    #[must_use = "builder methods must be chained or built"]
    #[inline]
    pub fn with_pointer(mut self, pointer: impl Into<Cow<'static, str>>) -> Self {
        let pointer = pointer.into();
        if let Some(normalized) = normalize_pointer(pointer.as_ref()) {
            self.field = Some(Cow::Owned(normalized));
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

    /// Returns the field path as canonical JSON Pointer (RFC 6901).
    ///
    /// The field is already stored in normalized pointer form (set via
    /// `with_field` or `with_pointer`), so this is a zero-allocation accessor.
    #[must_use]
    #[inline]
    pub fn field_pointer(&self) -> Option<Cow<'_, str>> {
        self.field.as_deref().map(Cow::Borrowed)
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
            "pointer": self.field_pointer(),
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

/// Renders a message template by substituting `{name}` placeholders with
/// the matching entry from `params`. `{{` and `}}` are literal braces.
/// Unknown `{name}` is left as-is. Zero allocation when the template has
/// no `{` at all.
fn render_template<'a>(
    template: &'a str,
    params: &[(Cow<'static, str>, Cow<'static, str>)],
) -> Cow<'a, str> {
    if !template.contains('{') {
        return Cow::Borrowed(template);
    }

    let mut out = String::with_capacity(template.len());
    let mut chars = template.char_indices().peekable();
    while let Some((_, c)) = chars.next() {
        if c == '{' {
            if matches!(chars.peek(), Some((_, '{'))) {
                out.push('{');
                chars.next();
                continue;
            }
            let mut name = String::new();
            let mut closed = false;
            for (_, nc) in chars.by_ref() {
                if nc == '}' {
                    closed = true;
                    break;
                }
                name.push(nc);
            }
            if !closed {
                out.push('{');
                out.push_str(&name);
                continue;
            }
            match params.iter().find(|(k, _)| k.as_ref() == name) {
                Some((_, v)) => out.push_str(v.as_ref()),
                None => {
                    out.push('{');
                    out.push_str(&name);
                    out.push('}');
                },
            }
        } else if c == '}' {
            if matches!(chars.peek(), Some((_, '}'))) {
                out.push('}');
                chars.next();
            } else {
                out.push('}');
            }
        } else {
            out.push(c);
        }
    }
    Cow::Owned(out)
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let params = self.params();
        let rendered = render_template(self.message.as_ref(), params);
        if let Some(field) = &self.field {
            write!(f, "[{}] {}: {}", field, self.code, rendered)?;
        } else {
            write!(f, "{}: {}", self.code, rendered)?;
        }

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

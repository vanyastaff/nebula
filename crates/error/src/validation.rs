//! Canonical structured validation error shared across the workspace.
//!
//! This is the single validation-error type consumed by `nebula-validator`
//! (rules engine), `nebula-schema` (typed-config pipeline), and
//! `nebula-expression` (evaluator errors converted at the schema seam). It
//! lives in `nebula-error` so all three depend *down* onto one type rather
//! than maintaining parallel `ValidationError` definitions.
//!
//! # Memory layout
//!
//! [`ValidationError`] keeps the common case small (80 bytes): `code`,
//! `message`, and `field` are inline; rarely-used data (`params`, `nested`,
//! `severity`, `help`, `source`) is boxed in `ErrorExtras` and lazily
//! allocated on first use. Field paths are canonical RFC 6901 JSON Pointers.

use std::{borrow::Cow, fmt, sync::Arc};

use crate::ErrorSeverity;

// ============================================================================
// JSON POINTER HELPERS
// ============================================================================

/// Normalises an already-pointer-shaped string (`/a/b` or `#/a/b`) to canonical
/// form, or returns `None` if it is not a JSON Pointer.
fn normalize_pointer(pointer: &str) -> Option<String> {
    let pointer = pointer.trim();
    if pointer.is_empty() || pointer == "#" {
        return None;
    }
    if let Some(rest) = pointer.strip_prefix('#') {
        return normalize_pointer(rest);
    }
    if pointer.starts_with('/') {
        return Some(pointer.to_owned());
    }
    None
}

/// Converts a dot/bracket path (`a.b[0].c`), a JSON Pointer (`/a/b`), or a URI
/// fragment (`#/a/b`) to a canonical RFC 6901 JSON Pointer. Returns `None` for
/// empty input.
fn to_json_pointer(path: &str) -> Option<String> {
    let path = path.trim();
    if path.is_empty() {
        return None;
    }
    if let Some(pointer) = normalize_pointer(path) {
        return Some(pointer);
    }

    let mut segments: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut chars = path.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '.' => {
                if !current.is_empty() {
                    segments.push(std::mem::take(&mut current));
                }
            },
            '[' => {
                if !current.is_empty() {
                    segments.push(std::mem::take(&mut current));
                }
                let mut idx = String::new();
                let mut closed = false;
                for c in chars.by_ref() {
                    if c == ']' {
                        closed = true;
                        break;
                    }
                    idx.push(c);
                }
                if closed && !idx.is_empty() {
                    segments.push(idx);
                } else {
                    // Unclosed bracket — treat `[` and contents as literal.
                    current.push('[');
                    current.push_str(&idx);
                }
            },
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        segments.push(current);
    }
    if segments.is_empty() {
        return None;
    }

    let pointer = segments
        .into_iter()
        .filter(|segment| !segment.is_empty())
        .map(|segment| segment.replace('~', "~0").replace('/', "~1"))
        .fold(String::new(), |mut acc, segment| {
            acc.push('/');
            acc.push_str(&segment);
            acc
        });

    if pointer.is_empty() {
        None
    } else {
        Some(pointer)
    }
}

/// Escapes a single segment per RFC 6901 (`~` → `~0`, `/` → `~1`).
fn escape_segment(segment: &str, out: &mut String) {
    for ch in segment.chars() {
        match ch {
            '~' => out.push_str("~0"),
            '/' => out.push_str("~1"),
            _ => out.push(ch),
        }
    }
}

// ============================================================================
// FIELD PATH
// ============================================================================

/// A validated field path stored as a canonical RFC 6901 JSON Pointer.
///
/// Zero-overhead newtype over `Cow<'static, str>` (24 bytes). Constructed via
/// [`parse`](FieldPath::parse), [`single`](FieldPath::single), or
/// [`from_segments`](FieldPath::from_segments); composed via
/// [`push`](FieldPath::push) / [`append`](FieldPath::append).
///
/// # Examples
///
/// ```
/// use nebula_error::FieldPath;
///
/// assert_eq!(FieldPath::parse("user.name").unwrap().as_str(), "/user/name");
/// assert_eq!(FieldPath::parse("items[0]").unwrap().as_str(), "/items/0");
/// assert_eq!(FieldPath::single("user").push("email").as_str(), "/user/email");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FieldPath(Cow<'static, str>);

impl FieldPath {
    /// Parses a path from dot, bracket, JSON-Pointer, or URI-fragment form.
    /// Returns `None` for empty/invalid input.
    #[must_use]
    pub fn parse(path: impl AsRef<str>) -> Option<Self> {
        to_json_pointer(path.as_ref()).map(|p| Self(Cow::Owned(p)))
    }

    /// Creates a single-segment path (`/segment`).
    #[must_use]
    pub fn single(segment: impl AsRef<str>) -> Self {
        let segment = segment.as_ref();
        let mut pointer = String::with_capacity(1 + segment.len());
        pointer.push('/');
        escape_segment(segment, &mut pointer);
        Self(Cow::Owned(pointer))
    }

    /// Builds a path from an iterator of segments. Returns `None` if every
    /// segment is empty.
    #[must_use]
    pub fn from_segments<I, S>(segments: I) -> Option<Self>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut pointer = String::new();
        let mut has_segments = false;
        for segment in segments {
            let seg = segment.as_ref();
            if !seg.is_empty() {
                has_segments = true;
                pointer.push('/');
                escape_segment(seg, &mut pointer);
            }
        }
        if has_segments {
            Some(Self(Cow::Owned(pointer)))
        } else {
            None
        }
    }

    /// Returns the canonical JSON Pointer string.
    #[must_use]
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns an iterator over the unescaped segments.
    pub fn segments(&self) -> impl Iterator<Item = Cow<'_, str>> {
        self.0[1..].split('/').map(|segment| {
            if segment.contains('~') {
                Cow::Owned(segment.replace("~1", "/").replace("~0", "~"))
            } else {
                Cow::Borrowed(segment)
            }
        })
    }

    /// Number of segments.
    #[must_use]
    pub fn depth(&self) -> usize {
        self.0[1..].split('/').count()
    }

    /// The last segment, if any.
    #[must_use]
    pub fn last_segment(&self) -> Option<Cow<'_, str>> {
        self.segments().last()
    }

    /// The parent path (all but the last segment), or `None` for single-segment.
    #[must_use]
    pub fn parent(&self) -> Option<Self> {
        match self.0.rfind('/') {
            Some(0) | None => None,
            Some(pos) => Some(Self(Cow::Owned(self.0[..pos].to_owned()))),
        }
    }

    /// Appends a segment, returning the extended path.
    #[must_use]
    pub fn push(&self, segment: impl AsRef<str>) -> Self {
        let segment = segment.as_ref();
        let mut pointer = String::with_capacity(self.0.len() + 1 + segment.len());
        pointer.push_str(&self.0);
        pointer.push('/');
        escape_segment(segment, &mut pointer);
        Self(Cow::Owned(pointer))
    }

    /// Appends all segments from another path.
    #[must_use]
    pub fn append(&self, other: &FieldPath) -> Self {
        let mut pointer = String::with_capacity(self.0.len() + other.0.len());
        pointer.push_str(&self.0);
        pointer.push_str(&other.0);
        Self(Cow::Owned(pointer))
    }

    /// Returns `true` when this path begins with `prefix`.
    #[must_use]
    pub fn starts_with(&self, prefix: &FieldPath) -> bool {
        let (this, pre) = (self.0.as_ref(), prefix.0.as_ref());
        this == pre
            || (this.starts_with(pre) && this.as_bytes().get(pre.len()).copied() == Some(b'/'))
    }

    /// Converts into the inner `Cow<'static, str>`.
    #[must_use]
    #[inline]
    pub fn into_inner(self) -> Cow<'static, str> {
        self.0
    }
}

impl fmt::Display for FieldPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<FieldPath> for Cow<'static, str> {
    fn from(path: FieldPath) -> Self {
        path.0
    }
}

impl From<FieldPath> for String {
    fn from(path: FieldPath) -> Self {
        path.0.into_owned()
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for FieldPath {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for FieldPath {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = <Cow<'de, str> as serde::Deserialize>::deserialize(d)?;
        FieldPath::parse(raw.as_ref())
            .ok_or_else(|| serde::de::Error::custom(format!("invalid field path: {raw:?}")))
    }
}

// ============================================================================
// ERROR EXTRAS (boxed rare fields)
// ============================================================================

/// Extended, rarely-used error data. Boxed to keep [`ValidationError`] at
/// 80 bytes.
#[derive(Debug, Clone, Default)]
struct ErrorExtras {
    params: Vec<(Cow<'static, str>, Cow<'static, str>)>,
    nested: Vec<ValidationError>,
    severity: ErrorSeverity,
    help: Option<Cow<'static, str>>,
    source: Option<Arc<dyn std::error::Error + Send + Sync>>,
}

impl PartialEq for ErrorExtras {
    fn eq(&self, other: &Self) -> bool {
        // `source` is a trait object without `PartialEq`; compare its `Display`.
        self.params == other.params
            && self.nested == other.nested
            && self.severity == other.severity
            && self.help == other.help
            && self.source.as_ref().map(ToString::to_string)
                == other.source.as_ref().map(ToString::to_string)
    }
}

// ============================================================================
// VALIDATION ERROR
// ============================================================================

/// A structured validation error.
///
/// `code` and `message` use `Cow<'static, str>` for zero allocation when known
/// at compile time. `field` is a canonical RFC 6901 JSON Pointer. Severity,
/// params, nested errors, help, and an optional source live in lazily-boxed
/// extras.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidationError {
    /// Machine-readable code (e.g. `min_length`, `required`, `type_mismatch`).
    pub code: Cow<'static, str>,
    /// Human-readable English message; may contain `{name}` placeholders
    /// rendered against [`params`](Self::params).
    pub message: Cow<'static, str>,
    /// Canonical RFC 6901 JSON Pointer to the offending field, if any.
    pub field: Option<Cow<'static, str>>,
    /// Lazily-allocated rare fields.
    extras: Option<Box<ErrorExtras>>,
}

impl ValidationError {
    /// Creates an error from a code and message.
    #[inline]
    #[must_use]
    pub fn new(code: impl Into<Cow<'static, str>>, message: impl Into<Cow<'static, str>>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            field: None,
            extras: None,
        }
    }

    /// Begins building an error fluently from a code (schema-style entry point).
    ///
    /// The default message is the code itself, so `builder(code).build()`
    /// renders sensibly rather than with an empty message.
    pub fn builder(code: impl Into<Cow<'static, str>>) -> ValidationErrorBuilder {
        let code = code.into();
        let message = code.clone();
        ValidationErrorBuilder {
            error: Self {
                code,
                message,
                field: None,
                extras: None,
            },
        }
    }

    // ----- builder-on-self (validator style) -------------------------------

    /// Sets the field path by normalising a dotted/bracketed string. Empty or
    /// invalid input leaves the field unset.
    #[must_use = "builder methods must be chained or built"]
    #[inline]
    pub fn with_field(mut self, field: impl Into<Cow<'static, str>>) -> Self {
        let field = field.into();
        if let Some(pointer) = to_json_pointer(field.as_ref()) {
            self.field = Some(Cow::Owned(pointer));
        }
        self
    }

    /// Sets the field path from a structured [`FieldPath`].
    #[must_use = "builder methods must be chained or built"]
    #[inline]
    pub fn with_field_path(mut self, path: FieldPath) -> Self {
        self.field = Some(path.into_inner());
        self
    }

    /// Sets the field path from a JSON Pointer (`/a/b`) or fragment (`#/a/b`).
    #[must_use = "builder methods must be chained or built"]
    #[inline]
    pub fn with_pointer(mut self, pointer: impl Into<Cow<'static, str>>) -> Self {
        let pointer = pointer.into();
        if let Some(normalized) = normalize_pointer(pointer.as_ref()) {
            self.field = Some(Cow::Owned(normalized));
        }
        self
    }

    /// Adds a message-template / metadata parameter.
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

    /// Replaces nested errors.
    #[must_use = "builder methods must be chained or built"]
    #[inline]
    pub fn with_nested(mut self, errors: Vec<ValidationError>) -> Self {
        if !errors.is_empty() {
            self.extras_mut().nested = errors;
        }
        self
    }

    /// Appends a single nested error.
    #[must_use = "builder methods must be chained or built"]
    #[inline]
    pub fn with_nested_error(mut self, error: ValidationError) -> Self {
        self.extras_mut().nested.push(error);
        self
    }

    /// Sets the severity.
    #[must_use = "builder methods must be chained or built"]
    #[inline]
    pub fn with_severity(mut self, severity: ErrorSeverity) -> Self {
        self.extras_mut().severity = severity;
        self
    }

    /// Attaches fix-it help text.
    #[must_use = "builder methods must be chained or built"]
    #[inline]
    pub fn with_help(mut self, help: impl Into<Cow<'static, str>>) -> Self {
        self.extras_mut().help = Some(help.into());
        self
    }

    /// Attaches an underlying cause.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_source<E>(mut self, err: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        self.extras_mut().source = Some(Arc::new(err));
        self
    }

    // ----- accessors -------------------------------------------------------

    /// Looks up a parameter value by key.
    #[must_use]
    #[inline]
    pub fn param(&self, key: &str) -> Option<&str> {
        self.params()
            .iter()
            .find(|(k, _)| k.as_ref() == key)
            .map(|(_, v)| v.as_ref())
    }

    /// All parameters.
    #[must_use]
    #[inline]
    pub fn params(&self) -> &[(Cow<'static, str>, Cow<'static, str>)] {
        self.extras.as_ref().map_or(&[], |e| e.params.as_slice())
    }

    /// Nested errors.
    #[must_use]
    #[inline]
    pub fn nested(&self) -> &[ValidationError] {
        self.extras.as_ref().map_or(&[], |e| e.nested.as_slice())
    }

    /// Whether this error carries nested errors.
    #[must_use]
    #[inline]
    pub fn has_nested(&self) -> bool {
        self.extras.as_ref().is_some_and(|e| !e.nested.is_empty())
    }

    /// Severity level (defaults to `Error`).
    #[must_use]
    #[inline]
    pub fn severity(&self) -> ErrorSeverity {
        self.extras
            .as_ref()
            .map_or(ErrorSeverity::Error, |e| e.severity)
    }

    /// Help text, if any.
    #[must_use]
    #[inline]
    pub fn help(&self) -> Option<&str> {
        self.extras.as_ref()?.help.as_deref()
    }

    /// Underlying cause, if any.
    #[must_use]
    #[inline]
    pub fn source_error(&self) -> Option<&(dyn std::error::Error + Send + Sync + 'static)> {
        self.extras.as_ref()?.source.as_deref()
    }

    /// The field as a canonical JSON Pointer (zero-allocation borrow).
    #[must_use]
    #[inline]
    pub fn field_pointer(&self) -> Option<Cow<'_, str>> {
        self.field.as_deref().map(Cow::Borrowed)
    }

    /// Total error count including nested errors.
    #[must_use]
    pub fn total_error_count(&self) -> usize {
        1 + self
            .nested()
            .iter()
            .map(ValidationError::total_error_count)
            .sum::<usize>()
    }

    /// Flattens this error and all nested errors depth-first.
    #[must_use]
    pub fn flatten(&self) -> Vec<&ValidationError> {
        let mut result = vec![self];
        for nested in self.nested() {
            result.extend(nested.flatten());
        }
        result
    }

    /// Renders the message template against this error's params. Zero
    /// allocation when the message has no `{` placeholder.
    #[must_use]
    pub fn rendered_message(&self) -> Cow<'_, str> {
        render_template(self.message.as_ref(), self.params())
    }

    /// Whether this is a hard error (severity `Error`).
    #[must_use]
    pub fn is_error(&self) -> bool {
        self.severity().is_error()
    }

    /// Whether this is a warning (severity `Warning`).
    #[must_use]
    pub fn is_warning(&self) -> bool {
        self.severity().is_warning()
    }

    #[inline]
    fn extras_mut(&mut self) -> &mut ErrorExtras {
        self.extras
            .get_or_insert_with(|| Box::new(ErrorExtras::default()))
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let rendered = render_template(self.message.as_ref(), self.params());
        if let Some(field) = &self.field {
            write!(f, "[{}] {}: {}", field, self.code, rendered)?;
        } else {
            write!(f, "{}: {}", self.code, rendered)?;
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

impl std::error::Error for ValidationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.extras
            .as_ref()?
            .source
            .as_deref()
            .map(|e| e as &(dyn std::error::Error + 'static))
    }
}

impl crate::Classify for ValidationError {
    fn category(&self) -> crate::ErrorCategory {
        crate::ErrorCategory::Validation
    }

    fn code(&self) -> crate::ErrorCode {
        crate::codes::VALIDATION
    }

    fn severity(&self) -> ErrorSeverity {
        ValidationError::severity(self)
    }
}

#[cfg(feature = "serde")]
impl ValidationError {
    /// Converts to a JSON structure for serialization / API mapping.
    #[must_use]
    pub fn to_json_value(&self) -> serde_json::Value {
        use serde_json::{Map, Value, json};

        let params: Map<String, Value> = self
            .params()
            .iter()
            .map(|(k, v)| (k.to_string(), Value::String(v.to_string())))
            .collect();

        json!({
            "code": self.code,
            "message": self.rendered_message(),
            "field": self.field,
            "pointer": self.field_pointer(),
            "params": params,
            "severity": self.severity().as_str(),
            "help": self.help(),
            "nested": self.nested().iter().map(ValidationError::to_json_value).collect::<Vec<_>>(),
        })
    }
}

// ============================================================================
// CONVENIENCE CONSTRUCTORS
// ============================================================================

impl ValidationError {
    /// A `required` error.
    #[inline]
    #[must_use]
    pub fn required(field: impl Into<Cow<'static, str>>) -> Self {
        Self::new("required", "This field is required").with_field(field)
    }

    /// A `min_length` error.
    #[inline]
    #[must_use]
    pub fn min_length(field: impl Into<Cow<'static, str>>, min: usize, actual: usize) -> Self {
        Self::new("min_length", format!("Must be at least {min} characters"))
            .with_field(field)
            .with_param("min", min.to_string())
            .with_param("actual", actual.to_string())
    }

    /// A `max_length` error.
    #[inline]
    #[must_use]
    pub fn max_length(field: impl Into<Cow<'static, str>>, max: usize, actual: usize) -> Self {
        Self::new("max_length", format!("Must be at most {max} characters"))
            .with_field(field)
            .with_param("max", max.to_string())
            .with_param("actual", actual.to_string())
    }

    /// An `invalid_format` error.
    #[inline]
    #[must_use]
    pub fn invalid_format(
        field: impl Into<Cow<'static, str>>,
        expected: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self::new("invalid_format", "Invalid format")
            .with_field(field)
            .with_param("expected", expected)
    }

    /// A `type_mismatch` error.
    #[inline]
    #[must_use]
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

    /// An `out_of_range` error.
    #[inline]
    #[must_use]
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

    /// An `exact_length` error.
    #[inline]
    #[must_use]
    pub fn exact_length(
        field: impl Into<Cow<'static, str>>,
        expected: usize,
        actual: usize,
    ) -> Self {
        Self::new(
            "exact_length",
            format!("Must be exactly {expected} characters"),
        )
        .with_field(field)
        .with_param("expected", expected.to_string())
        .with_param("actual", actual.to_string())
    }

    /// A `length_range` error.
    #[inline]
    #[must_use]
    pub fn length_range(
        field: impl Into<Cow<'static, str>>,
        min: usize,
        max: usize,
        actual: usize,
    ) -> Self {
        Self::new(
            "length_range",
            format!("Must be between {min} and {max} characters"),
        )
        .with_field(field)
        .with_param("min", min.to_string())
        .with_param("max", max.to_string())
        .with_param("actual", actual.to_string())
    }

    /// A `custom` error with a free-form message.
    #[inline]
    #[must_use]
    pub fn custom(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new("custom", message)
    }
}

// ============================================================================
// BUILDER (schema style)
// ============================================================================

/// Fluent builder for [`ValidationError`] (schema-style entry point via
/// [`ValidationError::builder`]).
#[must_use = "call .build() to produce a ValidationError"]
#[derive(Debug, Clone)]
pub struct ValidationErrorBuilder {
    error: ValidationError,
}

impl ValidationErrorBuilder {
    /// Sets the field path from a structured [`FieldPath`].
    #[must_use = "builder methods must be chained or built"]
    pub fn at(mut self, path: FieldPath) -> Self {
        self.error.field = Some(path.into_inner());
        self
    }

    /// Sets the field path by normalising a dotted/bracketed string.
    #[must_use = "builder methods must be chained or built"]
    pub fn at_field(mut self, field: impl Into<Cow<'static, str>>) -> Self {
        self.error = self.error.with_field(field);
        self
    }

    /// Downgrades severity to [`ErrorSeverity::Warning`].
    #[must_use = "builder methods must be chained or built"]
    pub fn warn(mut self) -> Self {
        self.error.extras_mut().severity = ErrorSeverity::Warning;
        self
    }

    /// Sets the severity explicitly.
    #[must_use = "builder methods must be chained or built"]
    pub fn severity(mut self, severity: ErrorSeverity) -> Self {
        self.error.extras_mut().severity = severity;
        self
    }

    /// Sets the human-readable message.
    #[must_use = "builder methods must be chained or built"]
    pub fn message(mut self, msg: impl Into<Cow<'static, str>>) -> Self {
        self.error.message = msg.into();
        self
    }

    /// Attaches a named parameter.
    #[must_use = "builder methods must be chained or built"]
    pub fn param(
        mut self,
        key: impl Into<Cow<'static, str>>,
        value: impl Into<Cow<'static, str>>,
    ) -> Self {
        self.error = self.error.with_param(key, value);
        self
    }

    /// Attaches help text.
    #[must_use = "builder methods must be chained or built"]
    pub fn help(mut self, help: impl Into<Cow<'static, str>>) -> Self {
        self.error.extras_mut().help = Some(help.into());
        self
    }

    /// Attaches a nested error.
    #[must_use = "builder methods must be chained or built"]
    pub fn nested(mut self, error: ValidationError) -> Self {
        self.error.extras_mut().nested.push(error);
        self
    }

    /// Attaches an underlying cause.
    #[must_use = "builder methods must be chained or built"]
    pub fn source<E>(mut self, err: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        self.error.extras_mut().source = Some(Arc::new(err));
        self
    }

    /// Finalises the error.
    #[must_use]
    pub fn build(self) -> ValidationError {
        self.error
    }
}

// ============================================================================
// VALIDATION ERRORS (collection)
// ============================================================================

/// A flat collection of [`ValidationError`] values accumulated during
/// validation.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ValidationErrors {
    errors: Vec<ValidationError>,
}

impl ValidationErrors {
    /// An empty collection.
    #[must_use]
    #[inline]
    pub fn new() -> Self {
        Self { errors: Vec::new() }
    }

    /// Adds an error.
    #[inline]
    pub fn add(&mut self, error: ValidationError) {
        self.errors.push(error);
    }

    /// Adds multiple errors.
    #[inline]
    pub fn extend(&mut self, errors: impl IntoIterator<Item = ValidationError>) {
        self.errors.extend(errors);
    }

    /// Whether any errors are present.
    #[must_use]
    #[inline]
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    /// Number of errors.
    #[must_use]
    #[inline]
    pub fn len(&self) -> usize {
        self.errors.len()
    }

    /// Whether the collection is empty.
    #[must_use]
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.errors.is_empty()
    }

    /// All errors as a slice.
    #[must_use]
    #[inline]
    pub fn errors(&self) -> &[ValidationError] {
        &self.errors
    }

    /// Mutable reference to the last error, if any.
    #[inline]
    pub fn last_mut(&mut self) -> Option<&mut ValidationError> {
        self.errors.last_mut()
    }

    /// Collapses into a single error carrying the rest as nested errors.
    #[inline]
    #[must_use]
    pub fn into_single_error(self, message: impl Into<Cow<'static, str>>) -> ValidationError {
        ValidationError::new("validation_errors", message).with_nested(self.errors)
    }

    /// Converts to a `Result`, yielding `ok_value` when empty.
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
// VALIDATION REPORT (severity-aware aggregator)
// ============================================================================

/// Accumulates validation issues with severity, separating hard errors from
/// warnings. Used by the schema pipeline for build / lint / validate / resolve.
#[derive(Clone, Debug, Default)]
pub struct ValidationReport {
    issues: Vec<ValidationError>,
}

impl ValidationReport {
    /// An empty report.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends an issue.
    pub fn push(&mut self, issue: ValidationError) {
        self.issues.push(issue);
    }

    /// Iterates over hard errors only (severity `Error`).
    pub fn errors(&self) -> impl Iterator<Item = &ValidationError> {
        self.issues.iter().filter(|i| i.severity().is_error())
    }

    /// Iterates over warnings only (severity `Warning`).
    pub fn warnings(&self) -> impl Iterator<Item = &ValidationError> {
        self.issues.iter().filter(|i| i.severity().is_warning())
    }

    /// Iterates over all issues.
    pub fn iter(&self) -> std::slice::Iter<'_, ValidationError> {
        self.issues.iter()
    }

    /// Iterates over issues whose field path starts with `prefix`.
    pub fn at_path<'a>(
        &'a self,
        prefix: &'a FieldPath,
    ) -> impl Iterator<Item = &'a ValidationError> {
        self.issues.iter().filter(move |i| {
            i.field
                .as_deref()
                .and_then(FieldPath::parse)
                .is_some_and(|p| p.starts_with(prefix))
        })
    }

    /// Whether the report has at least one hard error.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.errors().next().is_some()
    }

    /// Whether the report has at least one warning.
    #[must_use]
    pub fn has_warnings(&self) -> bool {
        self.warnings().next().is_some()
    }

    /// Total number of issues (errors + warnings).
    #[must_use]
    pub fn len(&self) -> usize {
        self.issues.len()
    }

    /// Whether the report has no issues.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.issues.is_empty()
    }
}

impl From<ValidationError> for ValidationReport {
    fn from(err: ValidationError) -> Self {
        let mut r = Self::new();
        r.push(err);
        r
    }
}

impl Extend<ValidationError> for ValidationReport {
    fn extend<I: IntoIterator<Item = ValidationError>>(&mut self, iter: I) {
        self.issues.extend(iter);
    }
}

impl IntoIterator for ValidationReport {
    type Item = ValidationError;
    type IntoIter = std::vec::IntoIter<ValidationError>;
    fn into_iter(self) -> Self::IntoIter {
        self.issues.into_iter()
    }
}

impl<'a> IntoIterator for &'a ValidationReport {
    type Item = &'a ValidationError;
    type IntoIter = std::slice::Iter<'a, ValidationError>;
    fn into_iter(self) -> Self::IntoIter {
        self.issues.iter()
    }
}

impl FromIterator<ValidationError> for ValidationReport {
    fn from_iter<I: IntoIterator<Item = ValidationError>>(iter: I) -> Self {
        Self {
            issues: iter.into_iter().collect(),
        }
    }
}

impl fmt::Display for ValidationReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.issues.is_empty() {
            return write!(f, "(no issues)");
        }
        for (i, issue) in self.issues.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "{issue}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ValidationReport {}

// ============================================================================
// HELPERS
// ============================================================================

/// Renders `{name}` placeholders against `params`. `{{`/`}}` are literal
/// braces; unknown `{name}` is left as-is. Zero allocation when `template`
/// has no `{`.
#[must_use]
pub fn render_template<'a>(
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
            if let Some((_, v)) = params.iter().find(|(k, _)| k.as_ref() == name) {
                out.push_str(v.as_ref());
            } else {
                out.push('{');
                out.push_str(&name);
                out.push('}');
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

#[inline]
fn redact_if_sensitive(key: &str, value: Cow<'static, str>) -> Cow<'static, str> {
    let lowered = key.to_ascii_lowercase();
    const SENSITIVE: [&str; 6] = [
        "password",
        "secret",
        "token",
        "api_key",
        "apikey",
        "credential",
    ];
    if SENSITIVE.iter().any(|pattern| lowered.contains(pattern)) {
        Cow::Borrowed("[REDACTED]")
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_error_is_80_bytes() {
        assert!(size_of::<ValidationError>() <= 80);
    }

    #[test]
    fn field_path_parse_forms() {
        assert_eq!(
            FieldPath::parse("user.name").unwrap().as_str(),
            "/user/name"
        );
        assert_eq!(
            FieldPath::parse("items[0].name").unwrap().as_str(),
            "/items/0/name"
        );
        assert_eq!(
            FieldPath::parse("#/user/email").unwrap().as_str(),
            "/user/email"
        );
        assert!(FieldPath::parse("").is_none());
    }

    #[test]
    fn field_path_compose_and_starts_with() {
        let p = FieldPath::single("user").push("email");
        assert_eq!(p.as_str(), "/user/email");
        assert!(p.starts_with(&FieldPath::single("user")));
        assert!(!FieldPath::single("user").starts_with(&p));
        assert!(!FieldPath::single("us").starts_with(&FieldPath::single("user")));
    }

    #[test]
    fn with_field_normalizes_to_pointer() {
        let e = ValidationError::new("t", "m").with_field("service.port");
        assert_eq!(e.field.as_deref(), Some("/service/port"));
        let e = ValidationError::new("t", "m").with_field("items[0].name");
        assert_eq!(e.field_pointer().as_deref(), Some("/items/0/name"));
    }

    #[test]
    fn builder_style() {
        let e = ValidationError::builder("max_length")
            .at(FieldPath::parse("user.email").unwrap())
            .message("too long")
            .param("max", "20")
            .warn()
            .build();
        assert_eq!(e.code, "max_length");
        assert_eq!(e.field.as_deref(), Some("/user/email"));
        assert_eq!(e.param("max"), Some("20"));
        assert!(e.is_warning());
    }

    #[test]
    fn convenience_ctors() {
        assert_eq!(ValidationError::min_length("n", 3, 1).code, "min_length");
        assert_eq!(
            ValidationError::exact_length("n", 3, 1).code,
            "exact_length"
        );
        assert_eq!(
            ValidationError::length_range("n", 1, 3, 5).code,
            "length_range"
        );
    }

    #[test]
    fn template_renders_and_redacts() {
        let e = ValidationError::new("x", "min {min}").with_param("min", "5");
        assert_eq!(e.rendered_message(), "min 5");
        let e = ValidationError::new("x", "y").with_param("password", "s3cr3t");
        assert_eq!(e.param("password"), Some("[REDACTED]"));
    }

    #[test]
    fn display_root_and_pathed() {
        assert_eq!(
            ValidationError::new("required", "missing").to_string(),
            "required: missing"
        );
        let e = ValidationError::new("required", "missing").with_field("x");
        assert_eq!(e.to_string(), "[/x] required: missing");
    }

    #[test]
    fn nested_flatten() {
        let e = ValidationError::new("root", "r").with_nested(vec![
            ValidationError::new("a", "1").with_nested(vec![ValidationError::new("g", "x")]),
            ValidationError::new("b", "2"),
        ]);
        assert_eq!(e.flatten().len(), 4);
        assert_eq!(e.total_error_count(), 4);
    }

    #[test]
    fn errors_collection() {
        let mut errs = ValidationErrors::new();
        errs.add(ValidationError::new("a", "1"));
        errs.add(ValidationError::new("b", "2"));
        assert_eq!(errs.len(), 2);
        assert!(errs.has_errors());
        assert_eq!(errs.into_single_error("failed").total_error_count(), 3);
    }

    #[test]
    fn report_splits_and_at_path() {
        let mut report = ValidationReport::new();
        report.push(ValidationError::new("required", "r").with_field("user.email"));
        report.push(ValidationError::builder("notice").warn().build());
        assert_eq!(report.errors().count(), 1);
        assert_eq!(report.warnings().count(), 1);
        let prefix = FieldPath::single("user");
        assert_eq!(report.at_path(&prefix).count(), 1);
    }

    #[test]
    fn report_display() {
        assert_eq!(ValidationReport::new().to_string(), "(no issues)");
    }

    #[test]
    fn source_chain() {
        #[derive(Debug)]
        struct Inner;
        impl fmt::Display for Inner {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("inner")
            }
        }
        impl std::error::Error for Inner {}
        let e = ValidationError::new("x", "y").with_source(Inner);
        assert!(std::error::Error::source(&e).is_some());
    }
}

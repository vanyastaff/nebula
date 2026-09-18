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
    kind: ValidationErrorKind,
    /// Parameters for the error message template.
    /// SmallVec optimizes for 0-2 params inline (covers ~95% of cases).
    params: SmallVec<[(Cow<'static, str>, Cow<'static, str>); 2]>,

    /// Nested validation errors for complex objects.
    nested: Vec<ValidationError>,

    /// Depth of the deepest node in this error's subtree, counting `self` as 1.
    ///
    /// Cached so the construction-time depth bound is checked in O(1) instead
    /// of re-walking the subtree on every `with_nested*` call.
    max_depth: usize,

    /// Severity level (defaults to Error).
    severity: ErrorSeverity,

    /// Help text or suggestion for fixing the error.
    help: Option<Cow<'static, str>>,
}

impl Default for ErrorExtras {
    fn default() -> Self {
        Self {
            kind: ValidationErrorKind::Violation,
            params: SmallVec::new(),
            nested: Vec::new(),
            max_depth: 1,
            severity: ErrorSeverity::Error,
            help: None,
        }
    }
}

/// Maximum depth of a nested error tree, counting the outermost error as one.
///
/// [`ValidationError::with_nested`] and
/// [`ValidationError::with_nested_error`] enforce this bound during
/// construction: children that would exceed it are replaced by a single
/// `nested_errors_omitted` diagnostic. The bound exists because everything
/// that touches the tree recurses once per level — the `kind`, `flatten`,
/// `total_error_count`, `to_json_value`, and `Display` traversals, and the
/// derived `Clone` / `PartialEq` / `Debug` / `Drop` impls. A tree capped here
/// cannot overflow the stack through any of them, including at drop time.
pub const MAX_ERROR_TREE_DEPTH: usize = 64;

/// Whether a diagnostic describes rejected input or an evaluation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationErrorKind {
    /// The input does not satisfy an executable rule.
    Violation,
    /// The rule is not valid for the requested operation.
    InvalidRule,
    /// Evaluation needs context or an evaluator that is unavailable.
    Unavailable,
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

    /// Optional field path, always in canonical RFC 6901 pointer form.
    ///
    /// Private on purpose: every writer (`with_field`, `with_field_path`,
    /// `with_pointer`) normalizes its input, so `field` and
    /// [`field_pointer`](Self::field_pointer) cannot disagree. A public field
    /// would let safe external code store raw dot notation here and produce an
    /// envelope whose `field` key contradicts its `pointer` key.
    field: Option<Cow<'static, str>>,

    /// Extended error data (params, nested, severity, help).
    /// Boxed to reduce struct size; lazily allocated on first use.
    extras: Option<Box<ErrorExtras>>,
}

impl ValidationError {
    /// Classifies this diagnostic for logical rule composition.
    #[must_use]
    pub fn kind(&self) -> ValidationErrorKind {
        let own = self
            .extras
            .as_ref()
            .map_or(ValidationErrorKind::Violation, |extras| extras.kind);
        if own != ValidationErrorKind::Violation {
            return own;
        }
        self.nested()
            .iter()
            .map(Self::kind)
            .find(|kind| *kind != ValidationErrorKind::Violation)
            .unwrap_or(ValidationErrorKind::Violation)
    }

    /// Whether this diagnostic is a failed alternative rather than structural.
    ///
    /// Logical combinators (`and`/`or`/`not`/`any`, `Rule::all`/`any`/`not`)
    /// treat only `Violation` as "this branch failed" and count it toward their
    /// aggregate verdict. An `InvalidRule` or `Unavailable` diagnostic is
    /// structural: it aborts the whole combinator instead of being absorbed by
    /// a passing sibling, so a misconfigured or un-evaluable branch cannot be
    /// masked.
    #[must_use]
    pub fn is_violation(&self) -> bool {
        self.kind() == ValidationErrorKind::Violation
    }

    pub(crate) fn invalid_rule(message: &'static str) -> Self {
        let mut error = Self::new("invalid_rule", message);
        error.extras_mut().kind = ValidationErrorKind::InvalidRule;
        error
    }

    pub(crate) fn unavailable(message: &'static str) -> Self {
        let mut error = Self::new("evaluation_unavailable", message);
        error.extras_mut().kind = ValidationErrorKind::Unavailable;
        error
    }
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

    /// Prefixes this error's field path with `segment`.
    ///
    /// Field combinators use this to compose `parent.child` when the inner
    /// validator already recorded `child`. The stored path is a JSON Pointer,
    /// so the two halves join directly: `/parent` + `/child`. Joining the
    /// segment with a dot instead would store `/parent/~1child`, escaping the
    /// separator into the key name.
    pub(crate) fn prepend_field_segment(mut self, segment: &str) -> Self {
        let parent = super::super::field_path::FieldPath::single(segment);
        let Some(child) = self.field.take() else {
            return self.with_field_path(parent);
        };
        // Both halves are canonical pointers by construction: every writer on
        // this type normalizes, and `field` is private to this module.
        let mut pointer = String::with_capacity(parent.as_str().len() + child.len());
        pointer.push_str(parent.as_str());
        pointer.push_str(&child);
        self.field = Some(Cow::Owned(pointer));
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
    ///
    /// Children are trimmed to fit [`MAX_ERROR_TREE_DEPTH`]: everything below
    /// the ceiling is dropped and its count recorded as a
    /// `nested_errors_omitted` parameter on the node where the cut happened.
    /// See that constant for why the bound is enforced at construction.
    #[must_use = "builder methods must be chained or built"]
    #[inline]
    pub fn with_nested(mut self, errors: Vec<ValidationError>) -> Self {
        if errors.is_empty() {
            return self;
        }
        let room = MAX_ERROR_TREE_DEPTH.saturating_sub(self.max_depth());
        let extras = self.extras_mut();
        for error in errors {
            if room == 0 {
                bump_omitted(extras, 1);
                continue;
            }
            let error = trim_to_depth(error, room);
            extras.max_depth = extras.max_depth.max(1 + error.max_depth());
            extras.nested.push(error);
        }
        self
    }

    /// Adds a single nested error.
    ///
    /// Enforces the same [`MAX_ERROR_TREE_DEPTH`] bound as
    /// [`Self::with_nested`].
    #[must_use = "builder methods must be chained or built"]
    #[inline]
    pub fn with_nested_error(mut self, error: ValidationError) -> Self {
        let room = MAX_ERROR_TREE_DEPTH.saturating_sub(self.max_depth());
        let extras = self.extras_mut();
        if room == 0 {
            bump_omitted(extras, 1);
            return self;
        }
        let error = trim_to_depth(error, room);
        extras.max_depth = extras.max_depth.max(1 + error.max_depth());
        extras.nested.push(error);
        self
    }

    /// Depth of the deepest node in this error's subtree, counting `self` as 1.
    #[must_use]
    #[inline]
    pub fn max_depth(&self) -> usize {
        self.extras.as_ref().map_or(1, |extras| extras.max_depth)
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
            .map_or(ErrorSeverity::Error, |e| e.severity)
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
            "kind": self.kind(),
            "message": self.message,
            "field": self.field,
            "pointer": self.field_pointer(),
            "params": params,
            "severity": format!("{:?}", self.severity()),
            "help": self.help(),
            "nested": self.nested().iter().map(ValidationError::to_json_value).collect::<Vec<_>>(),
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

/// Trim an error subtree so it fits within `room` levels.
///
/// Keeps the outermost `room` levels and replaces the children below the cut
/// with an omitted-count parameter on the node where the cut happened. The
/// result's `max_depth()` is `<= room`, so the caller can splice it in without
/// re-walking the subtree.
fn trim_to_depth(error: ValidationError, room: usize) -> ValidationError {
    if room == 0 {
        // Only reachable defensively; callers guard `room == 0` before calling.
        return error;
    }
    let ValidationError {
        code,
        message,
        field,
        mut extras,
    } = error;

    let Some(inner) = extras.take() else {
        return ValidationError {
            code,
            message,
            field,
            extras: None,
        };
    };
    let ErrorExtras {
        kind,
        params,
        nested,
        max_depth,
        severity,
        help,
    } = *inner;

    if max_depth <= room {
        return ValidationError {
            code,
            message,
            field,
            extras: Some(Box::new(ErrorExtras {
                kind,
                params,
                nested,
                max_depth,
                severity,
                help,
            })),
        };
    }

    let mut params = params;
    let mut trimmed_nested = Vec::with_capacity(nested.len());
    let mut omitted = 0usize;
    for child in nested {
        if room <= 1 {
            omitted += count_errors(&child);
            continue;
        }
        let child = trim_to_depth(child, room - 1);
        trimmed_nested.push(child);
    }
    if omitted > 0 {
        push_omitted_param(&mut params, omitted);
    }

    let trimmed_depth = trimmed_nested
        .iter()
        .map(ValidationError::max_depth)
        .max()
        .map_or(1, |depth| depth + 1);

    ValidationError {
        code,
        message,
        field,
        extras: Some(Box::new(ErrorExtras {
            kind,
            params,
            nested: trimmed_nested,
            max_depth: trimmed_depth,
            severity,
            help,
        })),
    }
}

/// Record that `count` diagnostics were dropped into an error's params.
fn push_omitted_param(
    params: &mut SmallVec<[(Cow<'static, str>, Cow<'static, str>); 2]>,
    count: usize,
) {
    let key = Cow::Borrowed("nested_errors_omitted");
    match params.iter_mut().find(|(existing, _)| *existing == key) {
        Some((_, value)) => {
            let total = value.parse::<usize>().unwrap_or(0).saturating_add(count);
            *value = Cow::Owned(total.to_string());
        },
        None => params.push((key, Cow::Owned(count.to_string()))),
    }
}

/// Increment an extras block's omitted-diagnostic count.
fn bump_omitted(extras: &mut ErrorExtras, count: usize) {
    push_omitted_param(&mut extras.params, count);
}

/// Count the nodes in an error subtree, iteratively.
fn count_errors(root: &ValidationError) -> usize {
    let mut count = 0;
    let mut pending = vec![root];
    while let Some(error) = pending.pop() {
        count += 1;
        pending.extend(error.nested());
    }
    count
}

/// Renders a message template by substituting `{name}` placeholders with
/// the matching entry from `params`. `{{` and `}}` are literal braces.
/// Unknown `{name}` is left as-is. Zero allocation when the template has
/// no `{` at all.
///
/// Crate-visible so `rule::Rule::validate` can eagerly render the user
/// template stored by a described rule against the inner error's params.
pub(crate) fn render_template<'a>(
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
            out.push('}');
            if matches!(chars.peek(), Some((_, '}'))) {
                chars.next();
            }
        } else {
            out.push(c);
        }
    }
    Cow::Owned(out)
}

impl ValidationError {
    /// Renders the message template against this error's params.
    ///
    /// This is the substitution [`Display`](fmt::Display) performs, exposed so
    /// a caller that needs the rendered text without formatter overhead — to
    /// attach it to a log record or store it back into the error — can obtain
    /// it directly. Returns [`Cow::Borrowed`] when the message has no `{`
    /// placeholders, so the common case does not allocate.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_validator::foundation::ValidationError;
    ///
    /// let error = ValidationError::new("min_length", "Must be at least {min} characters")
    ///     .with_param("min", "3");
    /// assert_eq!(error.rendered_message(), "Must be at least 3 characters");
    ///
    /// let plain = ValidationError::new("required", "This field is required");
    /// assert_eq!(plain.rendered_message(), "This field is required");
    /// ```
    pub fn rendered_message(&self) -> Cow<'_, str> {
        render_template(self.message.as_ref(), self.params())
    }
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

        // The (params: [...]) debug tail is intentionally removed: templates
        // now consume params in `rendered`, so re-listing them here would be
        // redundant and bypass the caller's intended message surface.
        // Raw params remain accessible via `params()` and `to_json_value()`.

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

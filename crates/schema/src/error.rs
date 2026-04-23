//! Unified structured error type for schema build, lint, validation, and resolution.

use std::{borrow::Cow, fmt, sync::Arc};

use serde_json::Value;

use crate::path::FieldPath;

/// Severity of a single issue.
#[non_exhaustive]
#[derive(Copy, Clone, PartialEq, Eq, Debug, Hash)]
pub enum Severity {
    /// A hard error that must be resolved.
    Error,
    /// A non-fatal advisory warning.
    Warning,
}

/// A single structured validation or schema issue.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub struct ValidationError {
    /// Machine-readable issue code (e.g. `"length.max"`, `"required"`).
    pub code: Cow<'static, str>,
    /// Path within the schema where the issue was observed.
    pub path: FieldPath,
    /// Severity level of this issue.
    pub severity: Severity,
    /// Structured key-value parameters for the issue (e.g. `max`, `actual`).
    pub params: Arc<[(Cow<'static, str>, Value)]>,
    /// Human-readable message describing the issue.
    pub message: Cow<'static, str>,
    /// Optional underlying cause.
    pub source: Option<Arc<dyn std::error::Error + Send + Sync>>,
}

impl ValidationError {
    /// Begin building a new `ValidationError` with the given machine-readable code.
    ///
    /// Returns a [`ValidationErrorBuilder`] — call `.build()` to finalise.
    ///
    /// # Example
    ///
    /// ```rust
    /// use nebula_schema::{FieldPath, ValidationError};
    ///
    /// let err = ValidationError::builder("required")
    ///     .at(FieldPath::parse("user.email").unwrap())
    ///     .message("field is required")
    ///     .build();
    ///
    /// assert_eq!(err.code, "required");
    /// ```
    pub fn builder(code: impl Into<Cow<'static, str>>) -> ValidationErrorBuilder {
        let code = code.into();
        // Default message = code so that build() without .message() produces
        // "[code]: code" rather than "[code]: " (empty message looks broken).
        let default_message = code.clone();
        ValidationErrorBuilder {
            code,
            path: FieldPath::root(),
            severity: Severity::Error,
            params: Vec::new(),
            message: default_message,
            source: None,
        }
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.path.is_root() {
            write!(f, "[{}]: {}", self.code, self.message)
        } else {
            write!(f, "[{}] at {}: {}", self.code, self.path, self.message)
        }
    }
}

impl std::error::Error for ValidationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_deref()
            .map(|e| e as &(dyn std::error::Error + 'static))
    }
}

/// Builder for [`ValidationError`].
#[must_use = "call .build() to produce a ValidationError"]
#[derive(Debug)]
pub struct ValidationErrorBuilder {
    code: Cow<'static, str>,
    path: FieldPath,
    severity: Severity,
    params: Vec<(Cow<'static, str>, Value)>,
    message: Cow<'static, str>,
    source: Option<Arc<dyn std::error::Error + Send + Sync>>,
}

impl ValidationErrorBuilder {
    /// Set the path at which the error occurred.
    #[must_use = "builder methods must be chained or built"]
    pub fn at(mut self, path: FieldPath) -> Self {
        self.path = path;
        self
    }
    /// Downgrade severity to [`Severity::Warning`].
    #[must_use = "builder methods must be chained or built"]
    pub const fn warn(mut self) -> Self {
        self.severity = Severity::Warning;
        self
    }
    /// Set the human-readable message.
    #[must_use = "builder methods must be chained or built"]
    pub fn message(mut self, msg: impl Into<Cow<'static, str>>) -> Self {
        self.message = msg.into();
        self
    }
    /// Attach a named parameter to the error.
    #[must_use = "builder methods must be chained or built"]
    pub fn param(mut self, key: &'static str, value: impl Into<Value>) -> Self {
        self.params.push((Cow::Borrowed(key), value.into()));
        self
    }
    /// Attach an underlying error cause.
    #[must_use = "builder methods must be chained or built"]
    pub fn source<E>(mut self, err: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        self.source = Some(Arc::new(err));
        self
    }
    /// Finalise and return the [`ValidationError`].
    #[must_use]
    pub fn build(self) -> ValidationError {
        ValidationError {
            code: self.code,
            path: self.path,
            severity: self.severity,
            params: self.params.into(),
            message: self.message,
            source: self.source,
        }
    }
}

/// Accumulates [`ValidationError`] issues produced during schema build, lint, or validation.
#[derive(Clone, Debug, Default)]
pub struct ValidationReport {
    issues: Vec<ValidationError>,
}

impl ValidationReport {
    /// Create an empty report.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    /// Append an issue to the report.
    pub fn push(&mut self, issue: ValidationError) {
        self.issues.push(issue);
    }
    /// Iterate over hard errors only.
    pub fn errors(&self) -> impl Iterator<Item = &ValidationError> {
        self.issues.iter().filter(|i| i.severity == Severity::Error)
    }
    /// Iterate over warnings only.
    pub fn warnings(&self) -> impl Iterator<Item = &ValidationError> {
        self.issues
            .iter()
            .filter(|i| i.severity == Severity::Warning)
    }
    /// Iterate over all issues regardless of severity.
    pub fn iter(&self) -> impl Iterator<Item = &ValidationError> {
        self.issues.iter()
    }
    /// Iterate over issues whose path starts with `prefix`.
    pub fn at_path(&self, prefix: &FieldPath) -> impl Iterator<Item = &ValidationError> {
        let prefix = prefix.clone();
        self.issues
            .iter()
            .filter(move |i| i.path.starts_with(&prefix))
    }
    /// Returns `true` if the report contains at least one hard error.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.errors().next().is_some()
    }
    /// Returns `true` if the report contains at least one warning.
    #[must_use]
    pub fn has_warnings(&self) -> bool {
        self.warnings().next().is_some()
    }
    /// Total number of issues (errors + warnings).
    #[must_use]
    pub const fn len(&self) -> usize {
        self.issues.len()
    }
    /// Returns `true` when the report has no issues.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
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

impl fmt::Display for ValidationReport {
    /// Formats the report as a newline-separated list of issues.
    ///
    /// Each line is the `Display` of the individual [`ValidationError`].
    /// An empty report formats as `"(no issues)"`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use nebula_schema::{FieldPath, ValidationError, ValidationReport};
    ///
    /// let mut report = ValidationReport::new();
    /// report.push(
    ///     ValidationError::builder("required")
    ///         .at(FieldPath::parse("name").unwrap())
    ///         .message("field is required")
    ///         .build(),
    /// );
    ///
    /// let text = report.to_string();
    /// assert!(text.contains("[required]"));
    /// assert!(text.contains("name"));
    /// ```
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

/// Canonical set of stable error codes emitted by the schema crate.
///
/// Plugins may add their own under a namespace prefix (e.g. `my_plugin.foo`).
/// A test in the schema crate guarantees every entry here is emittable from
/// an integration test (see `tests/flow/all_error_codes.rs`).
pub const STANDARD_CODES: &[&str] = &[
    // value validation
    "required",
    "type_mismatch",
    "length.min",
    "length.max",
    "range.min",
    "range.max",
    "pattern",
    "url",
    "email",
    "items.min",
    "items.max",
    "items.unique",
    "option.invalid",
    // mode
    "mode.required",
    "mode.invalid",
    // expression
    "expression.forbidden",
    "expression.required",
    "expression.parse",
    "expression.type_mismatch",
    "expression.runtime",
    // loader
    "loader.not_registered",
    "loader.missing_config",
    "loader.failed",
    // field resolution
    "field.not_found",
    "field.type_mismatch",
    // default value type mismatch
    "default.type_mismatch",
    // build-time
    "invalid_key",
    "duplicate_key",
    "dangling_reference",
    "self_dependency",
    "visibility_cycle",
    "required_cycle",
    "loader_dependency_cycle",
    "rule.contradictory",
    "missing_item_schema",
    "invalid_default_variant",
    "duplicate_variant",
    "schema.index_overflow",
    "schema.depth_limit",
    // option lint
    "option.type_inconsistent",
    // warnings
    "rule.incompatible",
    "notice.misuse",
    "missing_loader",
    "loader_without_dynamic",
    "duplicate_dependency",
    "missing_variant_label",
    "notice_missing_description",
];

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn builder_produces_full_error() {
        let path = FieldPath::parse("user.email").unwrap();
        let err = ValidationError::builder("length.max")
            .at(path.clone())
            .message("value too long")
            .param("max", json!(20))
            .param("actual", json!(42))
            .build();

        assert_eq!(err.code, "length.max");
        assert_eq!(err.path, path);
        assert_eq!(err.severity, Severity::Error);
        assert_eq!(err.params.len(), 2);
        assert!(err.message.contains("too long"));
    }

    #[test]
    fn warn_lowers_severity() {
        let err = ValidationError::builder("notice.misuse").warn().build();
        assert_eq!(err.severity, Severity::Warning);
    }

    #[test]
    fn display_format_is_stable() {
        let err = ValidationError::builder("required")
            .at(FieldPath::parse("x").unwrap())
            .message("missing")
            .build();
        assert_eq!(format!("{err}"), "[required] at x: missing");
    }

    #[test]
    fn display_omits_at_for_root_path() {
        let err = ValidationError::builder("required")
            .message("missing")
            .build();
        assert_eq!(format!("{err}"), "[required]: missing");
    }

    #[test]
    fn report_splits_errors_and_warnings() {
        let mut report = ValidationReport::new();
        report.push(ValidationError::builder("required").build());
        report.push(ValidationError::builder("notice.misuse").warn().build());
        assert!(report.has_errors());
        assert!(report.has_warnings());
        assert_eq!(report.errors().count(), 1);
        assert_eq!(report.warnings().count(), 1);
    }

    #[test]
    fn report_display_empty() {
        let report = ValidationReport::new();
        assert_eq!(report.to_string(), "(no issues)");
    }

    #[test]
    fn report_display_single_error() {
        let mut report = ValidationReport::new();
        report.push(
            ValidationError::builder("required")
                .at(FieldPath::parse("name").unwrap())
                .message("field is required")
                .build(),
        );
        let text = report.to_string();
        assert!(text.contains("[required]"), "code missing: {text}");
        assert!(text.contains("name"), "path missing: {text}");
        assert!(
            text.contains("field is required"),
            "message missing: {text}"
        );
    }

    #[test]
    fn report_display_multiple_issues_newline_separated() {
        let mut report = ValidationReport::new();
        report.push(
            ValidationError::builder("required")
                .at(FieldPath::parse("a").unwrap())
                .message("missing a")
                .build(),
        );
        report.push(
            ValidationError::builder("type_mismatch")
                .at(FieldPath::parse("b").unwrap())
                .message("wrong type for b")
                .build(),
        );
        let text = report.to_string();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "expected 2 lines, got: {text:?}");
        assert!(lines[0].contains("[required]"));
        assert!(lines[1].contains("[type_mismatch]"));
    }

    #[test]
    fn standard_codes_are_unique_and_nonempty() {
        assert!(!STANDARD_CODES.is_empty());
        let mut sorted: Vec<&str> = STANDARD_CODES.to_vec();
        sorted.sort_unstable();
        let before = sorted.len();
        sorted.dedup();
        assert_eq!(before, sorted.len(), "duplicate code in STANDARD_CODES");
        for code in STANDARD_CODES {
            assert!(!code.is_empty());
            assert!(
                code.chars()
                    .all(|c| c.is_ascii_lowercase() || c == '_' || c == '.')
            );
        }
    }
}

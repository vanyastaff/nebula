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
    #[allow(clippy::new_ret_no_self)]
    pub fn new(code: impl Into<Cow<'static, str>>) -> ValidationErrorBuilder {
        ValidationErrorBuilder {
            code: code.into(),
            path: FieldPath::root(),
            severity: Severity::Error,
            params: Vec::new(),
            message: Cow::Borrowed(""),
            source: None,
        }
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] at {}: {}", self.code, self.path, self.message)
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
    pub fn at(mut self, path: FieldPath) -> Self {
        self.path = path;
        self
    }
    /// Downgrade severity to [`Severity::Warning`].
    pub fn warn(mut self) -> Self {
        self.severity = Severity::Warning;
        self
    }
    /// Set the human-readable message.
    pub fn message(mut self, msg: impl Into<Cow<'static, str>>) -> Self {
        self.message = msg.into();
        self
    }
    /// Attach a named parameter to the error.
    pub fn param(mut self, key: &'static str, value: impl Into<Value>) -> Self {
        self.params.push((Cow::Borrowed(key), value.into()));
        self
    }
    /// Attach an underlying error cause.
    pub fn source<E>(mut self, err: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        self.source = Some(Arc::new(err));
        self
    }
    /// Finalise and return the [`ValidationError`].
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
    pub fn has_errors(&self) -> bool {
        self.errors().next().is_some()
    }
    /// Returns `true` if the report contains at least one warning.
    pub fn has_warnings(&self) -> bool {
        self.warnings().next().is_some()
    }
    /// Total number of issues (errors + warnings).
    pub fn len(&self) -> usize {
        self.issues.len()
    }
    /// Returns `true` when the report has no issues.
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

// ---------------------------------------------------------------------------
// Legacy shim — kept to avoid breaking key.rs and schema.rs before Task 5/6
// rewrite them. Do not use in new code.
// ---------------------------------------------------------------------------

/// Legacy error enum.
///
/// Kept temporarily so `key.rs` and `schema.rs` continue to compile while
/// Tasks 5 and 6 rewrite those callers. Will be deleted in a later task.
#[doc(hidden)]
#[allow(dead_code)]
#[derive(Debug)]
pub enum SchemaError {
    /// Field key violates format rules.
    InvalidKey(String),
    /// Duplicate key detected in a single schema.
    DuplicateKey(String),
    /// Referenced field does not exist in schema.
    FieldNotFound(String),
    /// Field exists but has an unexpected type for the requested operation.
    InvalidFieldType {
        /// The field key.
        key: String,
        /// Expected type name.
        expected: &'static str,
        /// Actual type name.
        actual: &'static str,
    },
    /// Field is dynamic but no loader key was configured.
    LoaderNotConfigured(String),
    /// Rule validation failure from the validator crate.
    Validation(nebula_validator::ValidatorError),
    /// Runtime loader invocation failed.
    Loader(crate::loader::LoaderError),
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidKey(msg) => write!(f, "invalid field key: {msg}"),
            Self::DuplicateKey(msg) => write!(f, "duplicate field key: {msg}"),
            Self::FieldNotFound(msg) => write!(f, "field not found: {msg}"),
            Self::InvalidFieldType {
                key,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "field `{key}` has invalid type: expected {expected}, got {actual}"
                )
            },
            Self::LoaderNotConfigured(msg) => write!(f, "field `{msg}` has no loader configured"),
            Self::Validation(e) => write!(f, "validation failed: {e}"),
            Self::Loader(e) => write!(f, "loader failed: {e}"),
        }
    }
}

impl std::error::Error for SchemaError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Validation(e) => Some(e),
            Self::Loader(e) => Some(e),
            _ => None,
        }
    }
}

impl From<nebula_validator::ValidatorError> for SchemaError {
    fn from(e: nebula_validator::ValidatorError) -> Self {
        Self::Validation(e)
    }
}

impl From<crate::loader::LoaderError> for SchemaError {
    fn from(e: crate::loader::LoaderError) -> Self {
        Self::Loader(e)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn builder_produces_full_error() {
        let path = FieldPath::parse("user.email").unwrap();
        let err = ValidationError::new("length.max")
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
        let err = ValidationError::new("notice.misuse").warn().build();
        assert_eq!(err.severity, Severity::Warning);
    }

    #[test]
    fn display_format_is_stable() {
        let err = ValidationError::new("required")
            .at(FieldPath::parse("x").unwrap())
            .message("missing")
            .build();
        assert_eq!(format!("{err}"), "[required] at x: missing");
    }

    #[test]
    fn report_splits_errors_and_warnings() {
        let mut report = ValidationReport::new();
        report.push(ValidationError::new("required").build());
        report.push(ValidationError::new("notice.misuse").warn().build());
        assert!(report.has_errors());
        assert!(report.has_warnings());
        assert_eq!(report.errors().count(), 1);
        assert_eq!(report.warnings().count(), 1);
    }
}

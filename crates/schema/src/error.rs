//! Unified structured error type for schema build, lint, validation, and resolution.

use std::{borrow::Cow, fmt, sync::Arc};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::path::FieldPath;

/// Severity of a single issue.
#[non_exhaustive]
#[derive(Copy, Clone, PartialEq, Eq, Debug, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// A hard error that must be resolved.
    Error,
    /// A non-fatal advisory warning.
    Warning,
}

impl<'de> Deserialize<'de> for Severity {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        // Forward-compatible AND fail-closed. `Severity` is `#[non_exhaustive]`,
        // so a newer writer may emit a value this version does not know. We do not
        // fail the parse — `ValidationReport` is `#[serde(transparent)]`, so a
        // derived "unknown variant" error would poison the *whole* report. But an
        // unknown severity is read as `Error`, NOT `Warning`: a future severity
        // could be *more* severe than `Error`, and the validation gate stops on
        // `has_errors()` — downgrading an unknown (possibly blocking) issue to an
        // advisory `Warning` would let the gate fail OPEN. Over-blocking on an
        // unrecognized severity is the safe direction; under-blocking is not.
        let raw = Cow::<str>::deserialize(d)?;
        Ok(match raw.as_ref() {
            "warning" => Self::Warning,
            _ => Self::Error,
        })
    }
}

/// A single structured validation or schema issue.
///
/// Serializable for wire transport (API responses, cross-process error
/// propagation): `code` is the stable machine-readable vocabulary
/// ([`STANDARD_CODES`]) and `path`/`severity`/`params`/`message` are the
/// client-facing payload. The internal `source` cause chain is **not** part of
/// the wire contract — it is `#[serde(skip)]` (a `dyn Error` has no serialized
/// form and is debug-only), so a deserialized error has `source: None`.
#[non_exhaustive]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ValidationError {
    /// Machine-readable issue code (e.g. `"required"`, `"type_mismatch"`,
    /// or a validator-native rule code such as `"max_length"`).
    pub code: Cow<'static, str>,
    /// Path within the schema where the issue was observed.
    pub path: FieldPath,
    /// Severity level of this issue.
    pub severity: Severity,
    /// Structured key-value parameters for the issue (e.g. `max`, `actual`).
    pub params: Arc<[(Cow<'static, str>, Value)]>,
    /// Human-readable message describing the issue.
    pub message: Cow<'static, str>,
    /// Optional underlying cause — debug-only, not serialized (a `dyn Error`
    /// has no wire form; a deserialized error always has `source: None`).
    #[serde(skip)]
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
    ///.at(FieldPath::parse("user.email").unwrap())
    ///.message("field is required")
    ///.build();
    ///
    /// assert_eq!(err.code, "required");
    /// ```
    pub fn builder(code: impl Into<Cow<'static, str>>) -> ValidationErrorBuilder {
        let code = code.into();
        // Default message = code so that build() without.message() produces
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
///
/// Serializes **transparently** as a flat JSON array of [`ValidationError`] — a
/// report *is* its list of issues — so a wire payload is `[{…}, {…}]`, not
/// `{"issues": […]}`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(transparent)]
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
    /// ValidationError::builder("required")
    ///.at(FieldPath::parse("name").unwrap())
    ///.message("field is required")
    ///.build(),
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

/// Canonical set of stable error codes observable from the schema crate.
///
/// Two provenances:
/// - **Schema-owned structural codes** — emitted by the schema crate itself
///   (`required`, `type_mismatch`, `items.*`, `option.*`, `mode.*`,
///   `expression.*`, and all build-time/lint codes). Stable.
/// - **Rule-failure codes** — produced by `nebula-validator` and surfaced
///   verbatim (no namespace remap). A failing length/range/format rule
///   reports the validator-native `min_length` / `max_length` / `min` /
///   `max` / `invalid_format`. This replaced the former schema-side
///   `length.*` / `range.*` / `pattern` / `url` / `email` remap.
///
/// Plugins may add their own under a namespace prefix (e.g. `my_plugin.foo`).
/// A test in the schema crate guarantees every entry here is emittable from
/// an integration test (see `tests/flow/all_error_codes.rs`).
pub const STANDARD_CODES: &[&str] = &[
    // value validation — schema-owned structural code
    "required",
    "type_mismatch",
    // rule failures — surfaced verbatim from nebula-validator
    "min_length",
    "max_length",
    "min",
    "max",
    "invalid_format",
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
    "recursion_limit",
    "secret.default_forbidden",
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

    #[test]
    fn validation_error_serde_round_trips() {
        let original = ValidationError::builder("length.max")
            .at(FieldPath::parse("user.email").unwrap())
            .warn()
            .message("value too long")
            .param("max", json!(20))
            .param("actual", json!(42))
            .build();
        let wire = serde_json::to_value(&original).expect("serialize");
        let restored: ValidationError = serde_json::from_value(wire).expect("deserialize");

        assert_eq!(restored.code, original.code);
        assert_eq!(restored.path, original.path);
        assert_eq!(restored.severity, original.severity);
        assert_eq!(restored.message, original.message);
        assert_eq!(restored.params.as_ref(), original.params.as_ref());
        assert!(restored.source.is_none());
    }

    #[test]
    fn source_is_not_serialized() {
        // The internal cause chain is debug-only and has no wire form.
        let cause = "nan".parse::<i32>().unwrap_err();
        let err = ValidationError::builder("type_mismatch")
            .source(cause)
            .build();
        assert!(err.source.is_some(), "source is attached in memory");

        let wire = serde_json::to_value(&err).expect("serialize");
        assert!(
            wire.get("source").is_none(),
            "source must not appear on the wire, got: {wire}"
        );
        let restored: ValidationError = serde_json::from_value(wire).expect("deserialize");
        assert!(
            restored.source.is_none(),
            "source drops to None across the wire"
        );
    }

    #[test]
    fn report_serializes_as_flat_array() {
        let mut report = ValidationReport::new();
        report.push(
            ValidationError::builder("required")
                .at(FieldPath::parse("a").unwrap())
                .build(),
        );
        report.push(ValidationError::builder("type_mismatch").warn().build());

        let wire = serde_json::to_value(&report).expect("serialize");
        assert!(
            wire.is_array(),
            "report serializes transparently as an array"
        );
        assert_eq!(wire.as_array().map(Vec::len), Some(2));

        let restored: ValidationReport = serde_json::from_value(wire).expect("deserialize");
        assert_eq!(restored.len(), 2);
        assert!(restored.has_errors() && restored.has_warnings());
    }

    #[test]
    fn severity_serializes_snake_case() {
        assert_eq!(
            serde_json::to_value(Severity::Error).unwrap(),
            json!("error")
        );
        assert_eq!(
            serde_json::to_value(Severity::Warning).unwrap(),
            json!("warning")
        );
        assert_eq!(
            serde_json::from_value::<Severity>(json!("warning")).unwrap(),
            Severity::Warning
        );
    }

    #[test]
    fn unknown_severity_deserializes_as_error_fail_closed() {
        // Forward compat + fail-closed: a severity a newer writer emits that this
        // version does not know reads as Error (never a parse failure, never a
        // silent downgrade), while a known `warning` still reads as Warning.
        assert_eq!(
            serde_json::from_value::<Severity>(json!("critical")).unwrap(),
            Severity::Error
        );
        assert_eq!(
            serde_json::from_value::<Severity>(json!("warning")).unwrap(),
            Severity::Warning
        );

        // A transparent report with an unknown severity still parses; the unknown
        // entry surfaces as a hard error so a downstream gate fails closed.
        let wire = json!([
            {"code": "notice.x", "path": "", "severity": "warning", "params": [], "message": "w"},
            {"code": "future", "path": "a", "severity": "critical", "params": [], "message": "y"},
        ]);
        let report: ValidationReport = serde_json::from_value(wire).expect("report parses");
        assert_eq!(report.len(), 2);
        assert!(
            report.has_errors(),
            "the unknown severity fails closed as an error"
        );
        assert!(report.has_warnings(), "the known warning is preserved");
    }
}

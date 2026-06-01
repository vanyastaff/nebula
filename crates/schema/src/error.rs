//! Structured error codes for schema build, lint, validation, and resolution.
//!
//! The structured error *types* (`ValidationError`, `ValidationErrorBuilder`,
//! `ValidationReport`, `ErrorSeverity`) are the canonical ones defined in
//! [`nebula_error`] and re-exported here so schema code keeps a single import
//! path. The schema crate owns only the stable set of error *codes*
//! ([`STANDARD_CODES`]).

pub use nebula_error::{
    ErrorSeverity, FieldPath as ErrorFieldPath, ValidationError, ValidationErrorBuilder,
    ValidationReport,
};

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
    use super::*;

    #[test]
    fn builder_produces_full_error() {
        let err = ValidationError::builder("max_length")
            .at_field("user.email")
            .message("value too long")
            .param("max", "20")
            .param("actual", "42")
            .build();

        assert_eq!(err.code, "max_length");
        assert_eq!(err.field.as_deref(), Some("/user/email"));
        assert_eq!(err.severity(), ErrorSeverity::Error);
        assert_eq!(err.params().len(), 2);
        assert!(err.message.contains("too long"));
    }

    #[test]
    fn warn_lowers_severity() {
        let err = ValidationError::builder("notice.misuse").warn().build();
        assert_eq!(err.severity(), ErrorSeverity::Warning);
    }

    #[test]
    fn display_format_is_stable() {
        let err = ValidationError::builder("required")
            .at_field("x")
            .message("missing")
            .build();
        assert_eq!(format!("{err}"), "[/x] required: missing");
    }

    #[test]
    fn display_omits_pointer_for_root() {
        let err = ValidationError::builder("required")
            .message("missing")
            .build();
        assert_eq!(format!("{err}"), "required: missing");
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
                .at_field("name")
                .message("field is required")
                .build(),
        );
        let text = report.to_string();
        assert!(text.contains("required"), "code missing: {text}");
        assert!(text.contains("/name"), "path missing: {text}");
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
                .at_field("a")
                .message("missing a")
                .build(),
        );
        report.push(
            ValidationError::builder("type_mismatch")
                .at_field("b")
                .message("wrong type for b")
                .build(),
        );
        let text = report.to_string();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "expected 2 lines, got: {text:?}");
        assert!(lines[0].contains("required"));
        assert!(lines[1].contains("type_mismatch"));
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

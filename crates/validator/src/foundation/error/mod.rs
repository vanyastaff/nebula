//! Error types for validation failures.
//!
//! The structured error types ([`ValidationError`], [`ValidationErrors`],
//! [`ErrorSeverity`], `FieldPath`, `render_template`) are the canonical
//! workspace types defined in `nebula-error` and re-exported here so existing
//! `crate::foundation::error::*` paths keep working. Only the validator-local
//! [`ValidationMode`] and the canonical [`codes`] live in this crate.

pub mod codes;
mod mode;

pub use mode::ValidationMode;
pub(crate) use nebula_error::render_template;
pub use nebula_error::{ErrorSeverity, ValidationError, ValidationErrors};

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::*;

    #[test]
    fn test_validation_error_size() {
        // Ensure the canonical struct is <= 80 bytes
        let size = size_of::<ValidationError>();
        assert!(
            size <= 80,
            "ValidationError size is {size} bytes, expected <= 80"
        );
    }

    #[test]
    fn test_simple_error_fields() {
        let error = ValidationError::new("test", "Test error");
        assert_eq!(error.code, "test");
        assert_eq!(error.message, "Test error");
    }

    #[test]
    fn test_error_with_field() {
        let error = ValidationError::new("required", "Field is required").with_field("email");
        assert_eq!(error.field.as_deref(), Some("/email"));
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
    fn test_dot_path_is_normalized_to_pointer() {
        let error = ValidationError::new("test", "Test").with_field("service.port");
        assert_eq!(error.field_pointer().as_deref(), Some("/service/port"));
    }

    #[test]
    fn test_bracket_path_is_normalized_to_pointer() {
        let error = ValidationError::new("test", "Test").with_field("items[0].name");
        assert_eq!(error.field_pointer().as_deref(), Some("/items/0/name"));
    }

    #[test]
    fn test_pointer_fragment_is_normalized() {
        let error = ValidationError::new("test", "Test").with_pointer("#/user/email");
        assert_eq!(error.field.as_deref(), Some("/user/email"));
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
    fn test_unclosed_bracket_preserves_content() {
        // Unclosed bracket should not silently drop the index content
        let error = ValidationError::new("test", "Test").with_field("items[0");
        // "items" becomes first segment, "[0" becomes second (bracket preserved as literal)
        assert_eq!(error.field.as_deref(), Some("/items/[0"));
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

    #[test]
    fn template_substitutes_named_placeholder() {
        let err = ValidationError::new("min_length", "got {value}, expected at least {min} chars")
            .with_param("min", "3")
            .with_param("value", "\"hi\"");
        let rendered = format!("{err}");
        assert!(rendered.contains("got \"hi\", expected at least 3 chars"));
    }

    #[test]
    fn template_leaves_unknown_placeholder_literal() {
        let err = ValidationError::new("test", "value is {unknown}");
        let rendered = format!("{err}");
        assert!(rendered.contains("value is {unknown}"));
    }

    #[test]
    fn template_escape_double_brace() {
        let err = ValidationError::new("test", "literal {{ and {{value}}");
        let rendered = format!("{err}");
        assert!(rendered.contains("literal { and {value}"));
    }

    #[test]
    fn plain_message_bypasses_template_path() {
        let err = ValidationError::new("test", "no placeholders here");
        let rendered = format!("{err}");
        assert!(rendered.contains("no placeholders here"));
    }

    #[test]
    fn render_template_helper_is_reexported() {
        let params = [(Cow::Borrowed("min"), Cow::Borrowed("3"))];
        let rendered = render_template("at least {min}", &params);
        assert_eq!(rendered, "at least 3");
    }

    #[test]
    fn display_does_not_leak_params_tail() {
        // Templates consume params in the rendered message, so the debug
        // `(params: [...])` tail was removed from Display to avoid double
        // exposure (and accidental info disclosure for non-redacted keys).
        let err = ValidationError::new("test", "value is {secret}")
            .with_param("secret", "shh")
            .with_param("other", "leaked");
        let rendered = format!("{err}");
        assert!(
            !rendered.contains("(params:"),
            "Display should not re-list params after template rendering: {rendered}"
        );
        assert!(
            !rendered.contains("other=leaked"),
            "param values not referenced by template must not appear in Display: {rendered}"
        );
    }
}

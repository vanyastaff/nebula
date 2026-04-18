//! Value-validation rules — operate on a single JSON value, no context.
//!
//! **Strict on JSON type mismatch.** A value rule bound to a JSON kind
//! (`MinLength` → string, `Min` → number, `MinItems` → array, …) returns
//! `ValidationError::type_mismatch` when the input's JSON kind does not
//! match. `null` is a distinct kind and does not match any typed rule —
//! schema layers are expected to filter optional/nullable fields upstream
//! before dispatching rules (see `nebula-schema::validated`). Strictness
//! here aligns with PRODUCT_CANON §4.2 ("no silent shape mismatches").

use serde::{Deserialize, Serialize};

use super::helpers::{compile_regex, format_json_number, json_number_cmp};
use crate::{
    foundation::{Validate, ValidationError},
    validators::{
        content::{EMAIL_PATTERN, URL_PATTERN},
        max_length, max_size, min_length, min_size,
    },
};

/// Value-validation rule. Takes a JSON value, returns `Ok` or a
/// `ValidationError` whose `params` include rule-specific placeholders
/// (`{min}`, `{max}`, `{pattern}`, `{allowed}`) for template rendering.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ValueRule {
    /// String must be at least `n` characters.
    MinLength(usize),
    /// String must be at most `n` characters.
    MaxLength(usize),
    /// String must match the regex.
    Pattern(String),
    /// Number must be >= bound.
    Min(serde_json::Number),
    /// Number must be <= bound.
    Max(serde_json::Number),
    /// Number must be strictly > bound.
    GreaterThan(serde_json::Number),
    /// Number must be strictly < bound.
    LessThan(serde_json::Number),
    /// Value must be one of the given alternatives (type-matched).
    OneOf(Vec<serde_json::Value>),
    /// Collection must contain at least `n` items.
    MinItems(usize),
    /// Collection must contain at most `n` items.
    MaxItems(usize),
    /// Value must be a valid email address.
    Email,
    /// Value must be a valid URL.
    Url,
}

/// JSON kind name for error reporting — stable across rustc versions.
fn json_type_name(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// Builds a `type_mismatch` error with the rule-wide `{value}` param so
/// templates can still render the offending input.
fn type_mismatch(value: &serde_json::Value, expected: &'static str) -> ValidationError {
    ValidationError::type_mismatch("", expected, json_type_name(value))
        .with_param("value", format!("{value}"))
}

impl ValueRule {
    /// Validates a JSON value against this rule.
    ///
    /// Kind-bound rules (`MinLength`, `MaxLength`, `Pattern`, `Email`, `Url`
    /// → string; `Min`, `Max`, `GreaterThan`, `LessThan` → number;
    /// `MinItems`, `MaxItems` → array) return `ValidationError::type_mismatch`
    /// (code `type_mismatch`, params `expected` / `actual` / `value`) when
    /// the input's JSON kind does not match. Callers that want tolerant
    /// behaviour on mismatched kinds should check the shape upstream — see
    /// `nebula-schema::validated` for the canonical pattern.
    ///
    /// `OneOf` is kind-agnostic: a value that is not in the allowed set
    /// reports `one_of` regardless of kind (mismatched kind is just a
    /// non-member). An empty allowed set passes.
    ///
    /// Rule-specific validation failures carry `params` for
    /// message-template rendering: `{min}`, `{max}`, `{pattern}`,
    /// `{allowed}`, plus always `{value}`.
    ///
    /// Exception: when `Pattern` holds a malformed regex, this returns a
    /// compile-time error with code `invalid_pattern` — no `{value}` param,
    /// because the rule is mis-configured independently of the input value.
    pub fn validate_value(&self, value: &serde_json::Value) -> Result<(), ValidationError> {
        match self {
            Self::MinLength(n) => {
                let s = value
                    .as_str()
                    .ok_or_else(|| type_mismatch(value, "string"))?;
                min_length(*n).validate(s).map_err(|e| {
                    e.with_param("min", n.to_string())
                        .with_param("value", format!("{value}"))
                })
            },
            Self::MaxLength(n) => {
                let s = value
                    .as_str()
                    .ok_or_else(|| type_mismatch(value, "string"))?;
                max_length(*n).validate(s).map_err(|e| {
                    e.with_param("max", n.to_string())
                        .with_param("value", format!("{value}"))
                })
            },
            Self::Pattern(p) => {
                let s = value
                    .as_str()
                    .ok_or_else(|| type_mismatch(value, "string"))?;
                let re = compile_regex(p)?;
                if !re.is_match(s) {
                    return Err(ValidationError::invalid_format("", "regex")
                        .with_param("pattern", p.clone())
                        .with_param("value", format!("{value}")));
                }
                Ok(())
            },
            Self::Min(bound) => {
                let ord =
                    json_number_cmp(value, bound).ok_or_else(|| type_mismatch(value, "number"))?;
                if ord.is_lt() {
                    return Err(ValidationError::new("min", "Value must be at least {min}")
                        .with_param("min", format_json_number(bound))
                        .with_param("value", format!("{value}")));
                }
                Ok(())
            },
            Self::Max(bound) => {
                let ord =
                    json_number_cmp(value, bound).ok_or_else(|| type_mismatch(value, "number"))?;
                if ord.is_gt() {
                    return Err(ValidationError::new("max", "Value must be at most {max}")
                        .with_param("max", format_json_number(bound))
                        .with_param("value", format!("{value}")));
                }
                Ok(())
            },
            Self::GreaterThan(bound) => {
                let ord =
                    json_number_cmp(value, bound).ok_or_else(|| type_mismatch(value, "number"))?;
                if !ord.is_gt() {
                    return Err(ValidationError::new(
                        "greater_than",
                        "Value must be greater than {min}",
                    )
                    .with_param("min", format_json_number(bound))
                    .with_param("value", format!("{value}")));
                }
                Ok(())
            },
            Self::LessThan(bound) => {
                let ord =
                    json_number_cmp(value, bound).ok_or_else(|| type_mismatch(value, "number"))?;
                if !ord.is_lt() {
                    return Err(
                        ValidationError::new("less_than", "Value must be less than {max}")
                            .with_param("max", format_json_number(bound))
                            .with_param("value", format!("{value}")),
                    );
                }
                Ok(())
            },
            // OneOf is kind-agnostic: the allowed set defines both the
            // accepted kinds and the accepted values, so a mismatched kind
            // is just "not one of the allowed values" — no need for a
            // separate type_mismatch path.
            Self::OneOf(values) => {
                if values.is_empty() {
                    return Ok(());
                }
                if values.contains(value) {
                    return Ok(());
                }
                let allowed = values
                    .iter()
                    .map(|v| format!("{v}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                Err(
                    ValidationError::new("one_of", "Value must be one of {allowed}")
                        .with_param("allowed", allowed)
                        .with_param("value", format!("{value}")),
                )
            },
            Self::MinItems(n) => {
                let items = value
                    .as_array()
                    .ok_or_else(|| type_mismatch(value, "array"))?;
                min_size::<serde_json::Value>(*n)
                    .validate(items.as_slice())
                    .map_err(|e| {
                        e.with_param("min", n.to_string())
                            .with_param("value", format!("{value}"))
                    })
            },
            Self::MaxItems(n) => {
                let items = value
                    .as_array()
                    .ok_or_else(|| type_mismatch(value, "array"))?;
                max_size::<serde_json::Value>(*n)
                    .validate(items.as_slice())
                    .map_err(|e| {
                        e.with_param("max", n.to_string())
                            .with_param("value", format!("{value}"))
                    })
            },
            Self::Email => {
                let s = value
                    .as_str()
                    .ok_or_else(|| type_mismatch(value, "string"))?;
                static EMAIL_RE: std::sync::LazyLock<regex::Regex> =
                    std::sync::LazyLock::new(|| {
                        regex::Regex::new(EMAIL_PATTERN).expect("email regex")
                    });
                if !EMAIL_RE.is_match(s) {
                    return Err(ValidationError::invalid_format("", "email")
                        .with_param("value", format!("{value}")));
                }
                Ok(())
            },
            Self::Url => {
                let s = value
                    .as_str()
                    .ok_or_else(|| type_mismatch(value, "string"))?;
                static URL_RE: std::sync::LazyLock<regex::Regex> =
                    std::sync::LazyLock::new(|| regex::Regex::new(URL_PATTERN).expect("url regex"));
                if !URL_RE.is_match(s) {
                    return Err(ValidationError::invalid_format("", "url")
                        .with_param("value", format!("{value}")));
                }
                Ok(())
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn min_length_ok_and_err() {
        assert!(
            ValueRule::MinLength(3)
                .validate_value(&json!("alice"))
                .is_ok()
        );
        assert!(
            ValueRule::MinLength(3)
                .validate_value(&json!("ab"))
                .is_err()
        );
    }

    #[test]
    fn min_length_rejects_non_string_with_type_mismatch() {
        let err = ValueRule::MinLength(3)
            .validate_value(&json!(42))
            .unwrap_err();
        assert_eq!(err.code.as_ref(), "type_mismatch");
        assert_eq!(err.param("expected"), Some("string"));
        assert_eq!(err.param("actual"), Some("number"));
    }

    #[test]
    fn min_rejects_below_bound() {
        let rule = ValueRule::Min(serde_json::Number::from(10));
        assert!(rule.validate_value(&json!(5)).is_err());
        assert!(rule.validate_value(&json!(15)).is_ok());
    }

    #[test]
    fn min_rejects_non_number_with_type_mismatch() {
        let err = ValueRule::Min(serde_json::Number::from(10))
            .validate_value(&json!("hi"))
            .unwrap_err();
        assert_eq!(err.code.as_ref(), "type_mismatch");
        assert_eq!(err.param("expected"), Some("number"));
        assert_eq!(err.param("actual"), Some("string"));
    }

    #[test]
    fn min_items_rejects_non_array_with_type_mismatch() {
        let err = ValueRule::MinItems(1)
            .validate_value(&json!("not-an-array"))
            .unwrap_err();
        assert_eq!(err.code.as_ref(), "type_mismatch");
        assert_eq!(err.param("expected"), Some("array"));
    }

    #[test]
    fn email_rejects_non_string_with_type_mismatch() {
        let err = ValueRule::Email.validate_value(&json!(42)).unwrap_err();
        assert_eq!(err.code.as_ref(), "type_mismatch");
        assert_eq!(err.param("expected"), Some("string"));
    }

    #[test]
    fn one_of_rejects_wrong_type_instead_of_silent_pass() {
        // Issue #264: OneOf(["a","b"]).validate(42) must not silently pass.
        let rule = ValueRule::OneOf(vec![json!("a"), json!("b")]);
        let err = rule.validate_value(&json!(42)).unwrap_err();
        assert_eq!(err.code.as_ref(), "one_of");
    }

    #[test]
    fn one_of_empty_passes() {
        assert!(ValueRule::OneOf(vec![]).validate_value(&json!("x")).is_ok());
    }

    #[test]
    fn null_rejected_as_type_mismatch() {
        let err = ValueRule::MinLength(3)
            .validate_value(&json!(null))
            .unwrap_err();
        assert_eq!(err.code.as_ref(), "type_mismatch");
        assert_eq!(err.param("actual"), Some("null"));
    }

    #[test]
    fn wire_form_scalar_is_tuple() {
        let r = ValueRule::MinLength(3);
        let j = serde_json::to_value(&r).unwrap();
        assert_eq!(j, json!({"min_length": 3}));
    }

    #[test]
    fn wire_form_unit_is_bare_string() {
        let r = ValueRule::Email;
        let j = serde_json::to_value(&r).unwrap();
        assert_eq!(j, json!("email"));
    }

    #[test]
    fn error_injects_params_for_template_rendering() {
        let err = ValueRule::MinLength(3)
            .validate_value(&json!("hi"))
            .unwrap_err();
        assert_eq!(err.param("min"), Some("3"));
    }
}

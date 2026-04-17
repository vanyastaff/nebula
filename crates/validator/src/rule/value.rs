//! Value-validation rules — operate on a single JSON value, no context.
//!
//! Silent-pass on JSON type mismatch (e.g. `MinLength` on a number
//! returns `Ok`) is preserved as documented ergonomic. Cross-kind
//! silent-pass (predicate returning `Ok` from `validate_value`) is
//! eliminated by the type split.

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

impl ValueRule {
    /// Validates a JSON value against this rule. Returns `Ok(())` silently
    /// when the JSON type doesn't match the rule's expected type.
    ///
    /// Errors carry rule-specific `params` for message-template rendering:
    /// `{min}`, `{max}`, `{pattern}`, `{allowed}`, plus always `{value}`.
    pub fn validate_value(&self, value: &serde_json::Value) -> Result<(), ValidationError> {
        match self {
            Self::MinLength(n) => {
                if let Some(s) = value.as_str() {
                    min_length(*n)
                        .validate(s)
                        .map_err(|e| e.with_param("min", n.to_string()).with_param("value", format!("{value}")))?;
                }
                Ok(())
            },
            Self::MaxLength(n) => {
                if let Some(s) = value.as_str() {
                    max_length(*n)
                        .validate(s)
                        .map_err(|e| e.with_param("max", n.to_string()).with_param("value", format!("{value}")))?;
                }
                Ok(())
            },
            Self::Pattern(p) => {
                if let Some(s) = value.as_str() {
                    let re = compile_regex(p)?;
                    if !re.is_match(s) {
                        return Err(ValidationError::invalid_format("", "regex")
                            .with_param("pattern", p.clone())
                            .with_param("value", format!("{value}")));
                    }
                }
                Ok(())
            },
            Self::Min(bound) => {
                if let Some(ord) = json_number_cmp(value, bound)
                    && ord.is_lt()
                {
                    return Err(ValidationError::new(
                        "min",
                        "Value must be at least {min}",
                    )
                    .with_param("min", format_json_number(bound))
                    .with_param("value", format!("{value}")));
                }
                Ok(())
            },
            Self::Max(bound) => {
                if let Some(ord) = json_number_cmp(value, bound)
                    && ord.is_gt()
                {
                    return Err(ValidationError::new(
                        "max",
                        "Value must be at most {max}",
                    )
                    .with_param("max", format_json_number(bound))
                    .with_param("value", format!("{value}")));
                }
                Ok(())
            },
            Self::GreaterThan(bound) => {
                if let Some(ord) = json_number_cmp(value, bound)
                    && !ord.is_gt()
                {
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
                if let Some(ord) = json_number_cmp(value, bound)
                    && !ord.is_lt()
                {
                    return Err(ValidationError::new(
                        "less_than",
                        "Value must be less than {max}",
                    )
                    .with_param("max", format_json_number(bound))
                    .with_param("value", format!("{value}")));
                }
                Ok(())
            },
            Self::OneOf(values) => {
                if values.is_empty() {
                    return Ok(());
                }
                let has_same_type = values
                    .iter()
                    .any(|v| std::mem::discriminant(v) == std::mem::discriminant(value));
                if !has_same_type {
                    return Ok(());
                }
                if !values.contains(value) {
                    let allowed = values
                        .iter()
                        .map(|v| format!("{v}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    return Err(ValidationError::new(
                        "one_of",
                        "must be one of {allowed}",
                    )
                    .with_param("allowed", allowed)
                    .with_param("value", format!("{value}")));
                }
                Ok(())
            },
            Self::MinItems(n) => {
                if let Some(items) = value.as_array() {
                    min_size::<serde_json::Value>(*n)
                        .validate(items.as_slice())
                        .map_err(|e| e.with_param("min", n.to_string()))?;
                }
                Ok(())
            },
            Self::MaxItems(n) => {
                if let Some(items) = value.as_array() {
                    max_size::<serde_json::Value>(*n)
                        .validate(items.as_slice())
                        .map_err(|e| e.with_param("max", n.to_string()))?;
                }
                Ok(())
            },
            Self::Email => {
                if let Some(s) = value.as_str() {
                    static EMAIL_RE: std::sync::LazyLock<regex::Regex> =
                        std::sync::LazyLock::new(|| regex::Regex::new(EMAIL_PATTERN).expect("email regex"));
                    if !EMAIL_RE.is_match(s) {
                        return Err(ValidationError::invalid_format("", "email")
                            .with_param("value", format!("{value}")));
                    }
                }
                Ok(())
            },
            Self::Url => {
                if let Some(s) = value.as_str() {
                    static URL_RE: std::sync::LazyLock<regex::Regex> =
                        std::sync::LazyLock::new(|| regex::Regex::new(URL_PATTERN).expect("url regex"));
                    if !URL_RE.is_match(s) {
                        return Err(ValidationError::invalid_format("", "url")
                            .with_param("value", format!("{value}")));
                    }
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
        assert!(ValueRule::MinLength(3).validate_value(&json!("alice")).is_ok());
        assert!(ValueRule::MinLength(3).validate_value(&json!("ab")).is_err());
    }

    #[test]
    fn min_length_silent_pass_on_non_string() {
        assert!(ValueRule::MinLength(3).validate_value(&json!(42)).is_ok());
    }

    #[test]
    fn min_rejects_below_bound() {
        let rule = ValueRule::Min(serde_json::Number::from(10));
        assert!(rule.validate_value(&json!(5)).is_err());
        assert!(rule.validate_value(&json!(15)).is_ok());
    }

    #[test]
    fn one_of_empty_passes() {
        assert!(ValueRule::OneOf(vec![]).validate_value(&json!("x")).is_ok());
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
        let err = ValueRule::MinLength(3).validate_value(&json!("hi")).unwrap_err();
        assert_eq!(err.param("min"), Some("3"));
    }
}

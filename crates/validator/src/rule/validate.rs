//! [`Rule::validate_value`] — the value-validation path for a rule.

use super::{
    Rule,
    helpers::{compile_regex, format_json_number, json_number_cmp, override_message},
};
use crate::{
    foundation::{Validate, ValidationError},
    validators::{
        content::{EMAIL_PATTERN, URL_PATTERN},
        max_length, max_size, min_length, min_size,
    },
};

impl Rule {
    /// Validates a JSON value against this rule.
    ///
    /// Only meaningful for value-validation rules. Predicate rules
    /// return `Ok(())` (use [`evaluate`](Self::evaluate) instead).
    /// Deferred rules return `Ok(())` (skipped at static time).
    ///
    /// # Type Coercion
    ///
    /// When the JSON value type doesn't match the rule's expected type
    /// (e.g. `MinLength` on a number), validation passes silently (`Ok(())`).
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError`] when the value violates the rule.
    /// The error's `code` field identifies the rule (e.g. `"min_length"`,
    /// `"pattern"`, `"one_of"`). Custom `message` overrides the default
    /// if provided in the rule.
    pub fn validate_value(&self, value: &serde_json::Value) -> Result<(), ValidationError> {
        match self {
            // ── Value rules ─────────────────────────────────────────
            Self::MinLength {
                min: min_val,
                message,
            } => {
                if let Some(s) = value.as_str() {
                    min_length(*min_val)
                        .validate(s)
                        .map_err(|e| override_message(e, message))?;
                }
                Ok(())
            },
            Self::MaxLength {
                max: max_val,
                message,
            } => {
                if let Some(s) = value.as_str() {
                    max_length(*max_val)
                        .validate(s)
                        .map_err(|e| override_message(e, message))?;
                }
                Ok(())
            },
            Self::Pattern { pattern, message } => {
                if let Some(s) = value.as_str() {
                    let re = compile_regex(pattern)?;
                    if !re.is_match(s) {
                        let err = ValidationError::invalid_format("", "regex")
                            .with_param("pattern", pattern.clone());
                        return Err(override_message(err, message));
                    }
                }
                Ok(())
            },
            Self::Min {
                min: min_val,
                message,
            } => {
                if let Some(ord) = json_number_cmp(value, min_val)
                    && ord.is_lt()
                {
                    let err = ValidationError::new(
                        "min",
                        format!("Value must be at least {}", format_json_number(min_val)),
                    )
                    .with_param("min", format_json_number(min_val))
                    .with_param("actual", value.to_string());
                    return Err(override_message(err, message));
                }
                Ok(())
            },
            Self::Max {
                max: max_val,
                message,
            } => {
                if let Some(ord) = json_number_cmp(value, max_val)
                    && ord.is_gt()
                {
                    let err = ValidationError::new(
                        "max",
                        format!("Value must be at most {}", format_json_number(max_val)),
                    )
                    .with_param("max", format_json_number(max_val))
                    .with_param("actual", value.to_string());
                    return Err(override_message(err, message));
                }
                Ok(())
            },
            Self::GreaterThan {
                min: min_val,
                message,
            } => {
                if let Some(ord) = json_number_cmp(value, min_val)
                    && !ord.is_gt()
                {
                    let err = ValidationError::new(
                        "greater_than",
                        format!("Value must be greater than {}", format_json_number(min_val)),
                    )
                    .with_param("min", format_json_number(min_val))
                    .with_param("actual", value.to_string());
                    return Err(override_message(err, message));
                }
                Ok(())
            },
            Self::LessThan {
                max: max_val,
                message,
            } => {
                if let Some(ord) = json_number_cmp(value, max_val)
                    && !ord.is_lt()
                {
                    let err = ValidationError::new(
                        "less_than",
                        format!("Value must be less than {}", format_json_number(max_val)),
                    )
                    .with_param("max", format_json_number(max_val))
                    .with_param("actual", value.to_string());
                    return Err(override_message(err, message));
                }
                Ok(())
            },
            Self::OneOf { values, message } => {
                if values.is_empty() {
                    return Ok(());
                }
                // Check if any candidate shares the same JSON type as the input.
                // If no type match exists, pass silently (consistent with other value rules).
                let has_same_type = values
                    .iter()
                    .any(|v| std::mem::discriminant(v) == std::mem::discriminant(value));
                if !has_same_type {
                    return Ok(());
                }
                if !values.contains(value) {
                    let msg = message
                        .clone()
                        .unwrap_or_else(|| "must be one of the allowed values".to_owned());
                    return Err(ValidationError::new("one_of", msg));
                }
                Ok(())
            },
            Self::MinItems {
                min: min_val,
                message,
            } => {
                if let Some(items) = value.as_array() {
                    min_size::<serde_json::Value>(*min_val)
                        .validate(items.as_slice())
                        .map_err(|e| override_message(e, message))?;
                }
                Ok(())
            },
            Self::MaxItems {
                max: max_val,
                message,
            } => {
                if let Some(items) = value.as_array() {
                    max_size::<serde_json::Value>(*max_val)
                        .validate(items.as_slice())
                        .map_err(|e| override_message(e, message))?;
                }
                Ok(())
            },

            Self::Email { message } => {
                if let Some(s) = value.as_str() {
                    static EMAIL_RE: std::sync::LazyLock<regex::Regex> =
                        std::sync::LazyLock::new(|| {
                            regex::Regex::new(EMAIL_PATTERN).expect("email regex is valid")
                        });
                    if !EMAIL_RE.is_match(s) {
                        let err = ValidationError::invalid_format("", "email");
                        return Err(override_message(err, message));
                    }
                }
                Ok(())
            },
            Self::Url { message } => {
                if let Some(s) = value.as_str() {
                    static URL_RE: std::sync::LazyLock<regex::Regex> =
                        std::sync::LazyLock::new(|| {
                            regex::Regex::new(URL_PATTERN).expect("url regex is valid")
                        });
                    if !URL_RE.is_match(s) {
                        let err = ValidationError::invalid_format("", "url");
                        return Err(override_message(err, message));
                    }
                }
                Ok(())
            },

            // ── Deferred — skip at static time ──────────────────────
            Self::UniqueBy { .. } | Self::Custom { .. } => Ok(()),

            // ── Context predicates — not value checks ───────────────
            Self::Eq { .. }
            | Self::Ne { .. }
            | Self::Gt { .. }
            | Self::Gte { .. }
            | Self::Lt { .. }
            | Self::Lte { .. }
            | Self::IsTrue { .. }
            | Self::IsFalse { .. }
            | Self::Set { .. }
            | Self::Empty { .. }
            | Self::Contains { .. }
            | Self::Matches { .. }
            | Self::In { .. } => Ok(()),

            // ── Logical combinators ─────────────────────────────────
            Self::All { rules } => {
                let mut errors = Vec::new();
                for rule in rules {
                    if let Err(e) = rule.validate_value(value) {
                        errors.push(e);
                    }
                }
                if errors.is_empty() {
                    Ok(())
                } else if errors.len() == 1 {
                    Err(errors.into_iter().next().unwrap())
                } else {
                    let count = errors.len();
                    Err(
                        ValidationError::new("all_failed", format!("{count} of the rules failed"))
                            .with_nested(errors),
                    )
                }
            },
            Self::Any { rules } => {
                if rules.is_empty() {
                    return Ok(());
                }
                let mut errors = Vec::new();
                for rule in rules {
                    match rule.validate_value(value) {
                        Ok(()) => return Ok(()),
                        Err(e) => errors.push(e),
                    }
                }
                let count = errors.len();
                Err(
                    ValidationError::new("any_failed", format!("All {count} alternatives failed"))
                        .with_nested(errors),
                )
            },
            Self::Not { inner } => match inner.validate_value(value) {
                Ok(()) => Err(ValidationError::new("not_failed", "negated rule passed")),
                Err(_) => Ok(()),
            },
        }
    }
}

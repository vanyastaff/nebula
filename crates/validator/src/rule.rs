//! Unified declarative rule system.
//!
//! [`Rule`] is the single source of truth for both value validation and
//! context-predicate evaluation. Adding a new rule means adding one
//! enum variant and one match arm — everything else works automatically.
//!
//! Rules are serializable to/from JSON via `serde` using `#[serde(tag = "rule")]`
//! internally-tagged representation. For example:
//!
//! ```json
//! {"rule": "min_length", "min": 3}
//! {"rule": "eq", "field": "status", "value": "active"}
//! {"rule": "all", "rules": [{"rule": "min_length", "min": 3}, {"rule": "max_length", "max": 20}]}
//! ```
//!
//! # Rule Categories
//!
//! | Category | Variants | Method | Classification |
//! |---|---|---|---|
//! | Value validation | `MinLength`, `MaxLength`, `Pattern`, `Min`, `Max`, `OneOf`, `MinItems`, `MaxItems` | [`Rule::validate_value`] | [`Rule::is_value_rule`] |
//! | Context predicate | `Eq`, `Ne`, `Gt`, `Gte`, `Lt`, `Lte`, `IsTrue`, `IsFalse`, `Set`, `Empty`, `Contains`, `Matches`, `In` | [`Rule::evaluate`] | [`Rule::is_predicate`] |
//! | Deferred | `Custom`, `UniqueBy` | skipped at schema time | [`Rule::is_deferred`] |
//! | Logical combinator | `All`, `Any`, `Not` | both methods | — |
//!
//! # Type Safety
//!
//! Value rules silently pass when the JSON type doesn't match (e.g. `MinLength`
//! on a number returns `Ok`). This allows rules to be applied broadly without
//! requiring callers to pre-filter by type.
//!
//! Predicate rules return `Ok(())` from [`Rule::validate_value`] (they are not
//! value checks), and value rules return `true` from [`Rule::evaluate`] (they
//! are not predicates). This makes both methods safe to call on any rule variant.
//!
//! # Examples
//!
//! ## Value Validation
//!
//! ```rust
//! use nebula_validator::Rule;
//! use serde_json::json;
//!
//! let rule = Rule::MinLength { min: 3, message: None };
//! assert!(rule.validate_value(&json!("alice")).is_ok());
//! assert!(rule.validate_value(&json!("ab")).is_err());
//!
//! // Non-matching types pass silently
//! assert!(rule.validate_value(&json!(42)).is_ok());
//! ```
//!
//! ## Context Predicates
//!
//! ```rust
//! use nebula_validator::Rule;
//! use serde_json::json;
//!
//! let rule = Rule::Eq {
//!     field: "status".into(),
//!     value: json!("active"),
//! };
//! let values: std::collections::HashMap<String, serde_json::Value> =
//!     serde_json::from_value(json!({ "status": "active" })).unwrap();
//! assert!(rule.evaluate(&values));
//! ```
//!
//! ## Logical Combinators
//!
//! ```rust
//! use nebula_validator::Rule;
//! use serde_json::json;
//!
//! let rule = Rule::All {
//!     rules: vec![
//!         Rule::MinLength { min: 3, message: None },
//!         Rule::MaxLength { max: 20, message: None },
//!     ],
//! };
//! assert!(rule.validate_value(&json!("hello")).is_ok());
//! assert!(rule.validate_value(&json!("ab")).is_err());
//! ```
//!
//! ## Serde Roundtrip
//!
//! ```rust
//! use nebula_validator::Rule;
//! use serde_json::json;
//!
//! let rule = Rule::MinLength { min: 5, message: Some("too short".into()) };
//! let json = serde_json::to_value(&rule).unwrap();
//! assert_eq!(json, json!({"rule": "min_length", "min": 5, "message": "too short"}));
//!
//! let back: Rule = serde_json::from_value(json).unwrap();
//! assert_eq!(back, rule);
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

use crate::foundation::{Validate, ValidationError};
use crate::validators::{max_length, max_size, min_length, min_size};

// ============================================================================
// REGEX CACHE — avoids recompiling patterns on every validate_value/evaluate call
// ============================================================================

/// Global regex cache to avoid recompiling the same pattern string repeatedly.
/// Keyed by the raw pattern string; values are compiled `Regex` instances.
static REGEX_CACHE: std::sync::LazyLock<Mutex<HashMap<String, regex::Regex>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

/// Returns a compiled regex for the given pattern, using the cache.
fn cached_regex(pattern: &str) -> Result<regex::Regex, ValidationError> {
    {
        let cache = REGEX_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(re) = cache.get(pattern) {
            return Ok(re.clone());
        }
    }
    let re = regex::Regex::new(pattern)
        .map_err(|e| ValidationError::new("invalid_pattern", format!("invalid regex: {e}")))?;
    let mut cache = REGEX_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache.entry(pattern.to_owned()).or_insert(re.clone());
    Ok(re)
}

// ============================================================================
// JSON NUMBER HELPERS — precision-safe comparison for integers > 2^53
// ============================================================================

/// Ordering between two JSON numbers, using the highest-precision path.
///
/// Tries `i64` first, then `u64`, then `f64`. Returns `None` if either
/// operand is not a number or if a `NaN` comparison is indeterminate.
fn json_number_cmp(
    value: &serde_json::Value,
    bound: &serde_json::Number,
) -> Option<std::cmp::Ordering> {
    let val_num = value.as_number()?;

    // i64 path — covers most integers exactly
    if let (Some(a), Some(b)) = (val_num.as_i64(), bound.as_i64()) {
        return Some(a.cmp(&b));
    }

    // u64 path — covers large positive integers that don't fit in i64
    if let (Some(a), Some(b)) = (val_num.as_u64(), bound.as_u64()) {
        return Some(a.cmp(&b));
    }

    // f64 fallback — handles floats; may lose precision for very large ints
    let a = val_num.as_f64()?;
    let b = bound.as_f64()?;
    a.partial_cmp(&b)
}

fn format_json_number(n: &serde_json::Number) -> String {
    n.to_string()
}

/// Unified declarative rule.
///
/// Covers value validation, context predicates, deferred runtime checks,
/// and logical combinators. One enum, one source of truth.
///
/// See the [module-level documentation](self) for categories, examples,
/// and serialization format.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "rule", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Rule {
    // ── Value validation rules ──────────────────────────────────────────
    /// String must match the regular expression.
    Pattern {
        /// Regular expression pattern.
        pattern: String,
        /// Optional custom error message.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    /// String must be at least `min` characters.
    MinLength {
        /// Minimum character count (inclusive).
        min: usize,
        /// Optional custom error message.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    /// String must be at most `max` characters.
    MaxLength {
        /// Maximum character count (inclusive).
        max: usize,
        /// Optional custom error message.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    /// Number must be ≥ `min`.
    Min {
        /// Lower bound (inclusive).
        min: serde_json::Number,
        /// Optional custom error message.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    /// Number must be ≤ `max`.
    Max {
        /// Upper bound (inclusive).
        max: serde_json::Number,
        /// Optional custom error message.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    /// Value must be one of the given options.
    OneOf {
        /// Allowed values.
        values: Vec<serde_json::Value>,
        /// Optional custom error message.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    /// Collection must contain at least `min` items.
    MinItems {
        /// Minimum item count (inclusive).
        min: usize,
        /// Optional custom error message.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    /// Collection must contain at most `max` items.
    MaxItems {
        /// Maximum item count (inclusive).
        max: usize,
        /// Optional custom error message.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    // ── Deferred rules (runtime only) ───────────────────────────────────
    /// Each list item must have a unique value for the given sub-field key.
    ///
    /// **Deferred**: not evaluated at schema-validation time.
    UniqueBy {
        /// Sub-field key path within each item (e.g. `"name"`).
        key: String,
        /// Optional custom error message.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    /// Custom expression-based validation.
    ///
    /// **Deferred**: not evaluated at schema-validation time.
    Custom {
        /// Expression string forwarded to the runtime evaluator.
        expression: String,
        /// Optional custom error message.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    // ── Context predicates (field-level conditions) ─────────────────────
    /// `field == value`
    Eq {
        /// Field id to read.
        field: String,
        /// Expected value.
        value: serde_json::Value,
    },

    /// `field != value`
    Ne {
        /// Field id to read.
        field: String,
        /// Value to differ from.
        value: serde_json::Value,
    },

    /// `field > value` (numeric comparison)
    Gt {
        /// Field id to read.
        field: String,
        /// Lower exclusive bound.
        value: serde_json::Number,
    },

    /// `field >= value` (numeric comparison)
    Gte {
        /// Field id to read.
        field: String,
        /// Lower inclusive bound.
        value: serde_json::Number,
    },

    /// `field < value` (numeric comparison)
    Lt {
        /// Field id to read.
        field: String,
        /// Upper exclusive bound.
        value: serde_json::Number,
    },

    /// `field <= value` (numeric comparison)
    Lte {
        /// Field id to read.
        field: String,
        /// Upper inclusive bound.
        value: serde_json::Number,
    },

    /// `field == true`
    IsTrue {
        /// Field id to read.
        field: String,
    },

    /// `field == false`
    IsFalse {
        /// Field id to read.
        field: String,
    },

    /// Field has a non-null, non-empty value.
    Set {
        /// Field id to read.
        field: String,
    },

    /// Field is null, absent, or empty string/array.
    Empty {
        /// Field id to read.
        field: String,
    },

    /// String or array field contains the given value.
    Contains {
        /// Field id to read.
        field: String,
        /// Value to search for.
        value: serde_json::Value,
    },

    /// String field matches the regular expression.
    Matches {
        /// Field id to read.
        field: String,
        /// Regular expression pattern.
        pattern: String,
    },

    /// Field value is a member of the given set.
    In {
        /// Field id to read.
        field: String,
        /// Allowed values.
        values: Vec<serde_json::Value>,
    },

    // ── Logical combinators ─────────────────────────────────────────────
    /// All inner rules must pass.
    All {
        /// Inner rules.
        rules: Vec<Rule>,
    },

    /// At least one inner rule must pass.
    Any {
        /// Inner rules.
        rules: Vec<Rule>,
    },

    /// Negates the inner rule.
    Not {
        /// Inner rule to negate.
        inner: Box<Rule>,
    },
}

impl Rule {
    // ── Shorthand constructors ──────────────────────────────────────────

    /// Creates a [`Pattern`](Self::Pattern) rule.
    #[must_use]
    pub fn pattern(pattern: impl Into<String>) -> Self {
        Self::Pattern {
            pattern: pattern.into(),
            message: None,
        }
    }

    /// Creates a [`MinLength`](Self::MinLength) rule.
    #[must_use]
    pub fn min_length(min: usize) -> Self {
        Self::MinLength {
            min,
            message: None,
        }
    }

    /// Creates a [`MaxLength`](Self::MaxLength) rule.
    #[must_use]
    pub fn max_length(max: usize) -> Self {
        Self::MaxLength {
            max,
            message: None,
        }
    }

    /// Creates a [`Min`](Self::Min) rule from an `i64`.
    #[must_use]
    pub fn min_value(min: i64) -> Self {
        Self::Min {
            min: serde_json::Number::from(min),
            message: None,
        }
    }

    /// Creates a [`Max`](Self::Max) rule from an `i64`.
    #[must_use]
    pub fn max_value(max: i64) -> Self {
        Self::Max {
            max: serde_json::Number::from(max),
            message: None,
        }
    }

    /// Creates a [`Min`](Self::Min) rule from an `f64`.
    #[must_use]
    pub fn min_value_f64(min: f64) -> Self {
        Self::Min {
            min: serde_json::Number::from_f64(min).expect("finite f64"),
            message: None,
        }
    }

    /// Creates a [`Max`](Self::Max) rule from an `f64`.
    #[must_use]
    pub fn max_value_f64(max: f64) -> Self {
        Self::Max {
            max: serde_json::Number::from_f64(max).expect("finite f64"),
            message: None,
        }
    }

    /// Creates a [`OneOf`](Self::OneOf) rule.
    #[must_use]
    pub fn one_of<V: Into<serde_json::Value>>(values: impl IntoIterator<Item = V>) -> Self {
        Self::OneOf {
            values: values.into_iter().map(Into::into).collect(),
            message: None,
        }
    }

    /// Creates a [`MinItems`](Self::MinItems) rule.
    #[must_use]
    pub fn min_items(min: usize) -> Self {
        Self::MinItems {
            min,
            message: None,
        }
    }

    /// Creates a [`MaxItems`](Self::MaxItems) rule.
    #[must_use]
    pub fn max_items(max: usize) -> Self {
        Self::MaxItems {
            max,
            message: None,
        }
    }

    /// Creates a [`UniqueBy`](Self::UniqueBy) rule.
    #[must_use]
    pub fn unique_by(key: impl Into<String>) -> Self {
        Self::UniqueBy {
            key: key.into(),
            message: None,
        }
    }

    /// Creates a [`Custom`](Self::Custom) rule.
    #[must_use]
    pub fn custom(expression: impl Into<String>) -> Self {
        Self::Custom {
            expression: expression.into(),
            message: None,
        }
    }

    /// Attaches a custom error message to this rule.
    #[must_use]
    pub fn with_message(self, msg: impl Into<String>) -> Self {
        let msg = Some(msg.into());
        match self {
            Self::Pattern { pattern, .. } => Self::Pattern { pattern, message: msg },
            Self::MinLength { min, .. } => Self::MinLength { min, message: msg },
            Self::MaxLength { max, .. } => Self::MaxLength { max, message: msg },
            Self::Min { min, .. } => Self::Min { min, message: msg },
            Self::Max { max, .. } => Self::Max { max, message: msg },
            Self::OneOf { values, .. } => Self::OneOf { values, message: msg },
            Self::MinItems { min, .. } => Self::MinItems { min, message: msg },
            Self::MaxItems { max, .. } => Self::MaxItems { max, message: msg },
            Self::UniqueBy { key, .. } => Self::UniqueBy { key, message: msg },
            Self::Custom { expression, .. } => Self::Custom { expression, message: msg },
            // Predicate variants don't have messages
            other => other,
        }
    }

    /// Returns `true` if this rule validates a single value
    /// (as opposed to evaluating context predicates).
    ///
    /// Deferred rules (`Custom`, `UniqueBy`) are **not** classified as value rules;
    /// use [`is_deferred`](Self::is_deferred) to check for those.
    #[must_use]
    pub fn is_value_rule(&self) -> bool {
        matches!(
            self,
            Self::Pattern { .. }
                | Self::MinLength { .. }
                | Self::MaxLength { .. }
                | Self::Min { .. }
                | Self::Max { .. }
                | Self::OneOf { .. }
                | Self::MinItems { .. }
                | Self::MaxItems { .. }
        )
    }

    /// Returns `true` if this rule evaluates context predicates
    /// (checks a sibling field value).
    #[must_use]
    pub fn is_predicate(&self) -> bool {
        matches!(
            self,
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
                | Self::In { .. }
        )
    }

    /// Returns `true` if this rule requires runtime expression context.
    ///
    /// Deferred rules are skipped during static schema validation.
    #[must_use]
    pub fn is_deferred(&self) -> bool {
        matches!(self, Self::UniqueBy { .. } | Self::Custom { .. })
    }

    /// Collects all field IDs referenced by context predicates in this rule.
    ///
    /// Recurses into logical combinators (`All`, `Any`, `Not`).
    /// Value-only rules and deferred rules return no references.
    pub fn field_references<'a>(&'a self, out: &mut Vec<&'a str>) {
        match self {
            Self::Eq { field, .. }
            | Self::Ne { field, .. }
            | Self::Gt { field, .. }
            | Self::Gte { field, .. }
            | Self::Lt { field, .. }
            | Self::Lte { field, .. }
            | Self::IsTrue { field }
            | Self::IsFalse { field }
            | Self::Set { field }
            | Self::Empty { field }
            | Self::Contains { field, .. }
            | Self::Matches { field, .. }
            | Self::In { field, .. } => out.push(field),
            Self::All { rules } | Self::Any { rules } => {
                for rule in rules {
                    rule.field_references(out);
                }
            }
            Self::Not { inner } => inner.field_references(out),
            _ => {}
        }
    }

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
            }
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
            }
            Self::Pattern { pattern, message } => {
                if let Some(s) = value.as_str() {
                    let re = cached_regex(pattern)?;
                    if !re.is_match(s) {
                        let err = ValidationError::invalid_format("", "regex")
                            .with_param("pattern", pattern.clone());
                        return Err(override_message(err, message));
                    }
                }
                Ok(())
            }
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
            }
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
            }
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
            }
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
            }
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
            }

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
            }
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
            }
            Self::Not { inner } => match inner.validate_value(value) {
                Ok(()) => Err(ValidationError::new("not_failed", "negated rule passed")),
                Err(_) => Ok(()),
            },
        }
    }

    /// Evaluates this rule as a boolean predicate against field values.
    ///
    /// Only meaningful for context-predicate rules and logical combinators.
    /// Value-validation rules return `true` (vacuously — use
    /// [`validate_value`](Self::validate_value) instead).
    ///
    /// # Missing Fields
    ///
    /// Behavior when the target field is absent:
    /// - `Eq` → `false` (can't equal anything)
    /// - `Ne` → `true` (absent ≠ any value)
    /// - `Gt`, `Gte`, `Lt`, `Lte` → `false` (no number to compare)
    /// - `IsTrue`, `IsFalse` → `false` (no boolean)
    /// - `Set` → `false`, `Empty` → `true`
    /// - `Contains`, `Matches`, `In` → `false`
    #[must_use]
    pub fn evaluate(&self, values: &std::collections::HashMap<String, serde_json::Value>) -> bool {
        match self {
            // ── Context predicates ──────────────────────────────────
            Self::Eq { field, value } => values.get(field).is_some_and(|v| v == value),
            Self::Ne { field, value } => values.get(field).is_none_or(|v| v != value),
            Self::Gt { field, value } => cmp_number(values.get(field), value, |a, b| a > b),
            Self::Gte { field, value } => cmp_number(values.get(field), value, |a, b| a >= b),
            Self::Lt { field, value } => cmp_number(values.get(field), value, |a, b| a < b),
            Self::Lte { field, value } => cmp_number(values.get(field), value, |a, b| a <= b),
            Self::IsTrue { field } => {
                values.get(field).and_then(serde_json::Value::as_bool) == Some(true)
            }
            Self::IsFalse { field } => {
                values.get(field).and_then(serde_json::Value::as_bool) == Some(false)
            }
            Self::Set { field } => values.get(field).is_some_and(|v| {
                !v.is_null()
                    && match v {
                        serde_json::Value::String(s) => !s.is_empty(),
                        serde_json::Value::Array(a) => !a.is_empty(),
                        _ => true,
                    }
            }),
            Self::Empty { field } => values.get(field).is_none_or(|v| {
                v.is_null()
                    || match v {
                        serde_json::Value::String(s) => s.is_empty(),
                        serde_json::Value::Array(a) => a.is_empty(),
                        _ => false,
                    }
            }),
            Self::Contains { field, value } => values.get(field).is_some_and(|v| match v {
                serde_json::Value::String(s) => {
                    value.as_str().is_some_and(|needle| s.contains(needle))
                }
                serde_json::Value::Array(items) => items.contains(value),
                _ => false,
            }),
            Self::Matches { field, pattern } => values
                .get(field)
                .and_then(serde_json::Value::as_str)
                .is_some_and(|string| cached_regex(pattern).is_ok_and(|re| re.is_match(string))),
            Self::In {
                field,
                values: candidates,
            } => values
                .get(field)
                .is_some_and(|current| candidates.contains(current)),

            // ── Logical combinators ─────────────────────────────────
            Self::All { rules } => rules.iter().all(|r| r.evaluate(values)),
            Self::Any { rules } => rules.iter().any(|r| r.evaluate(values)),
            Self::Not { inner } => !inner.evaluate(values),

            // ── Value/deferred rules — vacuously true ───────────────
            _ => true,
        }
    }
}

/// Replaces the error message if an override is provided.
fn override_message(mut error: ValidationError, message: &Option<String>) -> ValidationError {
    if let Some(msg) = message {
        error.message = std::borrow::Cow::Owned(msg.clone());
    }
    error
}

/// Numeric comparison helper.
fn cmp_number(
    value: Option<&serde_json::Value>,
    rhs: &serde_json::Number,
    op: impl Fn(f64, f64) -> bool,
) -> bool {
    let Some(lhs) = value.and_then(serde_json::Value::as_f64) else {
        return false;
    };
    let Some(rhs) = rhs.as_f64() else {
        return false;
    };
    op(lhs, rhs)
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    // ── Value validation ────────────────────────────────────────────────

    #[test]
    fn min_length_passes() {
        let rule = Rule::MinLength {
            min: 3,
            message: None,
        };
        assert!(rule.validate_value(&json!("alice")).is_ok());
    }

    #[test]
    fn min_length_fails() {
        let rule = Rule::MinLength {
            min: 3,
            message: None,
        };
        let err = rule.validate_value(&json!("ab")).unwrap_err();
        assert_eq!(err.code.as_ref(), "min_length");
    }

    #[test]
    fn min_length_custom_message() {
        let rule = Rule::MinLength {
            min: 5,
            message: Some("too short!".into()),
        };
        let err = rule.validate_value(&json!("ab")).unwrap_err();
        assert_eq!(err.message.as_ref(), "too short!");
    }

    #[test]
    fn max_length_passes() {
        let rule = Rule::MaxLength {
            max: 10,
            message: None,
        };
        assert!(rule.validate_value(&json!("hello")).is_ok());
    }

    #[test]
    fn max_length_fails() {
        let rule = Rule::MaxLength {
            max: 3,
            message: None,
        };
        assert!(rule.validate_value(&json!("hello")).is_err());
    }

    #[test]
    fn pattern_passes() {
        let rule = Rule::Pattern {
            pattern: "^[a-z]+$".into(),
            message: None,
        };
        assert!(rule.validate_value(&json!("hello")).is_ok());
    }

    #[test]
    fn pattern_fails() {
        let rule = Rule::Pattern {
            pattern: "^[a-z]+$".into(),
            message: None,
        };
        assert!(rule.validate_value(&json!("Hello123")).is_err());
    }

    #[test]
    fn min_numeric_passes() {
        let rule = Rule::Min {
            min: serde_json::Number::from(5),
            message: None,
        };
        assert!(rule.validate_value(&json!(10)).is_ok());
    }

    #[test]
    fn min_numeric_fails() {
        let rule = Rule::Min {
            min: serde_json::Number::from(5),
            message: None,
        };
        assert!(rule.validate_value(&json!(3)).is_err());
    }

    #[test]
    fn max_numeric_passes() {
        let rule = Rule::Max {
            max: serde_json::Number::from(100),
            message: None,
        };
        assert!(rule.validate_value(&json!(50)).is_ok());
    }

    #[test]
    fn max_numeric_fails() {
        let rule = Rule::Max {
            max: serde_json::Number::from(10),
            message: None,
        };
        assert!(rule.validate_value(&json!(20)).is_err());
    }

    #[test]
    fn one_of_passes() {
        let rule = Rule::OneOf {
            values: vec![json!("a"), json!("b"), json!("c")],
            message: None,
        };
        assert!(rule.validate_value(&json!("b")).is_ok());
    }

    #[test]
    fn one_of_fails() {
        let rule = Rule::OneOf {
            values: vec![json!("a"), json!("b")],
            message: None,
        };
        assert!(rule.validate_value(&json!("x")).is_err());
    }

    #[test]
    fn min_items_passes() {
        let rule = Rule::MinItems {
            min: 2,
            message: None,
        };
        assert!(rule.validate_value(&json!([1, 2, 3])).is_ok());
    }

    #[test]
    fn min_items_fails() {
        let rule = Rule::MinItems {
            min: 3,
            message: None,
        };
        assert!(rule.validate_value(&json!([1])).is_err());
    }

    #[test]
    fn max_items_passes() {
        let rule = Rule::MaxItems {
            max: 5,
            message: None,
        };
        assert!(rule.validate_value(&json!([1, 2])).is_ok());
    }

    #[test]
    fn max_items_fails() {
        let rule = Rule::MaxItems {
            max: 2,
            message: None,
        };
        assert!(rule.validate_value(&json!([1, 2, 3])).is_err());
    }

    #[test]
    fn deferred_rules_skip() {
        let rule = Rule::UniqueBy {
            key: "id".into(),
            message: None,
        };
        assert!(rule.validate_value(&json!([1, 1])).is_ok());
        assert!(rule.is_deferred());
    }

    #[test]
    fn non_matching_type_passes_silently() {
        // MinLength on a number → skip, no error
        let rule = Rule::MinLength {
            min: 3,
            message: None,
        };
        assert!(rule.validate_value(&json!(42)).is_ok());
    }

    // ── Context predicates ──────────────────────────────────────────────

    fn values(pairs: &[(&str, serde_json::Value)]) -> HashMap<String, serde_json::Value> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn eq_predicate() {
        let rule = Rule::Eq {
            field: "status".into(),
            value: json!("active"),
        };
        assert!(rule.evaluate(&values(&[("status", json!("active"))])));
        assert!(!rule.evaluate(&values(&[("status", json!("inactive"))])));
    }

    #[test]
    fn ne_predicate() {
        let rule = Rule::Ne {
            field: "status".into(),
            value: json!("deleted"),
        };
        assert!(rule.evaluate(&values(&[("status", json!("active"))])));
        assert!(!rule.evaluate(&values(&[("status", json!("deleted"))])));
    }

    #[test]
    fn gt_predicate() {
        let rule = Rule::Gt {
            field: "age".into(),
            value: serde_json::Number::from(18),
        };
        assert!(rule.evaluate(&values(&[("age", json!(20))])));
        assert!(!rule.evaluate(&values(&[("age", json!(18))])));
    }

    #[test]
    fn gte_predicate() {
        let rule = Rule::Gte {
            field: "age".into(),
            value: serde_json::Number::from(18),
        };
        assert!(rule.evaluate(&values(&[("age", json!(20))])));
        assert!(rule.evaluate(&values(&[("age", json!(18))])));
        assert!(!rule.evaluate(&values(&[("age", json!(17))])));
    }

    #[test]
    fn lt_predicate() {
        let rule = Rule::Lt {
            field: "count".into(),
            value: serde_json::Number::from(10),
        };
        assert!(rule.evaluate(&values(&[("count", json!(5))])));
        assert!(!rule.evaluate(&values(&[("count", json!(10))])));
        assert!(!rule.evaluate(&values(&[("count", json!(15))])));
    }

    #[test]
    fn lte_predicate() {
        let rule = Rule::Lte {
            field: "count".into(),
            value: serde_json::Number::from(10),
        };
        assert!(rule.evaluate(&values(&[("count", json!(5))])));
        assert!(rule.evaluate(&values(&[("count", json!(10))])));
        assert!(!rule.evaluate(&values(&[("count", json!(11))])));
    }

    #[test]
    fn is_true_predicate() {
        let rule = Rule::IsTrue {
            field: "enabled".into(),
        };
        assert!(rule.evaluate(&values(&[("enabled", json!(true))])));
        assert!(!rule.evaluate(&values(&[("enabled", json!(false))])));
    }

    #[test]
    fn is_false_predicate() {
        let rule = Rule::IsFalse {
            field: "disabled".into(),
        };
        assert!(rule.evaluate(&values(&[("disabled", json!(false))])));
        assert!(!rule.evaluate(&values(&[("disabled", json!(true))])));
    }

    #[test]
    fn set_predicate() {
        let rule = Rule::Set {
            field: "name".into(),
        };
        assert!(rule.evaluate(&values(&[("name", json!("Alice"))])));
        assert!(!rule.evaluate(&values(&[("name", json!(""))])));
        assert!(!rule.evaluate(&values(&[("name", json!(null))])));
        assert!(!rule.evaluate(&values(&[])));
    }

    #[test]
    fn empty_predicate() {
        let rule = Rule::Empty {
            field: "name".into(),
        };
        assert!(rule.evaluate(&values(&[])));
        assert!(rule.evaluate(&values(&[("name", json!(null))])));
        assert!(rule.evaluate(&values(&[("name", json!(""))])));
        assert!(!rule.evaluate(&values(&[("name", json!("Alice"))])));
    }

    #[test]
    fn contains_string_predicate() {
        let rule = Rule::Contains {
            field: "tags".into(),
            value: json!("rust"),
        };
        assert!(rule.evaluate(&values(&[("tags", json!("I love rust"))])));
        assert!(!rule.evaluate(&values(&[("tags", json!("I love go"))])));
    }

    #[test]
    fn contains_array_predicate() {
        let rule = Rule::Contains {
            field: "tags".into(),
            value: json!("rust"),
        };
        assert!(rule.evaluate(&values(&[("tags", json!(["rust", "go"]))])));
        assert!(!rule.evaluate(&values(&[("tags", json!(["python"]))])));
    }

    #[test]
    fn in_predicate() {
        let rule = Rule::In {
            field: "role".into(),
            values: vec![json!("admin"), json!("editor")],
        };
        assert!(rule.evaluate(&values(&[("role", json!("admin"))])));
        assert!(!rule.evaluate(&values(&[("role", json!("viewer"))])));
    }

    #[test]
    fn matches_predicate() {
        let rule = Rule::Matches {
            field: "email".into(),
            pattern: r"^[^@]+@[^@]+$".into(),
        };
        assert!(rule.evaluate(&values(&[("email", json!("a@b.com"))])));
        assert!(!rule.evaluate(&values(&[("email", json!("invalid"))])));
    }

    // ── Logical combinators ─────────────────────────────────────────────

    #[test]
    fn all_combinator() {
        let rule = Rule::All {
            rules: vec![
                Rule::MinLength {
                    min: 3,
                    message: None,
                },
                Rule::MaxLength {
                    max: 10,
                    message: None,
                },
            ],
        };
        assert!(rule.validate_value(&json!("hello")).is_ok());
        assert!(rule.validate_value(&json!("ab")).is_err());
        assert!(rule.validate_value(&json!("hello world!")).is_err());
    }

    #[test]
    fn any_combinator() {
        let rule = Rule::Any {
            rules: vec![
                Rule::Eq {
                    field: "a".into(),
                    value: json!(1),
                },
                Rule::Eq {
                    field: "b".into(),
                    value: json!(2),
                },
            ],
        };
        assert!(rule.evaluate(&values(&[("a", json!(1))])));
        assert!(rule.evaluate(&values(&[("b", json!(2))])));
        assert!(!rule.evaluate(&values(&[("a", json!(9)), ("b", json!(9))])));
    }

    #[test]
    fn not_combinator_predicate() {
        let rule = Rule::Not {
            inner: Box::new(Rule::Eq {
                field: "x".into(),
                value: json!(0),
            }),
        };
        assert!(rule.evaluate(&values(&[("x", json!(1))])));
        assert!(!rule.evaluate(&values(&[("x", json!(0))])));
    }

    #[test]
    fn not_combinator_value() {
        let rule = Rule::Not {
            inner: Box::new(Rule::MinLength {
                min: 5,
                message: None,
            }),
        };
        assert!(rule.validate_value(&json!("ab")).is_ok()); // MinLength fails → Not passes
        assert!(rule.validate_value(&json!("hello")).is_err()); // MinLength passes → Not fails
    }

    // ── Serde roundtrip ─────────────────────────────────────────────────

    #[test]
    fn serde_roundtrip_value_rule() {
        let rule = Rule::MinLength {
            min: 3,
            message: Some("too short".into()),
        };
        let json = serde_json::to_value(&rule).unwrap();
        assert_eq!(json["rule"], "min_length");
        assert_eq!(json["min"], 3);
        assert_eq!(json["message"], "too short");

        let back: Rule = serde_json::from_value(json).unwrap();
        assert_eq!(back, rule);
    }

    #[test]
    fn serde_roundtrip_predicate() {
        let rule = Rule::Eq {
            field: "status".into(),
            value: json!("active"),
        };
        let json = serde_json::to_value(&rule).unwrap();
        assert_eq!(json["rule"], "eq");

        let back: Rule = serde_json::from_value(json).unwrap();
        assert_eq!(back, rule);
    }

    #[test]
    fn serde_roundtrip_combinator() {
        let rule = Rule::All {
            rules: vec![
                Rule::MinLength {
                    min: 3,
                    message: None,
                },
                Rule::Eq {
                    field: "x".into(),
                    value: json!(1),
                },
            ],
        };
        let json = serde_json::to_value(&rule).unwrap();
        let back: Rule = serde_json::from_value(json).unwrap();
        assert_eq!(back, rule);
    }

    // ── Classification ──────────────────────────────────────────────────

    #[test]
    fn classification() {
        assert!(
            Rule::MinLength {
                min: 1,
                message: None
            }
            .is_value_rule()
        );
        assert!(
            !Rule::MinLength {
                min: 1,
                message: None
            }
            .is_predicate()
        );
        assert!(
            !Rule::MinLength {
                min: 1,
                message: None
            }
            .is_deferred()
        );

        assert!(
            Rule::Eq {
                field: "x".into(),
                value: json!(1)
            }
            .is_predicate()
        );
        assert!(
            !Rule::Eq {
                field: "x".into(),
                value: json!(1)
            }
            .is_value_rule()
        );

        assert!(
            Rule::UniqueBy {
                key: "id".into(),
                message: None
            }
            .is_deferred()
        );
        assert!(
            Rule::Custom {
                expression: "true".into(),
                message: None
            }
            .is_deferred()
        );
    }

    // ── Missing field edge cases ────────────────────────────────────────

    #[test]
    fn eq_missing_field_is_false() {
        let rule = Rule::Eq {
            field: "x".into(),
            value: json!(1),
        };
        assert!(!rule.evaluate(&values(&[])));
    }

    #[test]
    fn ne_missing_field_is_true() {
        let rule = Rule::Ne {
            field: "x".into(),
            value: json!(1),
        };
        // Missing field can't equal value, so Ne is true.
        assert!(rule.evaluate(&values(&[])));
    }

    #[test]
    fn gt_missing_field_is_false() {
        let rule = Rule::Gt {
            field: "x".into(),
            value: serde_json::Number::from(0),
        };
        assert!(!rule.evaluate(&values(&[])));
    }

    #[test]
    fn gt_non_numeric_field_is_false() {
        let rule = Rule::Gt {
            field: "x".into(),
            value: serde_json::Number::from(0),
        };
        assert!(!rule.evaluate(&values(&[("x", json!("text"))])));
    }

    #[test]
    fn gte_missing_field_is_false() {
        let rule = Rule::Gte {
            field: "x".into(),
            value: serde_json::Number::from(0),
        };
        assert!(!rule.evaluate(&values(&[])));
    }

    #[test]
    fn lt_missing_field_is_false() {
        let rule = Rule::Lt {
            field: "x".into(),
            value: serde_json::Number::from(0),
        };
        assert!(!rule.evaluate(&values(&[])));
    }

    #[test]
    fn lte_missing_field_is_false() {
        let rule = Rule::Lte {
            field: "x".into(),
            value: serde_json::Number::from(0),
        };
        assert!(!rule.evaluate(&values(&[])));
    }

    #[test]
    fn is_true_missing_field_is_false() {
        let rule = Rule::IsTrue { field: "x".into() };
        assert!(!rule.evaluate(&values(&[])));
    }

    #[test]
    fn is_true_non_bool_is_false() {
        let rule = Rule::IsTrue { field: "x".into() };
        assert!(!rule.evaluate(&values(&[("x", json!(1))])));
    }

    #[test]
    fn is_false_missing_field_is_false() {
        let rule = Rule::IsFalse { field: "x".into() };
        assert!(!rule.evaluate(&values(&[])));
    }

    #[test]
    fn is_false_non_bool_is_false() {
        let rule = Rule::IsFalse { field: "x".into() };
        assert!(!rule.evaluate(&values(&[("x", json!(0))])));
    }

    #[test]
    fn set_with_number_is_true() {
        let rule = Rule::Set { field: "x".into() };
        assert!(rule.evaluate(&values(&[("x", json!(0))])));
    }

    #[test]
    fn set_with_empty_array_is_false() {
        let rule = Rule::Set { field: "x".into() };
        assert!(!rule.evaluate(&values(&[("x", json!([]))])));
    }

    #[test]
    fn empty_with_empty_array_is_true() {
        let rule = Rule::Empty { field: "x".into() };
        assert!(rule.evaluate(&values(&[("x", json!([]))])));
    }

    #[test]
    fn empty_with_non_empty_array_is_false() {
        let rule = Rule::Empty { field: "x".into() };
        assert!(!rule.evaluate(&values(&[("x", json!([1]))])));
    }

    #[test]
    fn empty_with_number_is_false() {
        let rule = Rule::Empty { field: "x".into() };
        assert!(!rule.evaluate(&values(&[("x", json!(0))])));
    }

    #[test]
    fn contains_non_string_non_array_is_false() {
        let rule = Rule::Contains {
            field: "x".into(),
            value: json!(1),
        };
        assert!(!rule.evaluate(&values(&[("x", json!(42))])));
    }

    #[test]
    fn contains_missing_field_is_false() {
        let rule = Rule::Contains {
            field: "x".into(),
            value: json!("a"),
        };
        assert!(!rule.evaluate(&values(&[])));
    }

    #[test]
    fn matches_missing_field_is_false() {
        let rule = Rule::Matches {
            field: "x".into(),
            pattern: ".*".into(),
        };
        assert!(!rule.evaluate(&values(&[])));
    }

    #[test]
    fn matches_invalid_regex_is_false() {
        let rule = Rule::Matches {
            field: "x".into(),
            pattern: "[invalid".into(),
        };
        assert!(!rule.evaluate(&values(&[("x", json!("anything"))])));
    }

    #[test]
    fn in_missing_field_is_false() {
        let rule = Rule::In {
            field: "x".into(),
            values: vec![json!(1), json!(2)],
        };
        assert!(!rule.evaluate(&values(&[])));
    }

    // ── Numeric predicate with floats ───────────────────────────────────

    #[expect(
        clippy::approx_constant,
        reason = "3.14 is a representative float literal, not an approximation of π"
    )]
    #[test]
    fn gt_float_comparison() {
        let rule = Rule::Gt {
            field: "val".into(),
            value: serde_json::Number::from_f64(3.14).unwrap(),
        };
        assert!(rule.evaluate(&values(&[("val", json!(3.15))])));
        assert!(!rule.evaluate(&values(&[("val", json!(3.14))])));
        assert!(!rule.evaluate(&values(&[("val", json!(3.13))])));
    }

    // ── Value rules return true in evaluate ─────────────────────────────

    #[test]
    fn value_rule_evaluate_returns_true() {
        let rule = Rule::MinLength {
            min: 100,
            message: None,
        };
        // Value rules are vacuously true when used as predicates.
        assert!(rule.evaluate(&values(&[])));
    }

    #[test]
    fn deferred_rule_evaluate_returns_true() {
        let rule = Rule::Custom {
            expression: "false".into(),
            message: None,
        };
        assert!(rule.evaluate(&values(&[])));
    }

    // ── Predicates return Ok in validate_value ──────────────────────────

    #[test]
    fn predicate_validate_value_returns_ok() {
        let rule = Rule::Eq {
            field: "x".into(),
            value: json!(1),
        };
        assert!(rule.validate_value(&json!("anything")).is_ok());
    }

    // ── Value validation edge cases ─────────────────────────────────────

    #[test]
    fn pattern_invalid_regex_returns_error() {
        let rule = Rule::Pattern {
            pattern: "[invalid".into(),
            message: None,
        };
        let err = rule.validate_value(&json!("test")).unwrap_err();
        assert_eq!(err.code.as_ref(), "invalid_pattern");
    }

    #[expect(
        clippy::approx_constant,
        reason = "3.14 is a representative float literal, not an approximation of π"
    )]
    #[test]
    fn min_float_boundary() {
        let rule = Rule::Min {
            min: serde_json::Number::from_f64(3.14).unwrap(),
            message: None,
        };
        assert!(rule.validate_value(&json!(3.14)).is_ok());
        assert!(rule.validate_value(&json!(3.15)).is_ok());
        assert!(rule.validate_value(&json!(3.13)).is_err());
    }

    #[test]
    fn max_float_boundary() {
        let rule = Rule::Max {
            max: serde_json::Number::from_f64(9.99).unwrap(),
            message: None,
        };
        assert!(rule.validate_value(&json!(9.99)).is_ok());
        assert!(rule.validate_value(&json!(9.98)).is_ok());
        assert!(rule.validate_value(&json!(10.0)).is_err());
    }

    #[test]
    fn min_on_non_number_is_ok() {
        let rule = Rule::Min {
            min: serde_json::Number::from(5),
            message: None,
        };
        assert!(rule.validate_value(&json!("text")).is_ok());
    }

    #[test]
    fn max_on_non_number_is_ok() {
        let rule = Rule::Max {
            max: serde_json::Number::from(5),
            message: None,
        };
        assert!(rule.validate_value(&json!(null)).is_ok());
    }

    #[test]
    fn one_of_with_mixed_types() {
        let rule = Rule::OneOf {
            values: vec![json!(1), json!("yes"), json!(true)],
            message: None,
        };
        assert!(rule.validate_value(&json!(1)).is_ok());
        assert!(rule.validate_value(&json!("yes")).is_ok());
        assert!(rule.validate_value(&json!(true)).is_ok());
        assert!(rule.validate_value(&json!(false)).is_err());
    }

    #[test]
    fn one_of_custom_message() {
        let rule = Rule::OneOf {
            values: vec![json!("a")],
            message: Some("pick something valid".into()),
        };
        let err = rule.validate_value(&json!("z")).unwrap_err();
        assert_eq!(err.message.as_ref(), "pick something valid");
    }

    #[test]
    fn min_items_on_non_array_is_ok() {
        let rule = Rule::MinItems {
            min: 5,
            message: None,
        };
        assert!(rule.validate_value(&json!("not an array")).is_ok());
    }

    #[test]
    fn max_items_on_non_array_is_ok() {
        let rule = Rule::MaxItems {
            max: 1,
            message: None,
        };
        assert!(rule.validate_value(&json!(42)).is_ok());
    }

    #[test]
    fn min_length_on_null_is_ok() {
        let rule = Rule::MinLength {
            min: 3,
            message: None,
        };
        assert!(rule.validate_value(&json!(null)).is_ok());
    }

    #[test]
    fn max_length_custom_message() {
        let rule = Rule::MaxLength {
            max: 3,
            message: Some("way too long".into()),
        };
        let err = rule.validate_value(&json!("hello")).unwrap_err();
        assert_eq!(err.message.as_ref(), "way too long");
    }

    #[test]
    fn pattern_custom_message() {
        let rule = Rule::Pattern {
            pattern: "^[0-9]+$".into(),
            message: Some("digits only!".into()),
        };
        let err = rule.validate_value(&json!("abc")).unwrap_err();
        assert_eq!(err.message.as_ref(), "digits only!");
    }

    #[test]
    fn min_items_exact_boundary() {
        let rule = Rule::MinItems {
            min: 2,
            message: None,
        };
        assert!(rule.validate_value(&json!([1, 2])).is_ok());
        assert!(rule.validate_value(&json!([1])).is_err());
    }

    #[test]
    fn max_items_exact_boundary() {
        let rule = Rule::MaxItems {
            max: 2,
            message: None,
        };
        assert!(rule.validate_value(&json!([1, 2])).is_ok());
        assert!(rule.validate_value(&json!([1, 2, 3])).is_err());
    }

    #[test]
    fn validate_null_value_passes_all_value_rules() {
        let rules = vec![
            Rule::MinLength {
                min: 1,
                message: None,
            },
            Rule::MaxLength {
                max: 1,
                message: None,
            },
            Rule::Pattern {
                pattern: "^x$".into(),
                message: None,
            },
            Rule::Min {
                min: serde_json::Number::from(1),
                message: None,
            },
            Rule::Max {
                max: serde_json::Number::from(1),
                message: None,
            },
            Rule::MinItems {
                min: 1,
                message: None,
            },
            Rule::MaxItems {
                max: 1,
                message: None,
            },
        ];
        for rule in &rules {
            assert!(
                rule.validate_value(&json!(null)).is_ok(),
                "rule {:?} should pass on null",
                rule
            );
        }
    }

    // ── Combinator edge cases ───────────────────────────────────────────

    #[test]
    fn all_empty_rules_passes() {
        let rule = Rule::All { rules: vec![] };
        assert!(rule.validate_value(&json!("anything")).is_ok());
    }

    #[test]
    fn any_empty_rules_passes() {
        let rule = Rule::Any { rules: vec![] };
        assert!(rule.validate_value(&json!("anything")).is_ok());
    }

    #[test]
    fn all_with_single_failing_rule() {
        let rule = Rule::All {
            rules: vec![
                Rule::MinLength {
                    min: 1,
                    message: None,
                },
                Rule::MaxLength {
                    max: 3,
                    message: None,
                },
                Rule::Pattern {
                    pattern: "^[0-9]+$".into(),
                    message: None,
                },
            ],
        };
        // "ab" passes MinLength and MaxLength but fails Pattern
        assert!(rule.validate_value(&json!("ab")).is_err());
    }

    #[test]
    fn any_first_passes() {
        let rule = Rule::Any {
            rules: vec![
                Rule::MinLength {
                    min: 1,
                    message: None,
                },
                Rule::MinLength {
                    min: 100,
                    message: None,
                },
            ],
        };
        assert!(rule.validate_value(&json!("hello")).is_ok());
    }

    #[test]
    fn any_last_passes() {
        let rule = Rule::Any {
            rules: vec![
                Rule::MinLength {
                    min: 100,
                    message: None,
                },
                Rule::MinLength {
                    min: 1,
                    message: None,
                },
            ],
        };
        assert!(rule.validate_value(&json!("hello")).is_ok());
    }

    #[test]
    fn nested_combinators() {
        // All(Any(MinLength(10), MaxLength(3)), Pattern(^[a-z]+$))
        let rule = Rule::All {
            rules: vec![
                Rule::Any {
                    rules: vec![
                        Rule::MinLength {
                            min: 10,
                            message: None,
                        },
                        Rule::MaxLength {
                            max: 3,
                            message: None,
                        },
                    ],
                },
                Rule::Pattern {
                    pattern: "^[a-z]+$".into(),
                    message: None,
                },
            ],
        };
        // "ab" → Any(MinLength(10) fails, MaxLength(3) passes) → ok; Pattern passes → ok
        assert!(rule.validate_value(&json!("ab")).is_ok());
        // "AB" → Any passes, but Pattern fails
        assert!(rule.validate_value(&json!("AB")).is_err());
        // "abcde" → Any(MinLength fails, MaxLength fails) → fails
        assert!(rule.validate_value(&json!("abcde")).is_err());
    }

    #[test]
    fn not_with_not_double_negation() {
        let rule = Rule::Not {
            inner: Box::new(Rule::Not {
                inner: Box::new(Rule::MinLength {
                    min: 3,
                    message: None,
                }),
            }),
        };
        // Double negation: Not(Not(MinLength(3))) == MinLength(3)
        assert!(rule.validate_value(&json!("hello")).is_ok());
        assert!(rule.validate_value(&json!("ab")).is_err());
    }

    #[test]
    fn all_evaluate_with_predicates() {
        let rule = Rule::All {
            rules: vec![
                Rule::Eq {
                    field: "a".into(),
                    value: json!(1),
                },
                Rule::Set { field: "b".into() },
            ],
        };
        assert!(rule.evaluate(&values(&[("a", json!(1)), ("b", json!("x"))])));
        assert!(!rule.evaluate(&values(&[("a", json!(1))])));
        assert!(!rule.evaluate(&values(&[("b", json!("x"))])));
    }

    #[test]
    fn any_evaluate_with_predicates() {
        let rule = Rule::Any {
            rules: vec![
                Rule::Eq {
                    field: "a".into(),
                    value: json!(1),
                },
                Rule::Set { field: "b".into() },
            ],
        };
        assert!(rule.evaluate(&values(&[("a", json!(1))])));
        assert!(rule.evaluate(&values(&[("b", json!("x"))])));
        assert!(!rule.evaluate(&values(&[])));
    }

    // ── Serde all predicate variants ────────────────────────────────────

    #[test]
    fn serde_roundtrip_gt() {
        let rule = Rule::Gt {
            field: "x".into(),
            value: serde_json::Number::from(5),
        };
        let json = serde_json::to_value(&rule).unwrap();
        assert_eq!(json["rule"], "gt");
        let back: Rule = serde_json::from_value(json).unwrap();
        assert_eq!(back, rule);
    }

    #[test]
    fn serde_roundtrip_gte() {
        let rule = Rule::Gte {
            field: "x".into(),
            value: serde_json::Number::from(5),
        };
        let json = serde_json::to_value(&rule).unwrap();
        assert_eq!(json["rule"], "gte");
        let back: Rule = serde_json::from_value(json).unwrap();
        assert_eq!(back, rule);
    }

    #[test]
    fn serde_roundtrip_lt() {
        let rule = Rule::Lt {
            field: "x".into(),
            value: serde_json::Number::from(5),
        };
        let json = serde_json::to_value(&rule).unwrap();
        assert_eq!(json["rule"], "lt");
        let back: Rule = serde_json::from_value(json).unwrap();
        assert_eq!(back, rule);
    }

    #[test]
    fn serde_roundtrip_lte() {
        let rule = Rule::Lte {
            field: "x".into(),
            value: serde_json::Number::from(5),
        };
        let json = serde_json::to_value(&rule).unwrap();
        assert_eq!(json["rule"], "lte");
        let back: Rule = serde_json::from_value(json).unwrap();
        assert_eq!(back, rule);
    }

    #[test]
    fn serde_roundtrip_is_true() {
        let rule = Rule::IsTrue { field: "x".into() };
        let json = serde_json::to_value(&rule).unwrap();
        assert_eq!(json["rule"], "is_true");
        let back: Rule = serde_json::from_value(json).unwrap();
        assert_eq!(back, rule);
    }

    #[test]
    fn serde_roundtrip_is_false() {
        let rule = Rule::IsFalse { field: "x".into() };
        let json = serde_json::to_value(&rule).unwrap();
        assert_eq!(json["rule"], "is_false");
        let back: Rule = serde_json::from_value(json).unwrap();
        assert_eq!(back, rule);
    }

    #[test]
    fn serde_roundtrip_set() {
        let rule = Rule::Set { field: "x".into() };
        let json = serde_json::to_value(&rule).unwrap();
        assert_eq!(json["rule"], "set");
        let back: Rule = serde_json::from_value(json).unwrap();
        assert_eq!(back, rule);
    }

    #[test]
    fn serde_roundtrip_empty() {
        let rule = Rule::Empty { field: "x".into() };
        let json = serde_json::to_value(&rule).unwrap();
        assert_eq!(json["rule"], "empty");
        let back: Rule = serde_json::from_value(json).unwrap();
        assert_eq!(back, rule);
    }

    #[test]
    fn serde_roundtrip_contains() {
        let rule = Rule::Contains {
            field: "tags".into(),
            value: json!("rust"),
        };
        let json = serde_json::to_value(&rule).unwrap();
        assert_eq!(json["rule"], "contains");
        let back: Rule = serde_json::from_value(json).unwrap();
        assert_eq!(back, rule);
    }

    #[test]
    fn serde_roundtrip_matches() {
        let rule = Rule::Matches {
            field: "email".into(),
            pattern: r"^[^@]+@[^@]+$".into(),
        };
        let json = serde_json::to_value(&rule).unwrap();
        assert_eq!(json["rule"], "matches");
        let back: Rule = serde_json::from_value(json).unwrap();
        assert_eq!(back, rule);
    }

    #[test]
    fn serde_roundtrip_in() {
        let rule = Rule::In {
            field: "role".into(),
            values: vec![json!("admin"), json!("editor")],
        };
        let json = serde_json::to_value(&rule).unwrap();
        assert_eq!(json["rule"], "in");
        let back: Rule = serde_json::from_value(json).unwrap();
        assert_eq!(back, rule);
    }

    #[test]
    fn serde_roundtrip_ne() {
        let rule = Rule::Ne {
            field: "status".into(),
            value: json!("deleted"),
        };
        let json = serde_json::to_value(&rule).unwrap();
        assert_eq!(json["rule"], "ne");
        let back: Rule = serde_json::from_value(json).unwrap();
        assert_eq!(back, rule);
    }

    #[test]
    fn serde_roundtrip_not() {
        let rule = Rule::Not {
            inner: Box::new(Rule::Eq {
                field: "x".into(),
                value: json!(1),
            }),
        };
        let json = serde_json::to_value(&rule).unwrap();
        assert_eq!(json["rule"], "not");
        let back: Rule = serde_json::from_value(json).unwrap();
        assert_eq!(back, rule);
    }

    #[test]
    fn serde_roundtrip_any() {
        let rule = Rule::Any {
            rules: vec![
                Rule::Eq {
                    field: "a".into(),
                    value: json!(1),
                },
                Rule::IsTrue { field: "b".into() },
            ],
        };
        let json = serde_json::to_value(&rule).unwrap();
        assert_eq!(json["rule"], "any");
        let back: Rule = serde_json::from_value(json).unwrap();
        assert_eq!(back, rule);
    }

    #[test]
    fn serde_roundtrip_unique_by() {
        let rule = Rule::UniqueBy {
            key: "id".into(),
            message: Some("must be unique".into()),
        };
        let json = serde_json::to_value(&rule).unwrap();
        assert_eq!(json["rule"], "unique_by");
        let back: Rule = serde_json::from_value(json).unwrap();
        assert_eq!(back, rule);
    }

    #[test]
    fn serde_roundtrip_custom() {
        let rule = Rule::Custom {
            expression: "len(items) > 0".into(),
            message: None,
        };
        let json = serde_json::to_value(&rule).unwrap();
        assert_eq!(json["rule"], "custom");
        let back: Rule = serde_json::from_value(json).unwrap();
        assert_eq!(back, rule);
    }

    #[test]
    fn serde_deserialize_from_json_string() {
        let json_str = r#"{"rule":"min_length","min":5}"#;
        let rule: Rule = serde_json::from_str(json_str).unwrap();
        assert_eq!(
            rule,
            Rule::MinLength {
                min: 5,
                message: None
            }
        );
    }

    #[test]
    fn serde_deserialize_nested_combinator() {
        let json_str = r#"{
            "rule": "all",
            "rules": [
                {"rule": "min_length", "min": 3},
                {"rule": "not", "inner": {"rule": "eq", "field": "x", "value": 1}}
            ]
        }"#;
        let rule: Rule = serde_json::from_str(json_str).unwrap();
        match rule {
            Rule::All { rules } => assert_eq!(rules.len(), 2),
            other => panic!("expected All, got {other:?}"),
        }
    }

    // ── Classification completeness ─────────────────────────────────────

    #[test]
    fn all_combinators_are_neither_value_nor_predicate_nor_deferred() {
        let combinators = vec![
            Rule::All { rules: vec![] },
            Rule::Any { rules: vec![] },
            Rule::Not {
                inner: Box::new(Rule::IsTrue { field: "x".into() }),
            },
        ];
        for c in &combinators {
            assert!(!c.is_value_rule(), "{c:?} should not be value_rule");
            assert!(!c.is_predicate(), "{c:?} should not be predicate");
            assert!(!c.is_deferred(), "{c:?} should not be deferred");
        }
    }

    #[test]
    fn all_value_rules_are_not_predicates() {
        let value_rules = vec![
            Rule::Pattern {
                pattern: "x".into(),
                message: None,
            },
            Rule::MinLength {
                min: 1,
                message: None,
            },
            Rule::MaxLength {
                max: 1,
                message: None,
            },
            Rule::Min {
                min: serde_json::Number::from(1),
                message: None,
            },
            Rule::Max {
                max: serde_json::Number::from(1),
                message: None,
            },
            Rule::OneOf {
                values: vec![],
                message: None,
            },
            Rule::MinItems {
                min: 1,
                message: None,
            },
            Rule::MaxItems {
                max: 1,
                message: None,
            },
        ];
        for r in &value_rules {
            assert!(r.is_value_rule(), "{r:?} should be value_rule");
            assert!(!r.is_predicate(), "{r:?} should not be predicate");
        }
    }

    #[test]
    fn all_predicates_are_not_value_rules() {
        let predicates = vec![
            Rule::Eq {
                field: "x".into(),
                value: json!(1),
            },
            Rule::Ne {
                field: "x".into(),
                value: json!(1),
            },
            Rule::Gt {
                field: "x".into(),
                value: serde_json::Number::from(1),
            },
            Rule::Gte {
                field: "x".into(),
                value: serde_json::Number::from(1),
            },
            Rule::Lt {
                field: "x".into(),
                value: serde_json::Number::from(1),
            },
            Rule::Lte {
                field: "x".into(),
                value: serde_json::Number::from(1),
            },
            Rule::IsTrue { field: "x".into() },
            Rule::IsFalse { field: "x".into() },
            Rule::Set { field: "x".into() },
            Rule::Empty { field: "x".into() },
            Rule::Contains {
                field: "x".into(),
                value: json!(1),
            },
            Rule::Matches {
                field: "x".into(),
                pattern: "x".into(),
            },
            Rule::In {
                field: "x".into(),
                values: vec![],
            },
        ];
        for r in &predicates {
            assert!(r.is_predicate(), "{r:?} should be predicate");
            assert!(!r.is_value_rule(), "{r:?} should not be value_rule");
            assert!(!r.is_deferred(), "{r:?} should not be deferred");
        }
    }

    // ── Shorthand constructors ───────────────────────────────────────────

    #[test]
    fn shorthand_min_length() {
        let rule = Rule::min_length(5);
        assert!(matches!(rule, Rule::MinLength { min: 5, message: None }));
    }

    #[test]
    fn shorthand_pattern() {
        let rule = Rule::pattern(r"^\d+$");
        if let Rule::Pattern { pattern, message } = &rule {
            assert_eq!(pattern, r"^\d+$");
            assert!(message.is_none());
        } else {
            panic!("expected Pattern");
        }
    }

    #[test]
    fn shorthand_with_message() {
        let rule = Rule::min_length(3).with_message("Too short");
        assert!(matches!(
            rule,
            Rule::MinLength {
                min: 3,
                message: Some(ref m),
            } if m == "Too short"
        ));
    }

    #[test]
    fn shorthand_one_of() {
        let rule = Rule::one_of(["a", "b", "c"]);
        if let Rule::OneOf { values, message } = &rule {
            assert_eq!(values.len(), 3);
            assert!(message.is_none());
        } else {
            panic!("expected OneOf");
        }
    }

    #[test]
    fn shorthand_min_max_value() {
        let min = Rule::min_value(0);
        let max = Rule::max_value(100);
        assert!(matches!(min, Rule::Min { .. }));
        assert!(matches!(max, Rule::Max { .. }));
    }
}

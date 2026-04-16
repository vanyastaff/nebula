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
//! | Value validation | `MinLength`, `MaxLength`, `Pattern`, `Min`, `Max`, `OneOf`, `MinItems`, `MaxItems`, `Email`, `Url` | [`Rule::validate_value`] | [`Rule::is_value_rule`] |
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
//! let rule = Rule::MinLength {
//!     min: 3,
//!     message: None,
//! };
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
//!         Rule::MinLength {
//!             min: 3,
//!             message: None,
//!         },
//!         Rule::MaxLength {
//!             max: 20,
//!             message: None,
//!         },
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

mod classify;
mod constructors;
mod evaluate;
mod helpers;
mod validate;

#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};

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

    /// Value must be a valid email address.
    Email {
        /// Optional custom error message.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    /// Value must be a valid URL (http/https).
    Url {
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

impl crate::foundation::Validate<serde_json::Value> for Rule {
    fn validate(
        &self,
        input: &serde_json::Value,
    ) -> Result<(), crate::foundation::ValidationError> {
        self.validate_value(input)
    }
}

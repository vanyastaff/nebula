//! Supporting spec types used by parameter schemas and providers.

use std::fmt;

use crate::loader::OptionLoader;
use crate::option::SelectOption;

/// Simplified parameter subset that dynamic record providers may return.
///
/// Providers must not introduce nested Mode or Dynamic variants.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FieldSpec {
    /// Free-form text.
    Text {
        /// Stable field identifier.
        id: String,
        /// Display label.
        #[serde(default)]
        label: String,
        /// Render as a multi-line textarea.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        multiline: bool,
    },
    /// Number.
    Number {
        /// Stable field identifier.
        id: String,
        /// Display label.
        #[serde(default)]
        label: String,
        /// Restrict input to whole integers.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        integer: bool,
        /// Inclusive lower bound.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<serde_json::Number>,
        /// Inclusive upper bound.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<serde_json::Number>,
        /// Stepper increment for UI controls.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        step: Option<serde_json::Number>,
    },
    /// Boolean toggle.
    Boolean {
        /// Stable field identifier.
        id: String,
        /// Display label.
        #[serde(default)]
        label: String,
    },
    /// Select with static options.
    Select {
        /// Stable field identifier.
        id: String,
        /// Display label.
        #[serde(default)]
        label: String,
        /// Static options.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        options: Vec<SelectOption>,
        /// Allow selecting multiple values.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        multiple: bool,
        /// Allow values not present in the option list.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        allow_custom: bool,
        /// Display a search filter in the option picker.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        searchable: bool,
        /// Inline option loader; skipped during serialization.
        #[serde(skip)]
        loader: Option<OptionLoader>,
    },
}

/// Top-level filter expression emitted by a filter editor.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FilterExpr {
    /// A single field-operator-value assertion.
    Rule(FilterRule),
    /// A logical group combining multiple expressions.
    Group(FilterGroup),
}

/// Logical combinator for a [`FilterGroup`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterCombinator {
    /// All children must pass.
    #[default]
    And,
    /// At least one child must pass.
    Or,
}

/// A logical group of filter expressions.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FilterGroup {
    /// How child expressions are combined.
    pub combinator: FilterCombinator,
    /// Child expressions.
    pub children: Vec<FilterExpr>,
}

/// A single filter assertion.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FilterRule {
    /// Field id the assertion applies to.
    pub field: String,
    /// Comparison operator.
    pub op: FilterOp,
    /// Operand value (absent for unary operators like `is_set`/`is_empty`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
}

/// Comparison operator for a [`FilterRule`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterOp {
    /// Equal.
    Eq,
    /// Not equal.
    Ne,
    /// Greater than.
    Gt,
    /// Greater than or equal.
    Gte,
    /// Less than.
    Lt,
    /// Less than or equal.
    Lte,
    /// Value is in an array of comparands.
    In,
    /// Value is not in an array.
    NotIn,
    /// String or array contains the value.
    Contains,
    /// String matches a regexp.
    Matches,
    /// Field has a non-null/non-empty value.
    IsSet,
    /// Field is null or empty.
    IsEmpty,
}

// ── FieldSpec conversions ───────────────────────────────────────────────────

/// Error returned when a conversion to [`FieldSpec`] is not supported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldSpecConvertError {
    /// The name of the unsupported variant.
    pub variant: String,
}

impl fmt::Display for FieldSpecConvertError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "cannot convert to FieldSpec: unsupported variant `{}`",
            self.variant
        )
    }
}

impl std::error::Error for FieldSpecConvertError {}

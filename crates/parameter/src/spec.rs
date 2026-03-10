//! Supporting spec types used by schema fields and providers.

use crate::loader::OptionLoader;
use crate::metadata::FieldMetadata;
use crate::option::OptionSource;

/// One variant in a [`crate::field::Field::Mode`] discriminated-union field.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ModeVariant {
    /// Stable variant key.
    pub key: String,
    /// Display label.
    pub label: String,
    /// Optional description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Single content field for this variant.
    ///
    /// Use [`crate::field::Field::Object`] to group multiple sub-fields inside one variant.
    pub content: Box<crate::field::Field>,
}

/// Controls when the dynamic fields editor is rendered.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DynamicFieldsMode {
    /// Show all provider fields.
    #[default]
    All,
    /// Show only required provider fields initially.
    RequiredOnly,
}

/// Policy for values returned by a provider but absent from the schema.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnknownFieldPolicy {
    /// Keep unknown values and surface a warning.
    #[default]
    WarnKeep,
    /// Drop unknown values from storage.
    Strip,
    /// Fail validation when unknown values are present.
    Error,
}

/// Simplified field subset that [`crate::providers::DynamicRecordProvider`]s may return.
///
/// Providers must not introduce nested [`crate::field::Field::Mode`] or
/// [`crate::field::Field::DynamicFields`] variants.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FieldSpec {
    /// Free-form text.
    Text {
        /// Shared field metadata.
        #[serde(flatten)]
        meta: FieldMetadata,
        /// Render as a multi-line textarea.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        multiline: bool,
    },
    /// Number.
    Number {
        /// Shared field metadata.
        #[serde(flatten)]
        meta: FieldMetadata,
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
        /// Shared field metadata.
        #[serde(flatten)]
        meta: FieldMetadata,
    },
    /// Select with static or dynamic options.
    Select {
        /// Shared field metadata.
        #[serde(flatten)]
        meta: FieldMetadata,
        /// Option source.
        #[serde(flatten)]
        source: OptionSource,
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

/// Top-level filter expression emitted by a [`crate::field::Field::Filter`] editor.
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

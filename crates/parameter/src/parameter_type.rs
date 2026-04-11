//! Type-specific parameter variants.
//!
//! [`ParameterType`] is a 19-variant enum carrying **only** type-specific data.
//! Shared metadata (label, description, rules, conditions, etc.) lives on
//! [`Parameter`](crate::parameter::Parameter), which wraps this enum via `#[serde(flatten)]`.

use serde::{Deserialize, Serialize};

use crate::{
    display_mode::{ComputedReturn, DisplayMode},
    filter_field::FilterField,
    input_hint::InputHint,
    loader::{FilterFieldLoader, OptionLoader, RecordLoader},
    notice::NoticeSeverity,
    option::SelectOption,
    path::ParameterPath,
    spec::FilterOp,
};

/// Returns `true` for serde defaults.
fn default_true() -> bool {
    true
}

/// Default max nesting depth for filter groups.
fn default_depth() -> u8 {
    3
}

/// Describes the data type and type-specific configuration of a parameter.
///
/// Each variant carries only the fields unique to that parameter kind.
/// Shared metadata (label, description, validation rules, conditions, etc.)
/// is stored on [`super::parameter::Parameter`].
///
/// Serialized with `"type"` as the tag field and `snake_case` variant names:
/// `ParameterType::String { .. }` becomes `{ "type": "string", ... }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ParameterType {
    /// Free-form text input.
    String {
        /// Render as a multi-line textarea.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        multiline: bool,
        /// UI input hint (date picker, color picker, URL input, etc.).
        #[serde(default, skip_serializing_if = "InputHint::is_default")]
        input_hint: InputHint,
    },

    /// Numeric input (integer or floating-point).
    Number {
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
    Boolean,

    /// Single or multi-select from a list of options.
    Select {
        /// Static options displayed in the picker.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        options: Vec<SelectOption>,
        /// Whether options are loaded dynamically at runtime.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        dynamic: bool,
        /// Parameters whose values trigger a reload of options.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        depends_on: Vec<ParameterPath>,
        /// Allow selecting multiple values.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        multiple: bool,
        /// Allow values not present in the option list.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        allow_custom: bool,
        /// Display a search filter in the option picker.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        searchable: bool,
        /// Inline async loader for dynamic options; not serialized.
        #[serde(skip)]
        loader: Option<OptionLoader>,
    },

    /// Nested group of sub-parameters.
    Object {
        /// The child parameters within this object.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        parameters: Vec<super::parameter::Parameter>,
        /// Controls UI presentation of sub-parameters.
        #[serde(default, skip_serializing_if = "DisplayMode::is_default")]
        display_mode: DisplayMode,
    },

    /// Ordered collection of homogeneous items.
    List {
        /// Template parameter defining each list item.
        item: Box<super::parameter::Parameter>,
        /// Minimum number of items required.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min_items: Option<u32>,
        /// Maximum number of items allowed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_items: Option<u32>,
        /// Enforce uniqueness across items.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        unique: bool,
        /// Allow drag-and-drop reordering.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        sortable: bool,
    },

    /// Discriminated union — user picks one variant.
    Mode {
        /// Available variant parameters (each with a unique `id`).
        variants: Vec<super::parameter::Parameter>,
        /// Default variant `id` selected on first render.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default_variant: Option<String>,
    },

    /// Source code editor with syntax highlighting.
    Code {
        /// Programming language for syntax highlighting (e.g. `"json"`, `"python"`).
        language: String,
    },

    /// Date picker (no time component).
    #[deprecated(since = "0.4.0", note = "use String with InputHint::Date")]
    Date,

    /// Date and time picker.
    #[deprecated(since = "0.4.0", note = "use String with InputHint::DateTime")]
    DateTime,

    /// Time-only picker.
    #[deprecated(since = "0.4.0", note = "use String with InputHint::Time")]
    Time,

    /// Color picker.
    #[deprecated(since = "0.4.0", note = "use String with InputHint::Color")]
    Color,

    /// File upload input.
    File {
        /// MIME type filter (e.g. `"image/*"`, `"application/pdf"`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        accept: Option<String>,
        /// Maximum file size in bytes.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_size: Option<u64>,
        /// Allow uploading multiple files.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        multiple: bool,
    },

    /// Hidden parameter — not rendered in the UI.
    #[deprecated(since = "0.4.0", note = "set visible = false on Parameter instead")]
    Hidden,

    /// Complex filter/query builder.
    Filter {
        /// Allowed comparison operators; `None` means all operators.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        operators: Option<Vec<FilterOp>>,
        /// Whether logical grouping (AND/OR) is allowed.
        #[serde(default = "default_true")]
        allow_groups: bool,
        /// Maximum nesting depth for filter groups.
        #[serde(default = "default_depth")]
        max_depth: u8,
        /// Static filter field definitions.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        fields: Vec<FilterField>,
        /// Whether filter fields are loaded dynamically at runtime.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        dynamic_fields: bool,
        /// Parameters whose values trigger a reload of filter fields.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        depends_on: Vec<ParameterPath>,
        /// Inline async loader for dynamic filter fields; not serialized.
        #[serde(skip)]
        fields_loader: Option<FilterFieldLoader>,
    },

    /// Derived value computed from an expression.
    Computed {
        /// The expression to evaluate (e.g. `"{{first_name}} {{last_name}}"`).
        expression: String,
        /// The data type of the computed result.
        returns: ComputedReturn,
    },

    /// Dynamic parameter whose schema is resolved at runtime.
    Dynamic {
        /// Parameters whose values trigger a reload of the dynamic schema.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        depends_on: Vec<ParameterPath>,
        /// Inline async loader for dynamic records; not serialized.
        #[serde(skip)]
        loader: Option<RecordLoader>,
    },

    /// Display-only informational notice (no user input).
    Notice {
        /// Severity level controlling the visual style.
        #[serde(default, skip_serializing_if = "NoticeSeverity::is_default")]
        severity: NoticeSeverity,
    },
}

impl ParameterType {
    /// Returns the variant name as a static string.
    #[must_use]
    #[allow(deprecated)]
    pub fn variant_name(&self) -> &'static str {
        match self {
            Self::String { .. } => "String",
            Self::Number { .. } => "Number",
            Self::Boolean => "Boolean",
            Self::Select { .. } => "Select",
            Self::Object { .. } => "Object",
            Self::List { .. } => "List",
            Self::Mode { .. } => "Mode",
            Self::Code { .. } => "Code",
            Self::Date => "Date",
            Self::DateTime => "DateTime",
            Self::Time => "Time",
            Self::Color => "Color",
            Self::File { .. } => "File",
            Self::Hidden => "Hidden",
            Self::Filter { .. } => "Filter",
            Self::Computed { .. } => "Computed",
            Self::Dynamic { .. } => "Dynamic",
            Self::Notice { .. } => "Notice",
        }
    }
}

//! Core parameter definition — the v3 replacement for the v2 `Field` +
//! `FieldMetadata` types.
//!
//! A [`Parameter`] combines shared metadata (label, description, conditions,
//! validation rules, transformers) with a type-specific [`ParameterType`] variant.
//! Fluent builder methods make schema definitions concise and readable.
//!
//! # Examples
//!
//! ```
//! use nebula_parameter::parameter::Parameter;
//! use nebula_parameter::conditions::Condition;
//!
//! let schema = vec![
//!     Parameter::string("api_key").label("API Key").required().secret(),
//!     Parameter::integer("timeout_ms").label("Timeout (ms)").default(serde_json::json!(30_000)),
//!     Parameter::select("region")
//!         .label("Region")
//!         .option(serde_json::json!("us-east-1"), "US East")
//!         .option(serde_json::json!("eu-west-1"), "EU West")
//!         .searchable(),
//! ];
//!
//! assert_eq!(schema.len(), 3);
//! ```

use std::future::Future;

use serde::{Deserialize, Serialize};

use crate::conditions::Condition;
use crate::display_mode::{ComputedReturn, DisplayMode};
use crate::filter_field::FilterField;
use crate::input_hint::InputHint;
use crate::loader::{FilterFieldLoader, LoaderContext, LoaderError, OptionLoader, RecordLoader};
use crate::loader_result::LoaderResult;
use crate::notice::NoticeSeverity;
use crate::option::SelectOption;
use crate::parameter_type::ParameterType;
use crate::path::ParameterPath;
use crate::rules::Rule;
use crate::spec::FilterOp;
use crate::transformer::Transformer;

/// A single parameter in a workflow node's schema.
///
/// Combines shared metadata (label, description, validation rules, conditions,
/// transformers, etc.) with a type-specific [`ParameterType`] variant via
/// `#[serde(flatten)]`.
///
/// Use the named constructors ([`Parameter::string`], [`Parameter::select`], etc.)
/// followed by fluent builder methods to define parameters concisely.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Parameter {
    /// Stable identifier for this parameter within its schema.
    pub id: String,

    /// The type-specific configuration for this parameter.
    #[serde(flatten)]
    pub param_type: ParameterType,

    /// Human-readable display label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,

    /// Longer description or help text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Placeholder text shown when the field is empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,

    /// Inline hint displayed below the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,

    /// Default value used when no user input is provided.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,

    /// Whether a value must be provided.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub required: bool,

    /// Whether the value should be masked in the UI and encrypted at rest.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub secret: bool,

    /// Whether the field supports expression mode (e.g. `{{ variable }}`).
    ///
    /// Defaults to `true` — most fields support expressions. Use
    /// `.no_expression()` to disable for fields like boolean toggles.
    #[serde(default = "default_true")]
    pub expression: bool,

    /// Override the HTML input type (e.g. `"email"`, `"url"`).
    #[deprecated(since = "0.4.0", note = "use InputHint on ParameterType::String instead")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_type: Option<String>,

    /// Validation rules applied to the parameter value.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<Rule>,

    /// Condition that controls when this parameter is visible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_when: Option<Condition>,

    /// Condition that controls when this parameter is required.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_when: Option<Condition>,

    /// Condition that controls when this parameter is disabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled_when: Option<Condition>,

    /// Value transformers applied before consumption.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transformers: Vec<Transformer>,

    /// Grouping key for UI sectioning (used with [`DisplayMode::Sections`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

// ── Private helper ─────────────────────────────────────────────────────────

fn default_true() -> bool {
    true
}

/// Creates a [`Parameter`] with all shared metadata at defaults.
#[allow(deprecated)]
fn new_parameter(id: impl Into<String>, param_type: ParameterType) -> Parameter {
    Parameter {
        id: id.into(),
        param_type,
        label: None,
        description: None,
        placeholder: None,
        hint: None,
        default: None,
        required: false,
        secret: false,
        expression: true,
        input_type: None,
        rules: Vec::new(),
        visible_when: None,
        required_when: None,
        disabled_when: None,
        transformers: Vec::new(),
        group: None,
    }
}

// ── Constructors ───────────────────────────────────────────────────────────

impl Parameter {
    /// Creates a free-form text parameter.
    #[must_use]
    pub fn string(id: impl Into<String>) -> Self {
        new_parameter(
            id,
            ParameterType::String {
                multiline: false,
                input_hint: InputHint::default(),
            },
        )
    }

    /// Creates a numeric parameter (floating-point by default).
    #[must_use]
    pub fn number(id: impl Into<String>) -> Self {
        new_parameter(
            id,
            ParameterType::Number {
                integer: false,
                min: None,
                max: None,
                step: None,
            },
        )
    }

    /// Creates an integer-only numeric parameter.
    #[must_use]
    pub fn integer(id: impl Into<String>) -> Self {
        new_parameter(
            id,
            ParameterType::Number {
                integer: true,
                min: None,
                max: None,
                step: None,
            },
        )
    }

    /// Creates a boolean toggle parameter.
    #[must_use]
    pub fn boolean(id: impl Into<String>) -> Self {
        new_parameter(id, ParameterType::Boolean)
    }

    /// Creates a select parameter with no initial options.
    #[must_use]
    pub fn select(id: impl Into<String>) -> Self {
        new_parameter(
            id,
            ParameterType::Select {
                options: Vec::new(),
                dynamic: false,
                depends_on: Vec::new(),
                multiple: false,
                allow_custom: false,
                searchable: false,
                loader: None,
            },
        )
    }

    /// Creates a nested object parameter with no initial sub-parameters.
    #[must_use]
    pub fn object(id: impl Into<String>) -> Self {
        new_parameter(
            id,
            ParameterType::Object {
                parameters: Vec::new(),
                display_mode: DisplayMode::default(),
            },
        )
    }

    /// Creates an object parameter with pre-populated child parameters.
    #[must_use]
    pub fn object_with(id: impl Into<String>, parameters: Vec<Parameter>) -> Self {
        new_parameter(
            id,
            ParameterType::Object {
                parameters,
                display_mode: DisplayMode::default(),
            },
        )
    }

    /// Creates a list parameter with the given item template.
    #[must_use]
    pub fn list(id: impl Into<String>, item: Parameter) -> Self {
        new_parameter(
            id,
            ParameterType::List {
                item: Box::new(item),
                min_items: None,
                max_items: None,
                unique: false,
                sortable: false,
            },
        )
    }

    /// Creates a mode (discriminated union) parameter with no initial variants.
    #[must_use]
    pub fn mode(id: impl Into<String>) -> Self {
        new_parameter(
            id,
            ParameterType::Mode {
                variants: Vec::new(),
                default_variant: None,
            },
        )
    }

    /// Creates a code editor parameter for the given language.
    #[must_use]
    pub fn code(id: impl Into<String>, language: impl Into<String>) -> Self {
        new_parameter(
            id,
            ParameterType::Code {
                language: language.into(),
            },
        )
    }

    /// Creates a date picker parameter (String with `InputHint::Date`).
    #[must_use]
    pub fn date(id: impl Into<String>) -> Self {
        new_parameter(
            id,
            ParameterType::String {
                multiline: false,
                input_hint: InputHint::Date,
            },
        )
    }

    /// Creates a date-and-time picker parameter (String with `InputHint::DateTime`).
    #[must_use]
    pub fn datetime(id: impl Into<String>) -> Self {
        new_parameter(
            id,
            ParameterType::String {
                multiline: false,
                input_hint: InputHint::DateTime,
            },
        )
    }

    /// Creates a time-only picker parameter (String with `InputHint::Time`).
    #[must_use]
    pub fn time(id: impl Into<String>) -> Self {
        new_parameter(
            id,
            ParameterType::String {
                multiline: false,
                input_hint: InputHint::Time,
            },
        )
    }

    /// Creates a color picker parameter (String with `InputHint::Color`).
    #[must_use]
    pub fn color(id: impl Into<String>) -> Self {
        new_parameter(
            id,
            ParameterType::String {
                multiline: false,
                input_hint: InputHint::Color,
            },
        )
    }

    /// Creates a file upload parameter.
    #[must_use]
    pub fn file(id: impl Into<String>) -> Self {
        new_parameter(
            id,
            ParameterType::File {
                accept: None,
                max_size: None,
                multiple: false,
            },
        )
    }

    /// Creates a hidden parameter (not rendered in the UI).
    #[must_use]
    #[deprecated(since = "0.4.0", note = "set visible = false on Parameter instead")]
    #[allow(deprecated)]
    pub fn hidden(id: impl Into<String>) -> Self {
        new_parameter(id, ParameterType::Hidden)
    }

    /// Creates a filter/query builder parameter.
    #[must_use]
    pub fn filter(id: impl Into<String>) -> Self {
        new_parameter(
            id,
            ParameterType::Filter {
                operators: None,
                allow_groups: true,
                max_depth: 3,
                fields: Vec::new(),
                dynamic_fields: false,
                depends_on: Vec::new(),
                fields_loader: None,
            },
        )
    }

    /// Creates a computed parameter with an empty expression returning a string.
    #[must_use]
    pub fn computed(id: impl Into<String>) -> Self {
        new_parameter(
            id,
            ParameterType::Computed {
                expression: String::new(),
                returns: ComputedReturn::String,
            },
        )
    }

    /// Creates a dynamic parameter whose schema is resolved at runtime.
    #[must_use]
    pub fn dynamic(id: impl Into<String>) -> Self {
        new_parameter(
            id,
            ParameterType::Dynamic {
                depends_on: Vec::new(),
                loader: None,
            },
        )
    }

    /// Creates an informational notice (severity: Info).
    #[must_use]
    pub fn notice(id: impl Into<String>) -> Self {
        new_parameter(
            id,
            ParameterType::Notice {
                severity: NoticeSeverity::Info,
            },
        )
    }

    /// Creates a warning notice.
    #[must_use]
    pub fn warning(id: impl Into<String>) -> Self {
        new_parameter(
            id,
            ParameterType::Notice {
                severity: NoticeSeverity::Warning,
            },
        )
    }

    /// Creates a danger/error notice.
    #[must_use]
    pub fn danger(id: impl Into<String>) -> Self {
        new_parameter(
            id,
            ParameterType::Notice {
                severity: NoticeSeverity::Danger,
            },
        )
    }

    /// Creates a success notice.
    #[must_use]
    pub fn success(id: impl Into<String>) -> Self {
        new_parameter(
            id,
            ParameterType::Notice {
                severity: NoticeSeverity::Success,
            },
        )
    }
}

// ── Shared fluent builders ─────────────────────────────────────────────────

impl Parameter {
    /// Sets the human-readable label.
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Sets the description / help text.
    #[must_use]
    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Sets the placeholder text.
    #[must_use]
    pub fn placeholder(mut self, ph: impl Into<String>) -> Self {
        self.placeholder = Some(ph.into());
        self
    }

    /// Sets the inline hint displayed below the field.
    #[must_use]
    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// Marks this parameter as required.
    #[must_use]
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    /// Marks this parameter as secret (masked in UI, encrypted at rest).
    #[must_use]
    pub fn secret(mut self) -> Self {
        self.secret = true;
        self
    }

    /// Sets the default value.
    #[must_use]
    pub fn default(mut self, value: serde_json::Value) -> Self {
        self.default = Some(value);
        self
    }

    /// Adds a validation rule.
    #[must_use]
    pub fn with_rule(mut self, rule: Rule) -> Self {
        self.rules.push(rule);
        self
    }

    /// Sets the condition controlling visibility.
    #[must_use]
    pub fn visible_when(mut self, condition: Condition) -> Self {
        self.visible_when = Some(condition);
        self
    }

    /// Sets the condition controlling when this parameter is required.
    #[must_use]
    pub fn required_when(mut self, condition: Condition) -> Self {
        self.required_when = Some(condition);
        self
    }

    /// Sets the condition controlling when this parameter is disabled.
    #[must_use]
    pub fn disabled_when(mut self, condition: Condition) -> Self {
        self.disabled_when = Some(condition);
        self
    }

    /// Sets both `visible_when` and `required_when` to the same condition.
    ///
    /// Useful when a parameter should appear and become required together.
    #[must_use]
    pub fn active_when(mut self, condition: Condition) -> Self {
        self.visible_when = Some(condition.clone());
        self.required_when = Some(condition);
        self
    }

    /// Overrides the HTML input type (e.g. `"email"`, `"url"`).
    #[must_use]
    #[deprecated(since = "0.4.0", note = "use .input_hint(InputHint::...) instead")]
    #[allow(deprecated)]
    pub fn input_type(mut self, input_type: impl Into<String>) -> Self {
        self.input_type = Some(input_type.into());
        self
    }

    /// Sets the input hint for String parameters.
    ///
    /// Controls the UI widget rendered for this field (date picker,
    /// color picker, URL input, etc.). Only affects `ParameterType::String`.
    #[must_use]
    pub fn input_hint(mut self, hint: InputHint) -> Self {
        if let ParameterType::String { input_hint, .. } = &mut self.param_type {
            *input_hint = hint;
        }
        self
    }

    /// Sets the grouping key for UI sectioning.
    #[must_use]
    pub fn group(mut self, group: impl Into<String>) -> Self {
        self.group = Some(group.into());
        self
    }

    /// Enables expression mode (allows `{{ variable }}` interpolation).
    ///
    /// This is the default — most fields support expressions.
    #[must_use]
    pub fn expression(mut self) -> Self {
        self.expression = true;
        self
    }

    /// Disables expression mode for this field.
    ///
    /// Use for fields where expressions don't make sense (e.g., boolean
    /// toggles, resource pickers).
    #[must_use]
    pub fn no_expression(mut self) -> Self {
        self.expression = false;
        self
    }
}

// ── Transformer helpers ────────────────────────────────────────────────────

impl Parameter {
    /// Adds a [`Transformer::Trim`] to the transformer chain.
    #[must_use]
    pub fn trim(mut self) -> Self {
        self.transformers.push(Transformer::Trim);
        self
    }

    /// Adds a [`Transformer::Lowercase`] to the transformer chain.
    #[must_use]
    pub fn lowercase(mut self) -> Self {
        self.transformers.push(Transformer::Lowercase);
        self
    }

    /// Adds a [`Transformer::Uppercase`] to the transformer chain.
    #[must_use]
    pub fn uppercase(mut self) -> Self {
        self.transformers.push(Transformer::Uppercase);
        self
    }

    /// Adds a [`Transformer::Regex`] that extracts a capture group.
    #[must_use]
    pub fn extract_regex(mut self, pattern: impl Into<String>, group: usize) -> Self {
        self.transformers.push(Transformer::Regex {
            pattern: pattern.into(),
            group,
        });
        self
    }

    /// Adds an arbitrary transformer to the chain.
    #[must_use]
    pub fn transformer(mut self, t: Transformer) -> Self {
        self.transformers.push(t);
        self
    }
}

// ── Type-specific builders ─────────────────────────────────────────────────

/// Asserts in debug builds that `$self.param_type` matches `$variant`.
/// Zero cost in release builds — catches builder misuse during development.
macro_rules! debug_assert_type {
    ($self:expr, $( $variant:pat_param )|+, $method:literal) => {
        debug_assert!(
            matches!($self.param_type, $( $variant )|+),
            concat!($method, "() called on wrong ParameterType variant: {:?}"),
            $self.param_type
        );
    };
}

impl Parameter {
    // ── String ──────────────────────────────────────────────────────────

    /// Enables multi-line textarea mode. Only affects [`ParameterType::String`].
    #[must_use]
    pub fn multiline(mut self) -> Self {
        if let ParameterType::String { multiline, .. } = &mut self.param_type {
            *multiline = true;
        } else {
            debug_assert_type!(self, ParameterType::String { .. }, "multiline");
        }
        self
    }

    // ── Number ──────────────────────────────────────────────────────────

    /// Sets the inclusive lower bound. Only affects [`ParameterType::Number`].
    ///
    /// Uses `f64`; for values that cannot be represented as `f64` (e.g. very
    /// large integers), use the `min` field directly.
    #[must_use]
    pub fn min(mut self, v: f64) -> Self {
        if let ParameterType::Number { min, .. } = &mut self.param_type {
            *min = serde_json::Number::from_f64(v);
        } else {
            debug_assert_type!(self, ParameterType::Number { .. }, "min");
        }
        self
    }

    /// Sets the inclusive upper bound. Only affects [`ParameterType::Number`].
    #[must_use]
    pub fn max(mut self, v: f64) -> Self {
        if let ParameterType::Number { max, .. } = &mut self.param_type {
            *max = serde_json::Number::from_f64(v);
        } else {
            debug_assert_type!(self, ParameterType::Number { .. }, "max");
        }
        self
    }

    /// Sets the stepper increment. Only affects [`ParameterType::Number`].
    #[must_use]
    pub fn step(mut self, v: f64) -> Self {
        if let ParameterType::Number { step, .. } = &mut self.param_type {
            *step = serde_json::Number::from_f64(v);
        } else {
            debug_assert_type!(self, ParameterType::Number { .. }, "step");
        }
        self
    }

    /// Sets the inclusive lower bound from an `i64`. Only affects [`ParameterType::Number`].
    #[must_use]
    pub fn min_i64(mut self, v: i64) -> Self {
        if let ParameterType::Number { min, .. } = &mut self.param_type {
            *min = Some(serde_json::Number::from(v));
        } else {
            debug_assert_type!(self, ParameterType::Number { .. }, "min_i64");
        }
        self
    }

    /// Sets the inclusive upper bound from an `i64`. Only affects [`ParameterType::Number`].
    #[must_use]
    pub fn max_i64(mut self, v: i64) -> Self {
        if let ParameterType::Number { max, .. } = &mut self.param_type {
            *max = Some(serde_json::Number::from(v));
        } else {
            debug_assert_type!(self, ParameterType::Number { .. }, "max_i64");
        }
        self
    }

    /// Sets the stepper increment from an `i64`. Only affects [`ParameterType::Number`].
    #[must_use]
    pub fn step_i64(mut self, v: i64) -> Self {
        if let ParameterType::Number { step, .. } = &mut self.param_type {
            *step = Some(serde_json::Number::from(v));
        } else {
            debug_assert_type!(self, ParameterType::Number { .. }, "step_i64");
        }
        self
    }

    // ── Select ──────────────────────────────────────────────────────────

    /// Adds a static option to a [`ParameterType::Select`].
    #[must_use]
    pub fn option(mut self, value: impl Into<serde_json::Value>, label: impl Into<String>) -> Self {
        if let ParameterType::Select { options, .. } = &mut self.param_type {
            options.push(SelectOption::new(value.into(), label));
        } else {
            debug_assert_type!(self, ParameterType::Select { .. }, "option");
        }
        self
    }

    /// Adds a pre-built [`SelectOption`] to a [`ParameterType::Select`].
    #[must_use]
    pub fn option_with(mut self, opt: SelectOption) -> Self {
        if let ParameterType::Select { options, .. } = &mut self.param_type {
            options.push(opt);
        } else {
            debug_assert_type!(self, ParameterType::Select { .. }, "option_with");
        }
        self
    }

    /// Enables multi-select mode. Works for [`ParameterType::Select`] and
    /// [`ParameterType::File`].
    #[must_use]
    pub fn multiple(mut self) -> Self {
        match &mut self.param_type {
            ParameterType::Select { multiple, .. } | ParameterType::File { multiple, .. } => {
                *multiple = true;
            }
            _ => {
                debug_assert_type!(
                    self,
                    ParameterType::Select { .. } | ParameterType::File { .. },
                    "multiple"
                );
            }
        }
        self
    }

    /// Allows custom values not in the option list. Only affects [`ParameterType::Select`].
    #[must_use]
    pub fn allow_custom(mut self) -> Self {
        if let ParameterType::Select { allow_custom, .. } = &mut self.param_type {
            *allow_custom = true;
        } else {
            debug_assert_type!(self, ParameterType::Select { .. }, "allow_custom");
        }
        self
    }

    /// Enables search filtering in the option picker. Only affects [`ParameterType::Select`].
    #[must_use]
    pub fn searchable(mut self) -> Self {
        if let ParameterType::Select { searchable, .. } = &mut self.param_type {
            *searchable = true;
        } else {
            debug_assert_type!(self, ParameterType::Select { .. }, "searchable");
        }
        self
    }

    /// Sets dependency paths that trigger a reload. Works for [`ParameterType::Select`],
    /// [`ParameterType::Filter`], and [`ParameterType::Dynamic`].
    #[must_use]
    pub fn depends_on(mut self, deps: &[&str]) -> Self {
        let paths: Vec<ParameterPath> = deps.iter().map(|&s| ParameterPath::from(s)).collect();
        match &mut self.param_type {
            ParameterType::Select { depends_on, .. }
            | ParameterType::Filter { depends_on, .. }
            | ParameterType::Dynamic { depends_on, .. } => {
                *depends_on = paths;
            }
            _ => {
                debug_assert_type!(
                    self,
                    ParameterType::Select { .. }
                        | ParameterType::Filter { .. }
                        | ParameterType::Dynamic { .. },
                    "depends_on"
                );
            }
        }
        self
    }

    // ── Object ──────────────────────────────────────────────────────────

    /// Adds a child parameter to an [`ParameterType::Object`].
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn add(mut self, param: Parameter) -> Self {
        if let ParameterType::Object { parameters, .. } = &mut self.param_type {
            parameters.push(param);
        } else {
            debug_assert_type!(self, ParameterType::Object { .. }, "add");
        }
        self
    }

    /// Sets display mode to [`DisplayMode::Collapsed`]. Only affects [`ParameterType::Object`].
    #[must_use]
    pub fn collapsed(mut self) -> Self {
        if let ParameterType::Object { display_mode, .. } = &mut self.param_type {
            *display_mode = DisplayMode::Collapsed;
        } else {
            debug_assert_type!(self, ParameterType::Object { .. }, "collapsed");
        }
        self
    }

    /// Sets display mode to [`DisplayMode::PickFields`]. Only affects [`ParameterType::Object`].
    #[must_use]
    pub fn pick_fields(mut self) -> Self {
        if let ParameterType::Object { display_mode, .. } = &mut self.param_type {
            *display_mode = DisplayMode::PickFields;
        } else {
            debug_assert_type!(self, ParameterType::Object { .. }, "pick_fields");
        }
        self
    }

    /// Sets display mode to [`DisplayMode::Sections`]. Only affects [`ParameterType::Object`].
    #[must_use]
    pub fn sections(mut self) -> Self {
        if let ParameterType::Object { display_mode, .. } = &mut self.param_type {
            *display_mode = DisplayMode::Sections;
        } else {
            debug_assert_type!(self, ParameterType::Object { .. }, "sections");
        }
        self
    }

    // ── List ────────────────────────────────────────────────────────────

    /// Sets the minimum number of items. Only affects [`ParameterType::List`].
    #[must_use]
    pub fn min_items(mut self, n: u32) -> Self {
        if let ParameterType::List { min_items, .. } = &mut self.param_type {
            *min_items = Some(n);
        } else {
            debug_assert_type!(self, ParameterType::List { .. }, "min_items");
        }
        self
    }

    /// Sets the maximum number of items. Only affects [`ParameterType::List`].
    #[must_use]
    pub fn max_items(mut self, n: u32) -> Self {
        if let ParameterType::List { max_items, .. } = &mut self.param_type {
            *max_items = Some(n);
        } else {
            debug_assert_type!(self, ParameterType::List { .. }, "max_items");
        }
        self
    }

    /// Enforces uniqueness across list items. Only affects [`ParameterType::List`].
    #[must_use]
    pub fn unique(mut self) -> Self {
        if let ParameterType::List { unique, .. } = &mut self.param_type {
            *unique = true;
        } else {
            debug_assert_type!(self, ParameterType::List { .. }, "unique");
        }
        self
    }

    /// Enables drag-and-drop reordering. Only affects [`ParameterType::List`].
    #[must_use]
    pub fn sortable(mut self) -> Self {
        if let ParameterType::List { sortable, .. } = &mut self.param_type {
            *sortable = true;
        } else {
            debug_assert_type!(self, ParameterType::List { .. }, "sortable");
        }
        self
    }

    // ── Mode ────────────────────────────────────────────────────────────

    /// Adds a variant to a [`ParameterType::Mode`].
    #[must_use]
    pub fn variant(mut self, param: Parameter) -> Self {
        if let ParameterType::Mode { variants, .. } = &mut self.param_type {
            variants.push(param);
        } else {
            debug_assert_type!(self, ParameterType::Mode { .. }, "variant");
        }
        self
    }

    /// Sets the default variant key. Only affects [`ParameterType::Mode`].
    #[must_use]
    pub fn default_variant(mut self, key: impl Into<String>) -> Self {
        if let ParameterType::Mode {
            default_variant, ..
        } = &mut self.param_type
        {
            *default_variant = Some(key.into());
        } else {
            debug_assert_type!(self, ParameterType::Mode { .. }, "default_variant");
        }
        self
    }

    // ── File ────────────────────────────────────────────────────────────

    /// Sets the MIME type filter. Only affects [`ParameterType::File`].
    #[must_use]
    pub fn accept(mut self, accept: impl Into<String>) -> Self {
        if let ParameterType::File { accept: a, .. } = &mut self.param_type {
            *a = Some(accept.into());
        } else {
            debug_assert_type!(self, ParameterType::File { .. }, "accept");
        }
        self
    }

    /// Sets the maximum file size in bytes. Only affects [`ParameterType::File`].
    #[must_use]
    pub fn max_size(mut self, size: u64) -> Self {
        if let ParameterType::File { max_size, .. } = &mut self.param_type {
            *max_size = Some(size);
        } else {
            debug_assert_type!(self, ParameterType::File { .. }, "max_size");
        }
        self
    }

    // ── Filter ──────────────────────────────────────────────────────────

    /// Sets the allowed comparison operators. Only affects [`ParameterType::Filter`].
    #[must_use]
    pub fn operators(mut self, ops: Vec<FilterOp>) -> Self {
        if let ParameterType::Filter { operators, .. } = &mut self.param_type {
            *operators = Some(ops);
        } else {
            debug_assert_type!(self, ParameterType::Filter { .. }, "operators");
        }
        self
    }

    /// Sets whether logical grouping (AND/OR) is allowed. Only affects [`ParameterType::Filter`].
    #[must_use]
    pub fn allow_groups(mut self, allow: bool) -> Self {
        if let ParameterType::Filter { allow_groups, .. } = &mut self.param_type {
            *allow_groups = allow;
        } else {
            debug_assert_type!(self, ParameterType::Filter { .. }, "allow_groups");
        }
        self
    }

    /// Sets the maximum nesting depth. Only affects [`ParameterType::Filter`].
    #[must_use]
    pub fn max_depth(mut self, depth: u8) -> Self {
        if let ParameterType::Filter { max_depth, .. } = &mut self.param_type {
            *max_depth = depth;
        } else {
            debug_assert_type!(self, ParameterType::Filter { .. }, "max_depth");
        }
        self
    }

    /// Adds a static filter field. Only affects [`ParameterType::Filter`].
    #[must_use]
    pub fn filter_field(mut self, field: FilterField) -> Self {
        if let ParameterType::Filter { fields, .. } = &mut self.param_type {
            fields.push(field);
        } else {
            debug_assert_type!(self, ParameterType::Filter { .. }, "filter_field");
        }
        self
    }

    /// Enables dynamic filter field loading. Only affects [`ParameterType::Filter`].
    #[must_use]
    pub fn dynamic_fields(mut self) -> Self {
        if let ParameterType::Filter { dynamic_fields, .. } = &mut self.param_type {
            *dynamic_fields = true;
        } else {
            debug_assert_type!(self, ParameterType::Filter { .. }, "dynamic_fields");
        }
        self
    }

    // ── Computed ────────────────────────────────────────────────────────

    /// Sets the computed return type to string. Only affects [`ParameterType::Computed`].
    #[must_use]
    pub fn returns_string(mut self) -> Self {
        if let ParameterType::Computed { returns, .. } = &mut self.param_type {
            *returns = ComputedReturn::String;
        } else {
            debug_assert_type!(self, ParameterType::Computed { .. }, "returns_string");
        }
        self
    }

    /// Sets the computed return type to number. Only affects [`ParameterType::Computed`].
    #[must_use]
    pub fn returns_number(mut self) -> Self {
        if let ParameterType::Computed { returns, .. } = &mut self.param_type {
            *returns = ComputedReturn::Number;
        } else {
            debug_assert_type!(self, ParameterType::Computed { .. }, "returns_number");
        }
        self
    }

    /// Sets the computed return type to boolean. Only affects [`ParameterType::Computed`].
    #[must_use]
    pub fn returns_boolean(mut self) -> Self {
        if let ParameterType::Computed { returns, .. } = &mut self.param_type {
            *returns = ComputedReturn::Boolean;
        } else {
            debug_assert_type!(self, ParameterType::Computed { .. }, "returns_boolean");
        }
        self
    }
}

// ── Loader setters ─────────────────────────────────────────────────────────

impl Parameter {
    /// Attaches an async option loader to a [`ParameterType::Select`].
    ///
    /// Automatically marks the select as `dynamic = true`.
    #[must_use]
    pub fn with_option_loader<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(LoaderContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<LoaderResult<SelectOption>, LoaderError>> + Send + 'static,
    {
        if let ParameterType::Select {
            loader, dynamic, ..
        } = &mut self.param_type
        {
            *loader = Some(OptionLoader::new(f));
            *dynamic = true;
        } else {
            debug_assert_type!(self, ParameterType::Select { .. }, "with_option_loader");
        }
        self
    }

    /// Attaches an async record loader to a [`ParameterType::Dynamic`].
    #[must_use]
    pub fn with_record_loader<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(LoaderContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<LoaderResult<serde_json::Value>, LoaderError>> + Send + 'static,
    {
        if let ParameterType::Dynamic { loader, .. } = &mut self.param_type {
            *loader = Some(RecordLoader::new(f));
        } else {
            debug_assert_type!(self, ParameterType::Dynamic { .. }, "with_record_loader");
        }
        self
    }

    /// Attaches an async filter field loader to a [`ParameterType::Filter`].
    ///
    /// Automatically marks the filter as `dynamic_fields = true`.
    #[must_use]
    pub fn with_filter_field_loader<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(LoaderContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<LoaderResult<FilterField>, LoaderError>> + Send + 'static,
    {
        if let ParameterType::Filter {
            fields_loader,
            dynamic_fields,
            ..
        } = &mut self.param_type
        {
            *fields_loader = Some(FilterFieldLoader::new(f));
            *dynamic_fields = true;
        } else {
            debug_assert_type!(
                self,
                ParameterType::Filter { .. },
                "with_filter_field_loader"
            );
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_constructor_defaults() {
        let p = Parameter::string("name");
        assert_eq!(p.id, "name");
        assert_eq!(
            p.param_type,
            ParameterType::String {
                multiline: false,
                input_hint: InputHint::default()
            }
        );
        assert!(!p.required);
        assert!(!p.secret);
        assert!(p.label.is_none());
    }

    #[test]
    fn fluent_shared_metadata() {
        let p = Parameter::string("api_key")
            .label("API Key")
            .description("Your API key")
            .placeholder("sk-...")
            .hint("Find it in settings")
            .required()
            .secret();

        assert_eq!(p.label.as_deref(), Some("API Key"));
        assert_eq!(p.description.as_deref(), Some("Your API key"));
        assert_eq!(p.placeholder.as_deref(), Some("sk-..."));
        assert_eq!(p.hint.as_deref(), Some("Find it in settings"));
        assert!(p.required);
        assert!(p.secret);
    }

    #[test]
    fn integer_constructor() {
        let p = Parameter::integer("count");
        if let ParameterType::Number { integer, .. } = &p.param_type {
            assert!(integer);
        } else {
            panic!("expected Number variant");
        }
    }

    #[test]
    fn number_min_max_step() {
        let p = Parameter::number("score").min(0.0).max(100.0).step(0.5);
        if let ParameterType::Number { min, max, step, .. } = &p.param_type {
            assert!(min.is_some());
            assert!(max.is_some());
            assert!(step.is_some());
        } else {
            panic!("expected Number variant");
        }
    }

    #[test]
    fn number_i64_helpers() {
        let p = Parameter::integer("count")
            .min_i64(0)
            .max_i64(1000)
            .step_i64(1);
        if let ParameterType::Number { min, max, step, .. } = &p.param_type {
            assert_eq!(min.as_ref().and_then(|n| n.as_i64()), Some(0));
            assert_eq!(max.as_ref().and_then(|n| n.as_i64()), Some(1000));
            assert_eq!(step.as_ref().and_then(|n| n.as_i64()), Some(1));
        } else {
            panic!("expected Number variant");
        }
    }

    #[test]
    fn select_with_options() {
        let p = Parameter::select("region")
            .option(serde_json::json!("us-east-1"), "US East")
            .option(serde_json::json!("eu-west-1"), "EU West")
            .searchable()
            .multiple();

        if let ParameterType::Select {
            options,
            searchable,
            multiple,
            ..
        } = &p.param_type
        {
            assert_eq!(options.len(), 2);
            assert!(searchable);
            assert!(multiple);
        } else {
            panic!("expected Select variant");
        }
    }

    #[test]
    fn object_with_children() {
        let p = Parameter::object("auth")
            .collapsed()
            .add(Parameter::string("username"))
            .add(Parameter::string("password").secret());

        if let ParameterType::Object {
            parameters,
            display_mode,
        } = &p.param_type
        {
            assert_eq!(parameters.len(), 2);
            assert_eq!(*display_mode, DisplayMode::Collapsed);
        } else {
            panic!("expected Object variant");
        }
    }

    #[test]
    fn list_with_constraints() {
        let p = Parameter::list("tags", Parameter::string("tag"))
            .min_items(1)
            .max_items(10)
            .unique()
            .sortable();

        if let ParameterType::List {
            min_items,
            max_items,
            unique,
            sortable,
            ..
        } = &p.param_type
        {
            assert_eq!(*min_items, Some(1));
            assert_eq!(*max_items, Some(10));
            assert!(unique);
            assert!(sortable);
        } else {
            panic!("expected List variant");
        }
    }

    #[test]
    fn mode_with_variants() {
        let p = Parameter::mode("auth_type")
            .variant(Parameter::string("api_key"))
            .variant(Parameter::object("oauth2"))
            .default_variant("api_key");

        if let ParameterType::Mode {
            variants,
            default_variant,
        } = &p.param_type
        {
            assert_eq!(variants.len(), 2);
            assert_eq!(default_variant.as_deref(), Some("api_key"));
        } else {
            panic!("expected Mode variant");
        }
    }

    #[test]
    fn multiline_enables_textarea_mode() {
        let s = Parameter::string("text").multiline();
        assert_eq!(
            s.param_type,
            ParameterType::String {
                multiline: true,
                input_hint: InputHint::default()
            }
        );
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "multiline()")]
    fn multiline_on_wrong_type_panics_in_debug() {
        let _ = Parameter::number("num").multiline();
    }

    #[test]
    fn transformer_helpers() {
        let p = Parameter::string("email").trim().lowercase();
        assert_eq!(p.transformers.len(), 2);
        assert_eq!(p.transformers[0], Transformer::Trim);
        assert_eq!(p.transformers[1], Transformer::Lowercase);
    }

    #[test]
    fn notice_constructors() {
        let info = Parameter::notice("n1");
        let warn = Parameter::warning("n2");
        let danger = Parameter::danger("n3");
        let success = Parameter::success("n4");

        assert_eq!(
            info.param_type,
            ParameterType::Notice {
                severity: NoticeSeverity::Info
            }
        );
        assert_eq!(
            warn.param_type,
            ParameterType::Notice {
                severity: NoticeSeverity::Warning
            }
        );
        assert_eq!(
            danger.param_type,
            ParameterType::Notice {
                severity: NoticeSeverity::Danger
            }
        );
        assert_eq!(
            success.param_type,
            ParameterType::Notice {
                severity: NoticeSeverity::Success
            }
        );
    }

    #[test]
    fn active_when_sets_both_conditions() {
        let cond = Condition::eq("mode", "advanced");
        let p = Parameter::string("extra").active_when(cond.clone());
        assert_eq!(p.visible_when, Some(cond.clone()));
        assert_eq!(p.required_when, Some(cond));
    }

    #[test]
    fn expression_enables_interpolation() {
        let p = Parameter::string("template").expression();
        assert!(p.expression);
    }

    #[test]
    fn depends_on_sets_paths() {
        let p = Parameter::select("sub_region").depends_on(&["region", "country"]);
        if let ParameterType::Select { depends_on, .. } = &p.param_type {
            assert_eq!(depends_on.len(), 2);
            assert_eq!(depends_on[0].as_str(), "region");
            assert_eq!(depends_on[1].as_str(), "country");
        } else {
            panic!("expected Select variant");
        }
    }

    #[test]
    fn file_builders() {
        let p = Parameter::file("upload")
            .accept("image/*")
            .max_size(5_000_000)
            .multiple();

        if let ParameterType::File {
            accept,
            max_size,
            multiple,
        } = &p.param_type
        {
            assert_eq!(accept.as_deref(), Some("image/*"));
            assert_eq!(*max_size, Some(5_000_000));
            assert!(multiple);
        } else {
            panic!("expected File variant");
        }
    }

    #[test]
    fn filter_builders() {
        let p = Parameter::filter("query")
            .operators(vec![FilterOp::Eq, FilterOp::Ne])
            .allow_groups(false)
            .max_depth(2)
            .dynamic_fields()
            .depends_on(&["table"]);

        if let ParameterType::Filter {
            operators,
            allow_groups,
            max_depth,
            dynamic_fields,
            depends_on,
            ..
        } = &p.param_type
        {
            assert_eq!(operators.as_ref().map(Vec::len), Some(2));
            assert!(!allow_groups);
            assert_eq!(*max_depth, 2);
            assert!(dynamic_fields);
            assert_eq!(depends_on.len(), 1);
        } else {
            panic!("expected Filter variant");
        }
    }

    #[test]
    fn computed_return_type_builders() {
        let p = Parameter::computed("full_name").returns_number();
        if let ParameterType::Computed { returns, .. } = &p.param_type {
            assert_eq!(*returns, ComputedReturn::Number);
        } else {
            panic!("expected Computed variant");
        }
    }

    #[test]
    fn code_constructor() {
        let p = Parameter::code("template", "json");
        if let ParameterType::Code { language } = &p.param_type {
            assert_eq!(language, "json");
        } else {
            panic!("expected Code variant");
        }
    }

    #[test]
    fn serde_round_trip_simple() {
        let p = Parameter::string("name").label("Name").required();
        let json = serde_json::to_string(&p).unwrap();
        let back: Parameter = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn serde_round_trip_nested_object() {
        let p = Parameter::object("config")
            .add(Parameter::string("host"))
            .add(Parameter::integer("port").default(serde_json::json!(8080)));
        let json = serde_json::to_string(&p).unwrap();
        let back: Parameter = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn serde_flattened_type_tag() {
        let p = Parameter::boolean("enabled");
        let v: serde_json::Value = serde_json::to_value(&p).unwrap();
        assert_eq!(v["type"], "boolean");
        assert_eq!(v["id"], "enabled");
    }

    #[test]
    fn serde_omits_default_fields() {
        let p = Parameter::string("x");
        let json = serde_json::to_string(&p).unwrap();
        // Defaults should be omitted
        assert!(!json.contains("\"label\""));
        assert!(!json.contains("\"required\""));
        assert!(!json.contains("\"secret\""));
        assert!(!json.contains("\"multiline\""));
        assert!(!json.contains("\"rules\""));
        assert!(!json.contains("\"transformers\""));
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "searchable()")]
    fn searchable_on_wrong_type_panics_in_debug() {
        let _ = Parameter::string("x").searchable();
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "option()")]
    fn option_on_wrong_type_panics_in_debug() {
        let _ = Parameter::boolean("x").option("v", "label");
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "min()")]
    fn min_on_wrong_type_panics_in_debug() {
        let _ = Parameter::string("x").min(0.0);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "collapsed()")]
    fn collapsed_on_wrong_type_panics_in_debug() {
        let _ = Parameter::string("x").collapsed();
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "variant()")]
    fn variant_on_wrong_type_panics_in_debug() {
        let _ = Parameter::string("x").variant(Parameter::string("y"));
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "min_items()")]
    fn min_items_on_wrong_type_panics_in_debug() {
        let _ = Parameter::string("x").min_items(1);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "accept()")]
    fn accept_on_wrong_type_panics_in_debug() {
        let _ = Parameter::string("x").accept("image/*");
    }

    #[test]
    fn input_hint_and_string_constructor_agree() {
        let via_hint = Parameter::string("email").input_hint(InputHint::Email);
        let via_date = Parameter::date("start");
        match &via_hint.param_type {
            ParameterType::String { input_hint, .. } => assert_eq!(*input_hint, InputHint::Email),
            _ => panic!("expected String variant"),
        }
        match &via_date.param_type {
            ParameterType::String { input_hint, .. } => assert_eq!(*input_hint, InputHint::Date),
            _ => panic!("expected String variant"),
        }
    }
}

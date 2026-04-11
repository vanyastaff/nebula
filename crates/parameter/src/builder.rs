//! Typed builder API for [`ParameterCollection`].
//!
//! Each parameter type has its own builder with only relevant methods —
//! calling `.multiline()` on a number builder is a compile error.
//!
//! # Examples
//!
//! ```
//! use nebula_parameter::prelude::*;
//!
//! let params = ParameterCollection::builder()
//!     .string("url", |s| s.label("URL").required())
//!     .select("method", |s| {
//!         s.option("GET", "GET").option("POST", "POST").default("GET")
//!     })
//!     .number("timeout", |n| n.label("Timeout").integer().default(30))
//!     .boolean("verbose", |b| b.label("Verbose").no_expression())
//!     .build();
//!
//! assert_eq!(params.len(), 4);
//! ```

use crate::{
    ParameterCollection, conditions::Condition, input_hint::InputHint, option::SelectOption,
    parameter::Parameter, parameter_type::ParameterType, rules::Rule, transformer::Transformer,
};

// ── Collection builder ─────────────────────────────────────────────────────

/// Fluent builder for [`ParameterCollection`] with typed closures.
#[derive(Default)]
pub struct ParameterCollectionBuilder {
    params: Vec<Parameter>,
}

impl ParameterCollectionBuilder {
    /// Create a new empty builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a string parameter.
    #[must_use]
    pub fn string(
        mut self,
        id: impl Into<String>,
        f: impl FnOnce(StringBuilder) -> StringBuilder,
    ) -> Self {
        let b = StringBuilder(Parameter::string(id));
        self.params.push(f(b).0);
        self
    }

    /// Add a number parameter (floating-point by default).
    #[must_use]
    pub fn number(
        mut self,
        id: impl Into<String>,
        f: impl FnOnce(NumberBuilder) -> NumberBuilder,
    ) -> Self {
        let b = NumberBuilder(Parameter::number(id));
        self.params.push(f(b).0);
        self
    }

    /// Add an integer parameter.
    #[must_use]
    pub fn integer(
        mut self,
        id: impl Into<String>,
        f: impl FnOnce(NumberBuilder) -> NumberBuilder,
    ) -> Self {
        let b = NumberBuilder(Parameter::integer(id));
        self.params.push(f(b).0);
        self
    }

    /// Add a boolean parameter.
    #[must_use]
    pub fn boolean(
        mut self,
        id: impl Into<String>,
        f: impl FnOnce(BooleanBuilder) -> BooleanBuilder,
    ) -> Self {
        let b = BooleanBuilder(Parameter::boolean(id));
        self.params.push(f(b).0);
        self
    }

    /// Add a select (dropdown) parameter.
    #[must_use]
    pub fn select(
        mut self,
        id: impl Into<String>,
        f: impl FnOnce(SelectBuilder) -> SelectBuilder,
    ) -> Self {
        let b = SelectBuilder(Parameter::select(id));
        self.params.push(f(b).0);
        self
    }

    /// Add a nested object parameter.
    #[must_use]
    pub fn object(
        mut self,
        id: impl Into<String>,
        f: impl FnOnce(ObjectBuilder) -> ObjectBuilder,
    ) -> Self {
        let b = ObjectBuilder::new(id);
        self.params.push(f(b).into_parameter());
        self
    }

    /// Add a list parameter.
    #[must_use]
    pub fn list(
        mut self,
        id: impl Into<String>,
        item: Parameter,
        f: impl FnOnce(ListBuilder) -> ListBuilder,
    ) -> Self {
        let b = ListBuilder(Parameter::list(id, item));
        self.params.push(f(b).0);
        self
    }

    /// Add a code editor parameter.
    #[must_use]
    pub fn code(
        mut self,
        id: impl Into<String>,
        language: impl Into<String>,
        f: impl FnOnce(CodeBuilder) -> CodeBuilder,
    ) -> Self {
        let b = CodeBuilder(Parameter::code(id, language));
        self.params.push(f(b).0);
        self
    }

    /// Add a group of parameters with shared visibility conditions.
    #[must_use]
    pub fn group(
        mut self,
        group_name: impl Into<String>,
        f: impl FnOnce(GroupBuilder) -> GroupBuilder,
    ) -> Self {
        let gb = GroupBuilder::new(group_name);
        let built = f(gb);
        self.params.extend(built.params);
        self
    }

    /// Build the final [`ParameterCollection`].
    #[must_use]
    pub fn build(self) -> ParameterCollection {
        let mut collection = ParameterCollection::new();
        for param in self.params {
            collection = collection.add(param);
        }
        collection
    }
}

// ── Shared builder methods macro ───────────────────────────────────────────

macro_rules! shared_param_methods {
    ($builder:ident) => {
        impl $builder {
            /// Set the display label.
            #[must_use]
            pub fn label(mut self, label: impl Into<String>) -> Self {
                self.0.label = Some(label.into());
                self
            }

            /// Set the description/help text.
            #[must_use]
            pub fn description(mut self, desc: impl Into<String>) -> Self {
                self.0.description = Some(desc.into());
                self
            }

            /// Set placeholder text.
            #[must_use]
            pub fn placeholder(mut self, text: impl Into<String>) -> Self {
                self.0.placeholder = Some(text.into());
                self
            }

            /// Set inline hint text.
            #[must_use]
            pub fn hint(mut self, text: impl Into<String>) -> Self {
                self.0.hint = Some(text.into());
                self
            }

            /// Mark as required.
            #[must_use]
            pub fn required(mut self) -> Self {
                self.0.required = true;
                self
            }

            /// Mark as secret (masked in UI, encrypted at rest).
            #[must_use]
            pub fn secret(mut self) -> Self {
                self.0.secret = true;
                self
            }

            /// Disable expression mode for this field.
            #[must_use]
            pub fn no_expression(mut self) -> Self {
                self.0.expression = false;
                self
            }

            /// Set visibility condition.
            #[must_use]
            pub fn visible_when(mut self, condition: Condition) -> Self {
                self.0.visible_when = Some(condition);
                self
            }

            /// Set conditional required.
            #[must_use]
            pub fn required_when(mut self, condition: Condition) -> Self {
                self.0.required_when = Some(condition);
                self
            }

            /// Set disabled condition.
            #[must_use]
            pub fn disabled_when(mut self, condition: Condition) -> Self {
                self.0.disabled_when = Some(condition);
                self
            }

            /// Add a validation rule.
            #[must_use]
            pub fn rule(mut self, rule: Rule) -> Self {
                self.0.rules.push(rule);
                self
            }

            /// Add a transformer.
            #[must_use]
            pub fn transform(mut self, t: Transformer) -> Self {
                self.0.transformers.push(t);
                self
            }

            /// Set a grouping key for UI sectioning.
            #[must_use]
            pub fn group(mut self, group: impl Into<String>) -> Self {
                self.0.group = Some(group.into());
                self
            }
        }
    };
}

// ── Per-type builders ──────────────────────────────────────────────────────

/// Builder for string parameters.
pub struct StringBuilder(pub(crate) Parameter);
shared_param_methods!(StringBuilder);

impl StringBuilder {
    /// Enable multiline (textarea) mode.
    #[must_use]
    pub fn multiline(mut self) -> Self {
        if let ParameterType::String { multiline, .. } = &mut self.0.param_type {
            *multiline = true;
        }
        self
    }

    /// Set the input hint (url, email, date, etc.).
    #[must_use]
    pub fn input_hint(mut self, hint: InputHint) -> Self {
        if let ParameterType::String { input_hint, .. } = &mut self.0.param_type {
            *input_hint = hint;
        }
        self
    }

    /// Set the default value.
    #[must_use]
    pub fn default(mut self, value: impl Into<String>) -> Self {
        self.0.default = Some(serde_json::Value::String(value.into()));
        self
    }
}

/// Builder for number parameters.
pub struct NumberBuilder(pub(crate) Parameter);
shared_param_methods!(NumberBuilder);

impl NumberBuilder {
    /// Restrict to integer input.
    #[must_use]
    pub fn integer(mut self) -> Self {
        if let ParameterType::Number { integer, .. } = &mut self.0.param_type {
            *integer = true;
        }
        self
    }

    /// Set minimum value.
    #[must_use]
    pub fn min(mut self, v: f64) -> Self {
        if let ParameterType::Number { min, .. } = &mut self.0.param_type {
            *min = serde_json::Number::from_f64(v);
        }
        self
    }

    /// Set maximum value.
    #[must_use]
    pub fn max(mut self, v: f64) -> Self {
        if let ParameterType::Number { max, .. } = &mut self.0.param_type {
            *max = serde_json::Number::from_f64(v);
        }
        self
    }

    /// Set step increment.
    #[must_use]
    pub fn step(mut self, v: f64) -> Self {
        if let ParameterType::Number { step, .. } = &mut self.0.param_type {
            *step = serde_json::Number::from_f64(v);
        }
        self
    }

    /// Set default integer value.
    #[must_use]
    pub fn default(mut self, value: i64) -> Self {
        self.0.default = Some(serde_json::Value::Number(value.into()));
        self
    }

    /// Set default float value.
    #[must_use]
    pub fn default_f64(mut self, value: f64) -> Self {
        if let Some(n) = serde_json::Number::from_f64(value) {
            self.0.default = Some(serde_json::Value::Number(n));
        }
        self
    }
}

/// Builder for boolean parameters.
pub struct BooleanBuilder(pub(crate) Parameter);
shared_param_methods!(BooleanBuilder);

impl BooleanBuilder {
    /// Set default value.
    #[must_use]
    pub fn default(mut self, value: bool) -> Self {
        self.0.default = Some(serde_json::Value::Bool(value));
        self
    }
}

/// Builder for select (dropdown) parameters.
pub struct SelectBuilder(pub(crate) Parameter);
shared_param_methods!(SelectBuilder);

impl SelectBuilder {
    /// Add a single option with a string value.
    #[must_use]
    pub fn option(mut self, value: impl Into<String>, label: impl Into<String>) -> Self {
        if let ParameterType::Select { options, .. } = &mut self.0.param_type {
            let v: String = value.into();
            options.push(SelectOption::new(serde_json::Value::String(v), label));
        }
        self
    }

    /// Add multiple options from an iterator of (value, label) pairs.
    #[must_use]
    pub fn options<I, V, L>(mut self, opts: I) -> Self
    where
        I: IntoIterator<Item = (V, L)>,
        V: Into<String>,
        L: Into<String>,
    {
        if let ParameterType::Select { options, .. } = &mut self.0.param_type {
            for (v, l) in opts {
                let val: String = v.into();
                options.push(SelectOption::new(serde_json::Value::String(val), l));
            }
        }
        self
    }

    /// Enable search filtering in the dropdown.
    #[must_use]
    pub fn searchable(mut self) -> Self {
        if let ParameterType::Select { searchable, .. } = &mut self.0.param_type {
            *searchable = true;
        }
        self
    }

    /// Allow selecting multiple values.
    #[must_use]
    pub fn multiple(mut self) -> Self {
        if let ParameterType::Select { multiple, .. } = &mut self.0.param_type {
            *multiple = true;
        }
        self
    }

    /// Allow custom values not in the option list.
    #[must_use]
    pub fn allow_custom(mut self) -> Self {
        if let ParameterType::Select { allow_custom, .. } = &mut self.0.param_type {
            *allow_custom = true;
        }
        self
    }

    /// Set default value.
    #[must_use]
    pub fn default(mut self, value: impl Into<String>) -> Self {
        self.0.default = Some(serde_json::Value::String(value.into()));
        self
    }
}

/// Builder for code editor parameters.
pub struct CodeBuilder(pub(crate) Parameter);
shared_param_methods!(CodeBuilder);

impl CodeBuilder {
    /// Set default code content.
    #[must_use]
    pub fn default(mut self, value: impl Into<String>) -> Self {
        self.0.default = Some(serde_json::Value::String(value.into()));
        self
    }
}

/// Builder for list (array) parameters.
pub struct ListBuilder(pub(crate) Parameter);
shared_param_methods!(ListBuilder);

impl ListBuilder {
    /// Set minimum number of items.
    #[must_use]
    pub fn min_items(mut self, n: u32) -> Self {
        if let ParameterType::List { min_items, .. } = &mut self.0.param_type {
            *min_items = Some(n);
        }
        self
    }

    /// Set maximum number of items.
    #[must_use]
    pub fn max_items(mut self, n: u32) -> Self {
        if let ParameterType::List { max_items, .. } = &mut self.0.param_type {
            *max_items = Some(n);
        }
        self
    }
}

/// Builder for nested object parameters.
pub struct ObjectBuilder {
    id: String,
    inner: ParameterCollectionBuilder,
    label: Option<String>,
    description: Option<String>,
    visible_when: Option<Condition>,
    required_when: Option<Condition>,
    disabled_when: Option<Condition>,
    expression: bool,
    group: Option<String>,
}

impl ObjectBuilder {
    fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            inner: ParameterCollectionBuilder::new(),
            label: None,
            description: None,
            visible_when: None,
            required_when: None,
            disabled_when: None,
            expression: true,
            group: None,
        }
    }

    /// Set the display label.
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Set description.
    #[must_use]
    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Set visibility condition.
    #[must_use]
    pub fn visible_when(mut self, condition: Condition) -> Self {
        self.visible_when = Some(condition);
        self
    }

    /// Set conditional required.
    #[must_use]
    pub fn required_when(mut self, condition: Condition) -> Self {
        self.required_when = Some(condition);
        self
    }

    /// Disable expression mode.
    #[must_use]
    pub fn no_expression(mut self) -> Self {
        self.expression = false;
        self
    }

    /// Add a nested string parameter.
    #[must_use]
    pub fn string(
        mut self,
        id: impl Into<String>,
        f: impl FnOnce(StringBuilder) -> StringBuilder,
    ) -> Self {
        self.inner = self.inner.string(id, f);
        self
    }

    /// Add a nested number parameter.
    #[must_use]
    pub fn number(
        mut self,
        id: impl Into<String>,
        f: impl FnOnce(NumberBuilder) -> NumberBuilder,
    ) -> Self {
        self.inner = self.inner.number(id, f);
        self
    }

    /// Add a nested select parameter.
    #[must_use]
    pub fn select(
        mut self,
        id: impl Into<String>,
        f: impl FnOnce(SelectBuilder) -> SelectBuilder,
    ) -> Self {
        self.inner = self.inner.select(id, f);
        self
    }

    /// Add a nested boolean parameter.
    #[must_use]
    pub fn boolean(
        mut self,
        id: impl Into<String>,
        f: impl FnOnce(BooleanBuilder) -> BooleanBuilder,
    ) -> Self {
        self.inner = self.inner.boolean(id, f);
        self
    }

    fn into_parameter(self) -> Parameter {
        let nested = self.inner.build();
        let mut p = Parameter::object_with(&self.id, nested.into_vec());
        p.label = self.label;
        p.description = self.description;
        p.visible_when = self.visible_when;
        p.required_when = self.required_when;
        p.disabled_when = self.disabled_when;
        p.expression = self.expression;
        p.group = self.group;
        p
    }
}

/// Builder for a group of parameters sharing a condition.
///
/// Parameters added inside a group automatically inherit the group's
/// `visible_when` condition and group name.
pub struct GroupBuilder {
    group_name: String,
    visible_when: Option<Condition>,
    params: Vec<Parameter>,
}

impl GroupBuilder {
    fn new(name: impl Into<String>) -> Self {
        Self {
            group_name: name.into(),
            visible_when: None,
            params: Vec::new(),
        }
    }

    /// Set visibility condition for all parameters in this group.
    #[must_use]
    pub fn visible_when(mut self, condition: Condition) -> Self {
        self.visible_when = Some(condition);
        self
    }

    /// Add a string parameter to this group.
    #[must_use]
    pub fn string(
        mut self,
        id: impl Into<String>,
        f: impl FnOnce(StringBuilder) -> StringBuilder,
    ) -> Self {
        let mut b = StringBuilder(Parameter::string(id));
        b = f(b);
        self.push(b.0);
        self
    }

    /// Add a number parameter to this group.
    #[must_use]
    pub fn number(
        mut self,
        id: impl Into<String>,
        f: impl FnOnce(NumberBuilder) -> NumberBuilder,
    ) -> Self {
        let mut b = NumberBuilder(Parameter::number(id));
        b = f(b);
        self.push(b.0);
        self
    }

    /// Add a select parameter to this group.
    #[must_use]
    pub fn select(
        mut self,
        id: impl Into<String>,
        f: impl FnOnce(SelectBuilder) -> SelectBuilder,
    ) -> Self {
        let mut b = SelectBuilder(Parameter::select(id));
        b = f(b);
        self.push(b.0);
        self
    }

    /// Add a boolean parameter to this group.
    #[must_use]
    pub fn boolean(
        mut self,
        id: impl Into<String>,
        f: impl FnOnce(BooleanBuilder) -> BooleanBuilder,
    ) -> Self {
        let mut b = BooleanBuilder(Parameter::boolean(id));
        b = f(b);
        self.push(b.0);
        self
    }

    fn push(&mut self, mut param: Parameter) {
        // Inherit group name
        if param.group.is_none() {
            param.group = Some(self.group_name.clone());
        }
        // Inherit visible_when if not explicitly set
        if param.visible_when.is_none() {
            param.visible_when = self.visible_when.clone();
        }
        self.params.push(param);
    }
}

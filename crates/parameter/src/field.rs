use crate::conditions::Condition;
use crate::loader::{OptionLoader, RecordLoader};
use crate::metadata::FieldMetadata;
use crate::option::OptionSource;
use crate::rules::Rule;
use crate::spec::{DynamicRecordMode, FieldSpec, ModeVariant, PredicateOp, UnknownFieldPolicy};

fn default_true() -> bool {
    true
}

fn default_depth() -> u8 {
    3
}

/// Canonical schema field.
///
/// Every variant carries [`FieldMetadata`] flattened into the wire envelope so
/// that `id`, `label`, and all shared metadata appear at the same JSON level
/// as the `"type"` discriminator.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Field {
    /// Free-form text field.
    Text {
        #[serde(flatten)]
        meta: FieldMetadata,
        /// Render as a multi-line textarea.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        multiline: bool,
    },
    /// Numeric field.
    Number {
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
    /// Boolean toggle field.
    Boolean {
        #[serde(flatten)]
        meta: FieldMetadata,
    },
    /// Select field with static or dynamic options.
    Select {
        #[serde(flatten)]
        meta: FieldMetadata,
        /// Option source (static inline list or dynamic provider).
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
        ///
        /// When set, the engine calls this async loader to resolve options.
        #[serde(skip)]
        loader: Option<OptionLoader>,
    },
    /// Nested object field containing ordered sub-fields.
    Object {
        #[serde(flatten)]
        meta: FieldMetadata,
        /// Ordered sub-field definitions.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        fields: Vec<Field>,
    },
    /// Repeated field using an item template.
    List {
        #[serde(flatten)]
        meta: FieldMetadata,
        /// Template for each list item.
        item: Box<Field>,
        /// Minimum number of items.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min_items: Option<u32>,
        /// Maximum number of items.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_items: Option<u32>,
    },
    /// Discriminated-union field with named mode variants.
    Mode {
        #[serde(flatten)]
        meta: FieldMetadata,
        /// Available mode variants.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        variants: Vec<ModeVariant>,
        /// Key of the variant shown by default.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default_variant: Option<String>,
    },
    /// Hidden field: stored value with no visible editor.
    Hidden {
        #[serde(flatten)]
        meta: FieldMetadata,
    },
    /// Syntax-highlighted code editor field.
    Code {
        #[serde(flatten)]
        meta: FieldMetadata,
        /// Language hint for syntax highlighting (e.g. `"json"`, `"python"`).
        language: String,
    },
    /// Colour picker field (emits a hex/rgb/hsl string).
    Color {
        #[serde(flatten)]
        meta: FieldMetadata,
    },
    /// Calendar date picker (emits an ISO 8601 date string).
    Date {
        #[serde(flatten)]
        meta: FieldMetadata,
    },
    /// Date-and-time picker (emits an ISO 8601 datetime string).
    DateTime {
        #[serde(flatten)]
        meta: FieldMetadata,
    },
    /// Wall-clock time picker (emits `HH:MM` or `HH:MM:SS`).
    Time {
        #[serde(flatten)]
        meta: FieldMetadata,
    },
    /// File attachment field.
    File {
        #[serde(flatten)]
        meta: FieldMetadata,
        /// MIME-type or extension filter (e.g. `"image/*"`, `".pdf"`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        accept: Option<String>,
        /// Maximum upload size in bytes.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_size: Option<u64>,
        /// Allow selecting multiple files.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        multiple: bool,
    },
    /// Field set whose sub-fields are resolved at runtime by a provider or inline loader.
    DynamicFields {
        #[serde(flatten)]
        meta: FieldMetadata,
        /// Provider key registered in the runtime registry.
        provider: String,
        /// Field ids forwarded to the provider as context.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        depends_on: Vec<String>,
        /// How many provider fields are shown initially.
        #[serde(default)]
        mode: DynamicRecordMode,
        /// Policy for unknown fields returned by the provider.
        #[serde(default)]
        unknown_field_policy: UnknownFieldPolicy,
        /// Inline record loader; skipped during serialization.
        ///
        /// When set, the engine calls this function to resolve [`FieldSpec`]s
        /// instead of (or before) consulting a global provider registry.
        #[serde(skip)]
        loader: Option<RecordLoader>,
    },
    /// Visual condition-builder field that emits a [`PredicateExpr`].
    Filter {
        #[serde(flatten)]
        meta: FieldMetadata,
        /// Restrict available operators; `None` means allow all.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        operators: Option<Vec<PredicateOp>>,
        /// Allow rule groups (nested AND/OR logic).
        #[serde(default = "crate::field::default_true")]
        allow_groups: bool,
        /// Maximum nesting depth for predicate groups.
        #[serde(default = "crate::field::default_depth")]
        max_depth: u8,
    },
}

impl Field {
    /// Creates a text field with the given id.
    #[must_use]
    pub fn text(id: impl Into<String>) -> Self {
        Self::Text {
            meta: FieldMetadata::new(id),
            multiline: false,
        }
    }

    /// Creates a decimal number field.
    #[must_use]
    pub fn number(id: impl Into<String>) -> Self {
        Self::Number {
            meta: FieldMetadata::new(id),
            integer: false,
            min: None,
            max: None,
            step: None,
        }
    }

    /// Creates an integer-only number field.
    #[must_use]
    pub fn integer(id: impl Into<String>) -> Self {
        Self::Number {
            meta: FieldMetadata::new(id),
            integer: true,
            min: None,
            max: None,
            step: None,
        }
    }

    /// Creates a boolean field.
    #[must_use]
    pub fn boolean(id: impl Into<String>) -> Self {
        Self::Boolean {
            meta: FieldMetadata::new(id),
        }
    }

    /// Creates a static select field.
    #[must_use]
    pub fn select(id: impl Into<String>) -> Self {
        Self::Select {
            meta: FieldMetadata::new(id),
            source: OptionSource::Static {
                options: Vec::new(),
            },
            multiple: false,
            allow_custom: false,
            searchable: false,
            loader: None,
        }
    }

    /// Creates a dynamic-fields field backed by the given provider key.
    #[must_use]
    pub fn dynamic_fields(id: impl Into<String>, provider: impl Into<String>) -> Self {
        Self::DynamicFields {
            meta: FieldMetadata::new(id),
            provider: provider.into(),
            depends_on: Vec::new(),
            mode: DynamicRecordMode::default(),
            unknown_field_policy: UnknownFieldPolicy::default(),
            loader: None,
        }
    }

    // -- Fluent meta-setters -------------------------------------------------

    /// Returns a mutable reference to the shared [`FieldMetadata`].
    #[must_use]
    pub fn meta_mut(&mut self) -> &mut FieldMetadata {
        match self {
            Self::Text { meta, .. }
            | Self::Number { meta, .. }
            | Self::Boolean { meta, .. }
            | Self::Select { meta, .. }
            | Self::Object { meta, .. }
            | Self::List { meta, .. }
            | Self::Mode { meta, .. }
            | Self::Hidden { meta, .. }
            | Self::Code { meta, .. }
            | Self::Color { meta, .. }
            | Self::Date { meta, .. }
            | Self::DateTime { meta, .. }
            | Self::Time { meta, .. }
            | Self::File { meta, .. }
            | Self::DynamicFields { meta, .. }
            | Self::Filter { meta, .. } => meta,
        }
    }

    /// Returns a shared reference to the [`FieldMetadata`].
    #[must_use]
    pub fn meta(&self) -> &FieldMetadata {
        match self {
            Self::Text { meta, .. }
            | Self::Number { meta, .. }
            | Self::Boolean { meta, .. }
            | Self::Select { meta, .. }
            | Self::Object { meta, .. }
            | Self::List { meta, .. }
            | Self::Mode { meta, .. }
            | Self::Hidden { meta, .. }
            | Self::Code { meta, .. }
            | Self::Color { meta, .. }
            | Self::Date { meta, .. }
            | Self::DateTime { meta, .. }
            | Self::Time { meta, .. }
            | Self::File { meta, .. }
            | Self::DynamicFields { meta, .. }
            | Self::Filter { meta, .. } => meta,
        }
    }

    /// Sets the display label.
    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.meta_mut().set_label(label);
        self
    }

    /// Sets the description tooltip.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.meta_mut().set_description(description);
        self
    }

    /// Sets the placeholder text.
    #[must_use]
    pub fn with_placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.meta_mut().set_placeholder(placeholder);
        self
    }

    /// Sets the hint text.
    #[must_use]
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.meta_mut().set_hint(hint);
        self
    }

    /// Makes the field required.
    #[must_use]
    pub fn required(mut self) -> Self {
        self.meta_mut().set_required(true);
        self
    }

    /// Marks the field value as secret / masked.
    #[must_use]
    pub fn secret(mut self) -> Self {
        self.meta_mut().set_secret(true);
        self
    }

    /// Sets the default JSON value.
    #[must_use]
    pub fn with_default(mut self, value: serde_json::Value) -> Self {
        self.meta_mut().set_default(value);
        self
    }

    /// Appends a validation rule.
    #[must_use]
    pub fn with_rule(mut self, rule: Rule) -> Self {
        self.meta_mut().add_rule(rule);
        self
    }

    /// Sets field visibility condition.
    #[must_use]
    pub fn visible_when(mut self, condition: Condition) -> Self {
        self.meta_mut().set_visible_when(condition);
        self
    }

    /// Sets conditional-required rule.
    #[must_use]
    pub fn required_when(mut self, condition: Condition) -> Self {
        self.meta_mut().set_required_when(condition);
        self
    }

    /// Sets disabled/read-only condition.
    #[must_use]
    pub fn disabled_when(mut self, condition: Condition) -> Self {
        self.meta_mut().set_disabled_when(condition);
        self
    }

    // -- Loader setters ------------------------------------------------------

    /// Attaches an async inline option loader to a [`Field::Select`] variant.
    ///
    /// The closure receives a [`crate::loader::LoaderCtx`] by value and must
    /// return a future that resolves to a `Vec<SelectOption>`.
    ///
    /// Panics if called on a non-`Select` variant.
    #[must_use]
    pub fn with_option_loader<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(crate::loader::LoaderCtx) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Vec<crate::option::SelectOption>> + Send + 'static,
    {
        if let Self::Select { loader, .. } = &mut self {
            *loader = Some(OptionLoader::new(f));
        } else {
            panic!("with_option_loader called on a non-Select Field variant");
        }
        self
    }

    /// Attaches an async inline record loader to a [`Field::DynamicFields`] variant.
    ///
    /// The closure receives a [`crate::loader::LoaderCtx`] by value and must
    /// return a future that resolves to a `Vec<FieldSpec>`.
    ///
    /// Panics if called on a non-`DynamicFields` variant.
    #[must_use]
    pub fn with_record_loader<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(crate::loader::LoaderCtx) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Vec<FieldSpec>> + Send + 'static,
    {
        if let Self::DynamicFields { loader, .. } = &mut self {
            *loader = Some(RecordLoader::new(f));
        } else {
            panic!("with_record_loader called on a non-DynamicFields Field variant");
        }
        self
    }

    // -- Loader accessors ----------------------------------------------------

    /// Returns a reference to the attached [`OptionLoader`], if any.
    ///
    /// Returns `Some` only when `self` is a [`Field::Select`] variant with a
    /// loader attached.
    pub fn option_loader(&self) -> Option<&OptionLoader> {
        if let Self::Select { loader, .. } = self {
            loader.as_ref()
        } else {
            None
        }
    }

    /// Returns a reference to the attached [`RecordLoader`], if any.
    ///
    /// Returns `Some` only when `self` is a [`Field::DynamicFields`] variant
    /// with a loader attached.
    pub fn record_loader(&self) -> Option<&RecordLoader> {
        if let Self::DynamicFields { loader, .. } = self {
            loader.as_ref()
        } else {
            None
        }
    }
}

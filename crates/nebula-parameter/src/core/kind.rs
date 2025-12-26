use serde::{Deserialize, Serialize};

/// UI widget type for parameter rendering
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParameterKind {
    // Input Parameters
    Text,
    Textarea,
    Secret,
    Number,
    Checkbox,

    // Selection Parameters
    Select,
    MultiSelect,
    Radio,

    // Date/Time Parameters
    DateTime,
    Date,
    Time,

    // Specialized Input
    Code,
    File,
    Color,
    Resource,

    // Container Parameters
    Group,
    Object,
    List,
    Routing,
    Mode,
    Panel,

    // Special Parameters
    Hidden,
    Notice,
    Expirable,
}

/// Capabilities that a parameter kind can have
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParameterCapability {
    HasValue,
    Editable,
    Validatable,
    Displayable,
    Requirable,
    SupportsExpressions,
    Container,
    Interactive,
    Serializable,
}

impl ParameterKind {
    /// Get string representation
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Textarea => "textarea",
            Self::Secret => "secret",
            Self::Number => "number",
            Self::Checkbox => "checkbox",
            Self::Select => "select",
            Self::MultiSelect => "multiselect",
            Self::Radio => "radio",
            Self::DateTime => "datetime",
            Self::Date => "date",
            Self::Time => "time",
            Self::Code => "code",
            Self::File => "file",
            Self::Color => "color",
            Self::Resource => "resource",
            Self::Group => "group",
            Self::Object => "object",
            Self::List => "list",
            Self::Routing => "routing",
            Self::Mode => "mode",
            Self::Panel => "panel",
            Self::Hidden => "hidden",
            Self::Notice => "notice",
            Self::Expirable => "expirable",
        }
    }

    /// Get capabilities for this kind
    #[must_use]
    pub fn capabilities(&self) -> &'static [ParameterCapability] {
        use ParameterCapability::{
            Container, Displayable, Editable, HasValue, Interactive, Requirable, Serializable,
            SupportsExpressions, Validatable,
        };

        match self {
            // Input parameters - full capabilities
            Self::Text
            | Self::Textarea
            | Self::Secret
            | Self::Number
            | Self::Select
            | Self::MultiSelect
            | Self::Radio
            | Self::DateTime
            | Self::Date
            | Self::Time
            | Self::Code
            | Self::File
            | Self::Color
            | Self::Resource => &[
                HasValue,
                Editable,
                Validatable,
                Displayable,
                Requirable,
                SupportsExpressions,
                Serializable,
            ],

            // Boolean types - simpler
            Self::Checkbox => &[
                HasValue,
                Editable,
                Displayable,
                Requirable,
                SupportsExpressions,
                Serializable,
            ],

            // Container parameters
            Self::Group => &[HasValue, Container, Displayable, Serializable],

            Self::Object | Self::List => &[
                HasValue,
                Container,
                Validatable,
                Displayable,
                Requirable,
                SupportsExpressions,
                Serializable,
            ],

            Self::Routing => &[
                HasValue,
                Container,
                Editable,
                Displayable,
                Interactive,
                Serializable,
            ],

            Self::Mode => &[
                HasValue,
                Container,
                Editable,
                Displayable,
                SupportsExpressions,
                Serializable,
            ],

            Self::Panel => &[Container, Displayable, Serializable],

            // Special parameters
            Self::Hidden => &[HasValue, SupportsExpressions, Serializable],

            Self::Notice => &[Displayable],

            Self::Expirable => &[HasValue, Container, SupportsExpressions, Serializable],
        }
    }

    /// Check if this kind has a specific capability
    #[inline]
    #[must_use]
    pub fn has_capability(&self, capability: ParameterCapability) -> bool {
        self.capabilities().contains(&capability)
    }

    /// Convenience methods
    #[inline]
    #[must_use]
    pub fn has_value(&self) -> bool {
        self.has_capability(ParameterCapability::HasValue)
    }

    #[inline]
    #[must_use]
    pub fn is_editable(&self) -> bool {
        self.has_capability(ParameterCapability::Editable)
    }

    #[inline]
    #[must_use]
    pub fn is_validatable(&self) -> bool {
        self.has_capability(ParameterCapability::Validatable)
    }

    #[inline]
    #[must_use]
    pub fn is_displayable(&self) -> bool {
        self.has_capability(ParameterCapability::Displayable)
    }

    #[inline]
    #[must_use]
    pub fn is_requirable(&self) -> bool {
        self.has_capability(ParameterCapability::Requirable)
    }

    #[inline]
    #[must_use]
    pub fn supports_expressions(&self) -> bool {
        self.has_capability(ParameterCapability::SupportsExpressions)
    }

    #[inline]
    #[must_use]
    pub fn is_container(&self) -> bool {
        self.has_capability(ParameterCapability::Container)
    }

    /// Check if this is a text-based input
    #[must_use]
    pub fn is_text_based(&self) -> bool {
        matches!(
            self,
            Self::Text | Self::Textarea | Self::Secret | Self::Code
        )
    }

    /// Check if this is a selection-based input
    #[must_use]
    pub fn is_selection_based(&self) -> bool {
        matches!(self, Self::Select | Self::MultiSelect | Self::Radio)
    }

    /// Check if this is a temporal input
    #[must_use]
    pub fn is_temporal(&self) -> bool {
        matches!(self, Self::Date | Self::Time | Self::DateTime)
    }

    /// Check if this requires options
    #[must_use]
    pub fn requires_options(&self) -> bool {
        self.is_selection_based()
    }

    /// Get the corresponding value type
    #[must_use]
    pub fn value_type(&self) -> &'static str {
        match self {
            Self::Text | Self::Textarea | Self::Secret | Self::Code | Self::Color => "String",
            Self::Number => "Number",
            Self::Checkbox => "Boolean",
            Self::Select | Self::Radio | Self::Resource => "String",
            Self::MultiSelect => "Array",
            Self::DateTime | Self::Date | Self::Time => "DateTime",
            Self::File => "File",
            Self::Object | Self::Group => "Object",
            Self::List | Self::Routing => "Array",
            Self::Mode => "Any",
            Self::Panel => "None",
            Self::Hidden => "Any",
            Self::Notice => "None",
            Self::Expirable => "Any",
        }
    }

    /// Parse from string
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "text" => Some(Self::Text),
            "textarea" => Some(Self::Textarea),
            "secret" => Some(Self::Secret),
            "number" => Some(Self::Number),
            "checkbox" => Some(Self::Checkbox),
            "select" => Some(Self::Select),
            "multiselect" => Some(Self::MultiSelect),
            "radio" => Some(Self::Radio),
            "datetime" => Some(Self::DateTime),
            "date" => Some(Self::Date),
            "time" => Some(Self::Time),
            "code" => Some(Self::Code),
            "file" => Some(Self::File),
            "color" => Some(Self::Color),
            "resource" => Some(Self::Resource),
            "group" => Some(Self::Group),
            "object" => Some(Self::Object),
            "list" => Some(Self::List),
            "routing" => Some(Self::Routing),
            "mode" => Some(Self::Mode),
            "panel" => Some(Self::Panel),
            "hidden" => Some(Self::Hidden),
            "notice" => Some(Self::Notice),
            "expirable" => Some(Self::Expirable),
            _ => None,
        }
    }

    /// Get all variants
    #[must_use]
    pub fn all() -> &'static [Self] {
        &[
            Self::Text,
            Self::Textarea,
            Self::Secret,
            Self::Number,
            Self::Checkbox,
            Self::Select,
            Self::MultiSelect,
            Self::Radio,
            Self::DateTime,
            Self::Date,
            Self::Time,
            Self::Code,
            Self::File,
            Self::Color,
            Self::Resource,
            Self::Group,
            Self::Object,
            Self::List,
            Self::Routing,
            Self::Mode,
            Self::Panel,
            Self::Hidden,
            Self::Notice,
            Self::Expirable,
        ]
    }

    /// Get input kinds
    #[must_use]
    pub fn input_kinds() -> &'static [Self] {
        &[
            Self::Text,
            Self::Textarea,
            Self::Secret,
            Self::Number,
            Self::Checkbox,
            Self::Select,
            Self::MultiSelect,
            Self::Radio,
            Self::DateTime,
            Self::Date,
            Self::Time,
            Self::Code,
            Self::File,
            Self::Color,
            Self::Resource,
        ]
    }

    /// Get container kinds
    #[must_use]
    pub fn container_kinds() -> &'static [Self] {
        &[
            Self::Group,
            Self::Object,
            Self::List,
            Self::Routing,
            Self::Mode,
            Self::Panel,
        ]
    }
}

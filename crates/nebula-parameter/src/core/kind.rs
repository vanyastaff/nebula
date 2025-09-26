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
    Boolean,
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

    // Special Parameters
    Hidden,
    Notice,
    Button,
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
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Textarea => "textarea",
            Self::Secret => "secret",
            Self::Number => "number",
            Self::Boolean => "boolean",
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
            Self::Hidden => "hidden",
            Self::Notice => "notice",
            Self::Button => "button",
            Self::Expirable => "expirable",
        }
    }

    /// Get capabilities for this kind
    pub fn capabilities(&self) -> &'static [ParameterCapability] {
        use ParameterCapability::*;

        match self {
            // Input parameters - full capabilities
            Self::Text | Self::Textarea | Self::Secret | Self::Number |
            Self::Select | Self::MultiSelect | Self::Radio |
            Self::DateTime | Self::Date | Self::Time |
            Self::Code | Self::File | Self::Color | Self::Resource => {
                &[HasValue, Editable, Validatable, Displayable, Requirable, SupportsExpressions, Serializable]
            }

            // Boolean types - simpler
            Self::Boolean | Self::Checkbox => {
                &[HasValue, Editable, Displayable, Requirable, SupportsExpressions, Serializable]
            }

            // Container parameters
            Self::Group => {
                &[HasValue, Container, Displayable, Serializable]
            }

            Self::Object | Self::List => {
                &[HasValue, Container, Validatable, Displayable, Requirable, SupportsExpressions, Serializable]
            }

            Self::Routing => {
                &[HasValue, Container, Editable, Displayable, Interactive, Serializable]
            }

            Self::Mode => {
                &[HasValue, Container, Editable, Displayable, SupportsExpressions, Serializable]
            }

            // Special parameters
            Self::Hidden => {
                &[HasValue, SupportsExpressions, Serializable]
            }

            Self::Notice => {
                &[Displayable]
            }

            Self::Button => {
                &[Displayable, Interactive]
            }

            Self::Expirable => {
                &[HasValue, Container, SupportsExpressions, Serializable]
            }
        }
    }

    /// Check if this kind has a specific capability
    #[inline]
    pub fn has_capability(&self, capability: ParameterCapability) -> bool {
        self.capabilities().contains(&capability)
    }

    /// Convenience methods
    #[inline]
    pub fn has_value(&self) -> bool {
        self.has_capability(ParameterCapability::HasValue)
    }

    #[inline]
    pub fn is_editable(&self) -> bool {
        self.has_capability(ParameterCapability::Editable)
    }

    #[inline]
    pub fn is_validatable(&self) -> bool {
        self.has_capability(ParameterCapability::Validatable)
    }

    #[inline]
    pub fn is_displayable(&self) -> bool {
        self.has_capability(ParameterCapability::Displayable)
    }

    #[inline]
    pub fn is_requirable(&self) -> bool {
        self.has_capability(ParameterCapability::Requirable)
    }

    #[inline]
    pub fn supports_expressions(&self) -> bool {
        self.has_capability(ParameterCapability::SupportsExpressions)
    }

    #[inline]
    pub fn is_container(&self) -> bool {
        self.has_capability(ParameterCapability::Container)
    }

    /// Check if this is a text-based input
    pub fn is_text_based(&self) -> bool {
        matches!(self, Self::Text | Self::Textarea | Self::Secret | Self::Code)
    }

    /// Check if this is a selection-based input
    pub fn is_selection_based(&self) -> bool {
        matches!(self, Self::Select | Self::MultiSelect | Self::Radio)
    }

    /// Check if this is a temporal input
    pub fn is_temporal(&self) -> bool {
        matches!(self, Self::Date | Self::Time | Self::DateTime)
    }

    /// Check if this requires options
    pub fn requires_options(&self) -> bool {
        self.is_selection_based()
    }

    /// Get the corresponding value type
    pub fn value_type(&self) -> &'static str {
        match self {
            Self::Text | Self::Textarea | Self::Secret | Self::Code | Self::Color => "String",
            Self::Number => "Number",
            Self::Boolean | Self::Checkbox => "Boolean",
            Self::Select | Self::Radio | Self::Resource => "String",
            Self::MultiSelect => "Array",
            Self::DateTime | Self::Date | Self::Time => "DateTime",
            Self::File => "File",
            Self::Object | Self::Group => "Object",
            Self::List | Self::Routing => "Array",
            Self::Mode => "Any",
            Self::Hidden => "Any",
            Self::Notice => "None",
            Self::Button => "None",
            Self::Expirable => "Any",
        }
    }

    /// Parse from string
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "text" => Some(Self::Text),
            "textarea" => Some(Self::Textarea),
            "secret" => Some(Self::Secret),
            "number" => Some(Self::Number),
            "boolean" => Some(Self::Boolean),
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
            "hidden" => Some(Self::Hidden),
            "notice" => Some(Self::Notice),
            "button" => Some(Self::Button),
            "expirable" => Some(Self::Expirable),
            _ => None,
        }
    }

    /// Get all variants
    pub fn all() -> &'static [Self] {
        &[
            Self::Text, Self::Textarea, Self::Secret, Self::Number,
            Self::Boolean, Self::Checkbox, Self::Select, Self::MultiSelect,
            Self::Radio, Self::DateTime, Self::Date, Self::Time,
            Self::Code, Self::File, Self::Color, Self::Resource,
            Self::Group, Self::Object, Self::List, Self::Routing,
            Self::Mode, Self::Hidden, Self::Notice, Self::Button, Self::Expirable,
        ]
    }

    /// Get input kinds
    pub fn input_kinds() -> &'static [Self] {
        &[
            Self::Text, Self::Textarea, Self::Secret, Self::Number,
            Self::Boolean, Self::Checkbox, Self::Select, Self::MultiSelect,
            Self::Radio, Self::DateTime, Self::Date, Self::Time,
            Self::Code, Self::File, Self::Color, Self::Resource,
        ]
    }

    /// Get container kinds
    pub fn container_kinds() -> &'static [Self] {
        &[Self::Group, Self::Object, Self::List, Self::Routing, Self::Mode]
    }
}
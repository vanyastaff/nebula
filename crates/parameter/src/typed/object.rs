//! Generic Object parameter for grouped fields.

use serde::{Deserialize, Serialize};

use crate::def::ParameterDef;
use crate::display::ParameterDisplay;
use crate::metadata::ParameterMetadata;
use crate::types::object::ObjectOptions;
use crate::validation::ValidationRule;

/// A structured object parameter with named child fields.
///
/// ## Example
///
/// ```
/// use nebula_parameter::typed::{Object, Text, Number, Plain, Port};
/// use nebula_parameter::def::ParameterDef;
///
/// let db_config = Object::builder("database")
///     .label("Database Configuration")
///     .field(Text::<Plain>::builder("host")
///         .label("Host")
///         .default_value("localhost")
///         .build()
///         .into())
///     .field(Number::<Port>::builder("port")
///         .label("Port")
///         .default_value(5432)
///         .build()
///         .into())
///     .collapsible(true)
///     .build();
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Object {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    /// The child parameters that make up this object.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<ParameterDef>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<ObjectOptions>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validation: Vec<ValidationRule>,
}

impl Object {
    /// Create a new object parameter builder.
    #[must_use]
    pub fn builder(key: impl Into<String>) -> ObjectBuilder {
        ObjectBuilder::new(key)
    }

    /// Create a minimal object parameter.
    #[must_use]
    pub fn new(key: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            metadata: ParameterMetadata::new(key, name),
            fields: Vec::new(),
            default: None,
            options: None,
            display: None,
            validation: Vec::new(),
        }
    }
}

/// Builder for Object parameters.
#[derive(Debug)]
pub struct ObjectBuilder {
    metadata: ParameterMetadata,
    fields: Vec<ParameterDef>,
    default: Option<serde_json::Value>,
    options: Option<ObjectOptions>,
    display: Option<ParameterDisplay>,
    validation: Vec<ValidationRule>,
}

impl ObjectBuilder {
    fn new(key: impl Into<String>) -> Self {
        Self {
            metadata: ParameterMetadata::new(key, ""),
            fields: Vec::new(),
            default: None,
            options: None,
            display: None,
            validation: Vec::new(),
        }
    }

    /// Set the display label.
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.metadata.name = label.into();
        self
    }

    /// Set the description.
    #[must_use]
    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.metadata.description = Some(desc.into());
        self
    }

    /// Mark as required.
    #[must_use]
    pub fn required(mut self) -> Self {
        self.metadata.required = true;
        self
    }

    /// Add a child field.
    #[must_use]
    pub fn field(mut self, field: ParameterDef) -> Self {
        self.fields.push(field);
        self
    }

    /// Set multiple fields at once.
    #[must_use]
    pub fn fields(mut self, fields: impl IntoIterator<Item = ParameterDef>) -> Self {
        self.fields.extend(fields);
        self
    }

    /// Enable collapsible UI.
    #[must_use]
    pub fn collapsible(mut self, collapsible: bool) -> Self {
        self.options
            .get_or_insert_with(ObjectOptions::default)
            .collapsible = collapsible;
        self
    }

    /// Start collapsed by default.
    #[must_use]
    pub fn collapsed_by_default(mut self, collapsed: bool) -> Self {
        self.options
            .get_or_insert_with(ObjectOptions::default)
            .collapsed_by_default = collapsed;
        self
    }

    /// Add a validation rule.
    #[must_use]
    pub fn validation(mut self, rule: ValidationRule) -> Self {
        self.validation.push(rule);
        self
    }

    /// Build the Object parameter.
    #[must_use]
    pub fn build(self) -> Object {
        let mut metadata = self.metadata;
        if metadata.name.is_empty() {
            metadata.name = metadata.key.clone();
        }

        Object {
            metadata,
            fields: self.fields,
            default: self.default,
            options: self.options,
            display: self.display,
            validation: self.validation,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_creates_object() {
        let obj = Object::builder("config")
            .label("Configuration")
            .collapsible(true)
            .required()
            .build();

        assert_eq!(obj.metadata.key, "config");
        assert_eq!(obj.metadata.name, "Configuration");
        assert!(obj.metadata.required);
        assert_eq!(obj.options.as_ref().unwrap().collapsible, true);
    }
}

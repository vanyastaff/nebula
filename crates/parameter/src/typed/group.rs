//! Generic Group parameter for UI-only visual grouping.

use serde::{Deserialize, Serialize};

use crate::def::ParameterDef;
use crate::display::ParameterDisplay;
use crate::metadata::ParameterMetadata;
use crate::types::group::GroupOptions;

/// A UI-only visual grouping of parameters.
///
/// Group carries no value — children's values are stored flat.
/// Use case: "Advanced Settings" collapsible section.
///
/// ## Example
///
/// ```
/// use nebula_parameter::typed::{Group, Text, Checkbox, Plain, Toggle};
///
/// let advanced = Group::builder("advanced")
///     .label("Advanced Settings")
///     .parameter(Text::<Plain>::builder("custom_field")
///         .label("Custom Field")
///         .build()
///         .into())
///     .parameter(Checkbox::<Toggle>::builder("debug_mode")
///         .label("Debug Mode")
///         .build()
///         .into())
///     .collapsible(true)
///     .collapsed_by_default(true)
///     .bordered(true)
///     .build();
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Group {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    /// The grouped child parameters.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<ParameterDef>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<GroupOptions>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,
}

impl Group {
    /// Create a new group parameter builder.
    #[must_use]
    pub fn builder(key: impl Into<String>) -> GroupBuilder {
        GroupBuilder::new(key)
    }

    /// Create a minimal group parameter.
    #[must_use]
    pub fn new(key: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            metadata: ParameterMetadata::new(key, name),
            parameters: Vec::new(),
            options: None,
            display: None,
        }
    }
}

/// Builder for Group parameters.
#[derive(Debug)]
pub struct GroupBuilder {
    metadata: ParameterMetadata,
    parameters: Vec<ParameterDef>,
    options: Option<GroupOptions>,
    display: Option<ParameterDisplay>,
}

impl GroupBuilder {
    fn new(key: impl Into<String>) -> Self {
        Self {
            metadata: ParameterMetadata::new(key, ""),
            parameters: Vec::new(),
            options: None,
            display: None,
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

    /// Add a child parameter.
    #[must_use]
    pub fn parameter(mut self, param: ParameterDef) -> Self {
        self.parameters.push(param);
        self
    }

    /// Set multiple parameters at once.
    #[must_use]
    pub fn parameters(mut self, params: impl IntoIterator<Item = ParameterDef>) -> Self {
        self.parameters.extend(params);
        self
    }

    /// Enable collapsible UI.
    #[must_use]
    pub fn collapsible(mut self, collapsible: bool) -> Self {
        self.options
            .get_or_insert_with(GroupOptions::default)
            .collapsible = collapsible;
        self
    }

    /// Start collapsed by default.
    #[must_use]
    pub fn collapsed_by_default(mut self, collapsed: bool) -> Self {
        self.options
            .get_or_insert_with(GroupOptions::default)
            .collapsed_by_default = collapsed;
        self
    }

    /// Show visible border.
    #[must_use]
    pub fn bordered(mut self, bordered: bool) -> Self {
        self.options
            .get_or_insert_with(GroupOptions::default)
            .bordered = bordered;
        self
    }

    /// Build the Group parameter.
    #[must_use]
    pub fn build(self) -> Group {
        let mut metadata = self.metadata;
        if metadata.name.is_empty() {
            metadata.name = metadata.key.clone();
        }

        Group {
            metadata,
            parameters: self.parameters,
            options: self.options,
            display: self.display,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_creates_group() {
        let group = Group::builder("advanced")
            .label("Advanced Settings")
            .collapsible(true)
            .bordered(true)
            .build();

        assert_eq!(group.metadata.key, "advanced");
        assert_eq!(group.metadata.name, "Advanced Settings");
        assert_eq!(group.options.as_ref().unwrap().collapsible, true);
        assert_eq!(group.options.as_ref().unwrap().bordered, true);
    }
}

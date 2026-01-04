//! Panel parameter type for organizing parameters into sections/tabs

use serde::{Deserialize, Serialize};

use crate::core::{
    Describable, Displayable, Parameter, ParameterDisplay, ParameterError, ParameterKind,
    ParameterMetadata, Validatable,
};
use nebula_value::Value;

/// Panel parameter - container for organizing parameters into sections/tabs
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_parameter::prelude::*;
///
/// let param = PanelParameter::builder()
///     .key("settings")
///     .name("Settings")
///     .description("Application settings")
///     .options(
///         PanelParameterOptions::builder()
///             .default_panel("general")
///             .allow_multiple_open(false)
///             .build()
///     )
///     .build()?;
/// ```
#[derive(Serialize)]
pub struct PanelParameter {
    /// Parameter metadata (key, name, description, etc.)
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    /// Panel sections with their parameters
    pub panels: Vec<Panel>,

    /// Configuration options for this parameter type
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<PanelParameterOptions>,

    /// Display conditions controlling when this parameter is shown
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,
}

/// A single panel section containing parameters
#[derive(Serialize)]
pub struct Panel {
    /// Unique key for this panel
    pub key: String,

    /// Display label for the panel
    pub label: String,

    /// Optional description for the panel
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Parameters contained in this panel
    #[serde(skip)]
    pub children: Vec<Box<dyn Parameter>>,

    /// Optional icon identifier
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,

    /// Whether this panel is enabled
    #[serde(default)]
    pub enabled: bool,
}

/// Configuration options for panel parameter
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PanelParameterOptions {
    /// Key of the default active panel
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_panel: Option<String>,

    /// Whether multiple panels can be open at once (accordion mode)
    #[serde(default)]
    pub allow_multiple_open: bool,
}

// =============================================================================
// PanelParameter Builder
// =============================================================================

/// Builder for `PanelParameter`
#[derive(Default)]
pub struct PanelParameterBuilder {
    // Metadata fields
    key: Option<String>,
    name: Option<String>,
    description: String,
    required: bool,
    placeholder: Option<String>,
    hint: Option<String>,
    // Parameter fields
    panels: Vec<Panel>,
    options: Option<PanelParameterOptions>,
    display: Option<ParameterDisplay>,
}

impl PanelParameter {
    /// Create a new builder
    #[must_use]
    pub fn builder() -> PanelParameterBuilder {
        PanelParameterBuilder::new()
    }
}

impl PanelParameterBuilder {
    /// Create a new builder
    #[must_use]
    pub fn new() -> Self {
        Self {
            key: None,
            name: None,
            description: String::new(),
            required: false,
            placeholder: None,
            hint: None,
            panels: Vec::new(),
            options: None,
            display: None,
        }
    }

    // -------------------------------------------------------------------------
    // Metadata methods
    // -------------------------------------------------------------------------

    /// Set the parameter key (required)
    #[must_use]
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Set the display name (required)
    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set the description
    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Set whether the parameter is required
    #[must_use]
    pub fn required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }

    /// Set placeholder text
    #[must_use]
    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = Some(placeholder.into());
        self
    }

    /// Set hint text
    #[must_use]
    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    // -------------------------------------------------------------------------
    // Parameter-specific methods
    // -------------------------------------------------------------------------

    /// Set the panels
    #[must_use]
    pub fn panels(mut self, panels: Vec<Panel>) -> Self {
        self.panels = panels;
        self
    }

    /// Add a single panel
    #[must_use]
    pub fn panel(mut self, panel: Panel) -> Self {
        self.panels.push(panel);
        self
    }

    /// Set the options
    #[must_use]
    pub fn options(mut self, options: PanelParameterOptions) -> Self {
        self.options = Some(options);
        self
    }

    /// Set display conditions
    #[must_use]
    pub fn display(mut self, display: ParameterDisplay) -> Self {
        self.display = Some(display);
        self
    }

    // -------------------------------------------------------------------------
    // Build
    // -------------------------------------------------------------------------

    /// Build the `PanelParameter`
    ///
    /// # Errors
    ///
    /// Returns error if required fields are missing or key format is invalid.
    pub fn build(self) -> Result<PanelParameter, ParameterError> {
        let metadata = ParameterMetadata::builder()
            .key(
                self.key
                    .ok_or_else(|| ParameterError::BuilderMissingField {
                        field: "key".into(),
                    })?,
            )
            .name(
                self.name
                    .ok_or_else(|| ParameterError::BuilderMissingField {
                        field: "name".into(),
                    })?,
            )
            .description(self.description)
            .required(self.required)
            .build()?;

        let mut metadata = metadata;
        metadata.placeholder = self.placeholder;
        metadata.hint = self.hint;

        Ok(PanelParameter {
            metadata,
            panels: self.panels,
            options: self.options,
            display: self.display,
        })
    }
}

// =============================================================================
// PanelParameterOptions Builder
// =============================================================================

/// Builder for `PanelParameterOptions`
#[derive(Debug, Default)]
pub struct PanelParameterOptionsBuilder {
    default_panel: Option<String>,
    allow_multiple_open: bool,
}

impl PanelParameterOptions {
    /// Create a new builder
    #[must_use]
    pub fn builder() -> PanelParameterOptionsBuilder {
        PanelParameterOptionsBuilder::default()
    }
}

impl PanelParameterOptionsBuilder {
    /// Set the default active panel
    #[must_use]
    pub fn default_panel(mut self, default_panel: impl Into<String>) -> Self {
        self.default_panel = Some(default_panel.into());
        self
    }

    /// Set whether multiple panels can be open at once
    #[must_use]
    pub fn allow_multiple_open(mut self, allow: bool) -> Self {
        self.allow_multiple_open = allow;
        self
    }

    /// Build the options
    #[must_use]
    pub fn build(self) -> PanelParameterOptions {
        PanelParameterOptions {
            default_panel: self.default_panel,
            allow_multiple_open: self.allow_multiple_open,
        }
    }
}

// =============================================================================
// Panel Implementation
// =============================================================================

impl std::fmt::Debug for Panel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Panel")
            .field("key", &self.key)
            .field("label", &self.label)
            .field("description", &self.description)
            .field("children", &format!("{} children", self.children.len()))
            .field("icon", &self.icon)
            .field("enabled", &self.enabled)
            .finish()
    }
}

impl std::fmt::Debug for PanelParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PanelParameter")
            .field("metadata", &self.metadata)
            .field("panels", &self.panels)
            .field("options", &self.options)
            .field("display", &self.display)
            .finish()
    }
}

impl Panel {
    /// Create a new panel
    pub fn new(key: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            description: None,
            children: Vec::new(),
            icon: None,
            enabled: true,
        }
    }

    /// Set the description
    #[must_use = "builder methods must be chained or built"]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Set the icon
    #[must_use = "builder methods must be chained or built"]
    pub fn with_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Add a child parameter
    #[must_use = "builder methods must be chained or built"]
    pub fn with_child(mut self, child: Box<dyn Parameter>) -> Self {
        self.children.push(child);
        self
    }

    /// Add multiple child parameters
    #[must_use = "builder methods must be chained or built"]
    pub fn with_children(mut self, children: Vec<Box<dyn Parameter>>) -> Self {
        self.children.extend(children);
        self
    }

    /// Set enabled state
    #[must_use = "builder methods must be chained or built"]
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Get the number of children in this panel
    #[must_use]
    pub fn child_count(&self) -> usize {
        self.children.len()
    }

    /// Check if this panel is empty
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.children.is_empty()
    }
}

// =============================================================================
// Trait Implementations
// =============================================================================

impl Describable for PanelParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::Panel
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
    }
}

// Panel parameters implement minimal Validatable for blanket Parameter impl
impl Validatable for PanelParameter {
    fn is_empty(&self, _value: &Value) -> bool {
        self.panels.is_empty() // Panel is empty if it has no panels
    }
}

impl std::fmt::Display for PanelParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PanelParameter({})", self.metadata.name)
    }
}

impl Displayable for PanelParameter {
    fn display(&self) -> Option<&ParameterDisplay> {
        self.display.as_ref()
    }

    fn set_display(&mut self, display: Option<ParameterDisplay>) {
        self.display = display;
    }
}

impl PanelParameter {
    /// Add a panel
    pub fn add_panel(&mut self, panel: Panel) {
        self.panels.push(panel);
    }

    /// Get a panel by key
    #[must_use]
    pub fn get_panel(&self, key: &str) -> Option<&Panel> {
        self.panels.iter().find(|p| p.key == key)
    }

    /// Get a mutable panel by key
    pub fn get_panel_mut(&mut self, key: &str) -> Option<&mut Panel> {
        self.panels.iter_mut().find(|p| p.key == key)
    }

    /// Get all panel keys
    #[must_use]
    pub fn get_panel_keys(&self) -> Vec<&str> {
        self.panels.iter().map(|p| p.key.as_str()).collect()
    }

    /// Get the default active panel key
    #[must_use]
    pub fn get_default_panel(&self) -> Option<&str> {
        self.options
            .as_ref()
            .and_then(|opts| opts.default_panel.as_deref())
            .or_else(|| self.panels.first().map(|p| p.key.as_str()))
    }

    /// Check if multiple panels can be open at once
    #[must_use]
    pub fn allows_multiple_open(&self) -> bool {
        self.options
            .as_ref()
            .is_some_and(|opts| opts.allow_multiple_open)
    }

    /// Get the total number of panels
    #[must_use]
    pub fn panel_count(&self) -> usize {
        self.panels.len()
    }

    /// Get all enabled panels
    #[must_use]
    pub fn get_enabled_panels(&self) -> Vec<&Panel> {
        self.panels.iter().filter(|p| p.enabled).collect()
    }

    /// Get all parameters from all panels (flattened)
    #[must_use]
    pub fn get_all_parameters(&self) -> Vec<&dyn Parameter> {
        self.panels
            .iter()
            .flat_map(|panel| panel.children.iter().map(std::convert::AsRef::as_ref))
            .collect()
    }

    /// Get parameters from a specific panel
    #[must_use]
    pub fn get_panel_parameters(&self, panel_key: &str) -> Option<Vec<&dyn Parameter>> {
        self.get_panel(panel_key).map(|panel| {
            panel
                .children
                .iter()
                .map(std::convert::AsRef::as_ref)
                .collect()
        })
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_panel_parameter_builder() {
        let param = PanelParameter::builder()
            .key("settings")
            .name("Settings")
            .description("Application settings")
            .build()
            .unwrap();

        assert_eq!(param.metadata.key.as_str(), "settings");
        assert_eq!(param.metadata.name, "Settings");
    }

    #[test]
    fn test_panel_parameter_with_options() {
        let param = PanelParameter::builder()
            .key("tabs")
            .name("Tabs")
            .options(
                PanelParameterOptions::builder()
                    .default_panel("general")
                    .allow_multiple_open(true)
                    .build(),
            )
            .build()
            .unwrap();

        assert!(param.allows_multiple_open());
    }

    #[test]
    fn test_panel_parameter_with_panels() {
        let param = PanelParameter::builder()
            .key("config")
            .name("Configuration")
            .panel(Panel::new("general", "General").with_description("General settings"))
            .panel(Panel::new("advanced", "Advanced").with_enabled(false))
            .build()
            .unwrap();

        assert_eq!(param.panel_count(), 2);
        assert_eq!(param.get_enabled_panels().len(), 1);
    }

    #[test]
    fn test_panel_parameter_missing_key() {
        let result = PanelParameter::builder().name("Test").build();

        assert!(matches!(
            result,
            Err(ParameterError::BuilderMissingField { field }) if field == "key"
        ));
    }

    #[test]
    fn test_panel() {
        let panel = Panel::new("test", "Test Panel")
            .with_description("A test panel")
            .with_icon("settings")
            .with_enabled(true);

        assert_eq!(panel.key, "test");
        assert_eq!(panel.label, "Test Panel");
        assert!(panel.enabled);
        assert!(panel.is_empty());
    }
}

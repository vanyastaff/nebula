use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::core::{Displayable, ParameterDisplay, ParameterKind, ParameterMetadata, ParameterType};

/// Panel parameter - container for organizing parameters into sections/tabs
#[derive(Serialize)]
pub struct PanelParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    /// Panel sections with their parameters
    pub panels: Vec<Panel>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<PanelParameterOptions>,

    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Parameters contained in this panel
    #[serde(skip)]
    pub children: Vec<Box<dyn ParameterType>>,

    /// Optional icon identifier
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,

    /// Whether this panel is enabled
    #[serde(default)]
    pub enabled: bool,
}

/// Configuration options for panel parameter
#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct PanelParameterOptions {
    /// Key of the default active panel
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_panel: Option<String>,

    /// Whether multiple panels can be open at once (accordion mode)
    #[serde(default)]
    pub allow_multiple_open: bool,
}

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
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Set the icon
    pub fn with_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Add a child parameter
    pub fn with_child(mut self, child: Box<dyn ParameterType>) -> Self {
        self.children.push(child);
        self
    }

    /// Add multiple child parameters
    pub fn with_children(mut self, children: Vec<Box<dyn ParameterType>>) -> Self {
        self.children.extend(children);
        self
    }

    /// Set enabled state
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Get the number of children in this panel
    pub fn child_count(&self) -> usize {
        self.children.len()
    }

    /// Check if this panel is empty
    pub fn is_empty(&self) -> bool {
        self.children.is_empty()
    }
}

impl ParameterType for PanelParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::Panel
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
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
    /// Create a new panel parameter
    pub fn new(metadata: ParameterMetadata) -> Self {
        Self {
            metadata,
            panels: Vec::new(),
            options: None,
            display: None,
        }
    }

    /// Add a panel
    pub fn add_panel(&mut self, panel: Panel) {
        self.panels.push(panel);
    }

    /// Get a panel by key
    pub fn get_panel(&self, key: &str) -> Option<&Panel> {
        self.panels.iter().find(|p| p.key == key)
    }

    /// Get a mutable panel by key
    pub fn get_panel_mut(&mut self, key: &str) -> Option<&mut Panel> {
        self.panels.iter_mut().find(|p| p.key == key)
    }

    /// Get all panel keys
    pub fn get_panel_keys(&self) -> Vec<&str> {
        self.panels.iter().map(|p| p.key.as_str()).collect()
    }

    /// Get the default active panel key
    pub fn get_default_panel(&self) -> Option<&str> {
        self.options
            .as_ref()
            .and_then(|opts| opts.default_panel.as_deref())
            .or_else(|| self.panels.first().map(|p| p.key.as_str()))
    }

    /// Check if multiple panels can be open at once
    pub fn allows_multiple_open(&self) -> bool {
        self.options
            .as_ref()
            .map(|opts| opts.allow_multiple_open)
            .unwrap_or(false)
    }

    /// Get the total number of panels
    pub fn panel_count(&self) -> usize {
        self.panels.len()
    }

    /// Get all enabled panels
    pub fn get_enabled_panels(&self) -> Vec<&Panel> {
        self.panels.iter().filter(|p| p.enabled).collect()
    }

    /// Get all parameters from all panels (flattened)
    pub fn get_all_parameters(&self) -> Vec<&dyn ParameterType> {
        self.panels
            .iter()
            .flat_map(|panel| panel.children.iter().map(|child| child.as_ref()))
            .collect()
    }

    /// Get parameters from a specific panel
    pub fn get_panel_parameters(&self, panel_key: &str) -> Option<Vec<&dyn ParameterType>> {
        self.get_panel(panel_key)
            .map(|panel| panel.children.iter().map(|child| child.as_ref()).collect())
    }
}

use serde::{Deserialize, Serialize};

use crate::core::{
    Displayable, Parameter, ParameterDisplay, ParameterError, ParameterKind, ParameterMetadata,
    ParameterValidation, Validatable,
};
use nebula_value::Value;

/// Value for list parameters containing array of child parameter values
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ListValue {
    /// Array of values from child parameters
    pub items: Vec<nebula_value::Value>,
}

impl ListValue {
    /// Create a new `ListValue`
    #[must_use]
    pub fn new(items: Vec<nebula_value::Value>) -> Self {
        Self { items }
    }

    /// Create an empty `ListValue`
    #[must_use]
    pub fn empty() -> Self {
        Self { items: Vec::new() }
    }

    /// Add an item to the list
    pub fn push(&mut self, item: nebula_value::Value) {
        self.items.push(item);
    }

    /// Get item count
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Check if the list is empty
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// Parameter for lists - acts as a container with child parameters
#[derive(Serialize, bon::Builder)]
pub struct ListParameter {
    #[serde(flatten)]
    /// Parameter metadata including key, name, description
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Default value if parameter is not set
    pub default: Option<nebula_value::Array>,

    /// Child parameters in this list
    #[serde(skip)]
    pub children: Vec<Box<dyn Parameter>>,

    /// Template parameter for creating new items (optional)
    #[serde(skip)]
    pub item_template: Option<Box<dyn Parameter>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Configuration options for this parameter type
    pub options: Option<ListParameterOptions>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Display rules controlling when this parameter is shown
    pub display: Option<ParameterDisplay>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Validation rules for this parameter
    pub validation: Option<ParameterValidation>,
}

/// Configuration options for list parameters
#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]
pub struct ListParameterOptions {
    /// Minimum number of items
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_items: Option<usize>,

    /// Maximum number of items
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_items: Option<usize>,

    /// Whether items can be reordered
    #[serde(default = "default_allow_reorder")]
    pub allow_reorder: bool,

    /// Whether items can be duplicated
    #[builder(default)]
    #[serde(default)]
    pub allow_duplicates: bool,
}

fn default_allow_reorder() -> bool {
    true
}

impl Default for ListParameterOptions {
    fn default() -> Self {
        Self {
            min_items: None,
            max_items: None,
            allow_reorder: true,
            allow_duplicates: true,
        }
    }
}

// Manual Debug implementation since we skip trait objects
impl std::fmt::Debug for ListParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListParameter")
            .field("metadata", &self.metadata)
            .field("default", &self.default)
            .field(
                "children",
                &format!("Vec<Box<dyn ParameterType>> (len: {})", self.children.len()),
            )
            .field("item_template", &"Option<Box<dyn ParameterType>>")
            .field("options", &self.options)
            .field("display", &self.display)
            .field("validation", &self.validation)
            .finish()
    }
}

impl Parameter for ListParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::List
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
    }
}

impl std::fmt::Display for ListParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ListParameter({})", self.metadata.name)
    }
}

impl Validatable for ListParameter {
    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }

    fn validate_sync(&self, value: &Value) -> Result<(), ParameterError> {
        // Type check
        let arr = match value {
            Value::Array(a) => a,
            _ => {
                return Err(ParameterError::InvalidValue {
                    key: self.metadata.key.clone(),
                    reason: format!("Expected array value for list, got {}", value.kind()),
                });
            }
        };

        // Required check
        if self.is_empty(value) && self.is_required() {
            return Err(ParameterError::MissingValue {
                key: self.metadata.key.clone(),
            });
        }

        // Item count validation
        if let Some(options) = &self.options {
            let item_count = arr.len();

            if let Some(min_items) = options.min_items
                && item_count < min_items
            {
                return Err(ParameterError::InvalidValue {
                    key: self.metadata.key.clone(),
                    reason: format!("List must have at least {min_items} items, got {item_count}"),
                });
            }

            if let Some(max_items) = options.max_items
                && item_count > max_items
            {
                return Err(ParameterError::InvalidValue {
                    key: self.metadata.key.clone(),
                    reason: format!("List must have at most {max_items} items, got {item_count}"),
                });
            }
        }

        Ok(())
    }

    fn is_empty(&self, value: &Value) -> bool {
        match value {
            Value::Array(a) => a.is_empty(),
            _ => true,
        }
    }
}

impl Displayable for ListParameter {
    fn display(&self) -> Option<&ParameterDisplay> {
        self.display.as_ref()
    }

    fn set_display(&mut self, display: Option<ParameterDisplay>) {
        self.display = display;
    }
}

impl ListParameter {
    /// Create a new list parameter as a container
    pub fn new(
        key: &str,
        name: &str,
        description: &str,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            metadata: ParameterMetadata {
                key: nebula_core::ParameterKey::new(key)?,
                name: name.to_string(),
                description: description.to_string(),
                required: false,
                placeholder: Some("Add list items...".to_string()),
                hint: Some("List container with child parameters".to_string()),
            },
            default: None,
            children: Vec::new(),
            item_template: None,
            options: Some(ListParameterOptions::default()),
            display: None,
            validation: None,
        })
    }

    /// Set the template parameter for creating new items
    pub fn set_template(&mut self, template: Box<dyn Parameter>) {
        self.item_template = Some(template);
    }

    /// Get the template parameter
    #[must_use]
    pub fn template(&self) -> Option<&Box<dyn Parameter>> {
        self.item_template.as_ref()
    }

    /// Add a child parameter to the list
    pub fn add_child(&mut self, child: Box<dyn Parameter>) {
        self.children.push(child);
    }

    /// Remove a child parameter by index
    pub fn remove_child(&mut self, index: usize) -> Option<Box<dyn Parameter>> {
        if index < self.children.len() {
            Some(self.children.remove(index))
        } else {
            None
        }
    }

    /// Get child parameter by index
    #[must_use]
    pub fn get_child(&self, index: usize) -> Option<&Box<dyn Parameter>> {
        self.children.get(index)
    }

    /// Get mutable child parameter by index
    pub fn get_child_mut(&mut self, index: usize) -> Option<&mut Box<dyn Parameter>> {
        self.children.get_mut(index)
    }

    /// Get all children
    #[must_use]
    pub fn children(&self) -> &[Box<dyn Parameter>] {
        &self.children
    }

    /// Get mutable reference to all children
    pub fn children_mut(&mut self) -> &mut Vec<Box<dyn Parameter>> {
        &mut self.children
    }

    /// Get children count
    #[must_use]
    pub fn children_count(&self) -> usize {
        self.children.len()
    }

    /// Add an item to a list value
    #[must_use = "operation result must be checked"]
    pub fn add_item(value: &nebula_value::Array, item: nebula_value::Value) -> nebula_value::Array {
        value.push(item)
    }

    /// Remove an item from a list value by index
    #[must_use = "operation result must be checked"]
    pub fn remove_item(
        value: &nebula_value::Array,
        index: usize,
    ) -> Result<(nebula_value::Array, nebula_value::Value), String> {
        if index < value.len() {
            value.remove(index).map_err(|e| e.to_string())
        } else {
            Err("Index out of bounds".to_string())
        }
    }

    /// Move an item to a different position in a list value
    #[must_use = "operation result must be checked"]
    pub fn move_item(
        value: &nebula_value::Array,
        old_index: usize,
        new_index: usize,
    ) -> Result<nebula_value::Array, String> {
        if old_index < value.len() && new_index < value.len() && old_index != new_index {
            // Remove from old position
            let (value_after_remove, item) = value.remove(old_index).map_err(|e| e.to_string())?;

            // Insert at new position
            value_after_remove
                .insert(new_index, item)
                .map_err(|e| e.to_string())
        } else {
            Err("Invalid indices for item move".to_string())
        }
    }

    /// Create a new item from the template parameter
    #[must_use = "operation result must be used"]
    pub fn create_item_from_template(&self) -> Result<nebula_value::Value, ParameterError> {
        if let Some(template) = &self.item_template {
            // Create a default value based on the template parameter type
            let default_val = match template.kind() {
                ParameterKind::Text => nebula_value::Value::text(""),
                ParameterKind::Number => nebula_value::Value::integer(0),
                ParameterKind::Checkbox => nebula_value::Value::boolean(false),
                _ => nebula_value::Value::text("template_value"),
            };
            Ok(default_val)
        } else {
            Ok(nebula_value::Value::Null)
        }
    }
}

// Note: Conversion function removed - use nebula_value::ValueRefExt trait instead
// The trait provides .to_json() method for ergonomic conversions

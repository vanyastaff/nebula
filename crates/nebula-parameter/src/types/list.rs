use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::core::{
    Displayable,  HasValue, Parameter, ParameterDisplay, ParameterError, ParameterKind,
    ParameterMetadata, ParameterValidation, Validatable,
};
use crate::core::traits::Expressible;
use nebula_expression::MaybeExpression;
use nebula_value::Value;

/// Value for list parameters containing array of child parameter values
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ListValue {
    /// Array of values from child parameters
    pub items: Vec<nebula_value::Value>,
}

impl ListValue {
    /// Create a new ListValue
    pub fn new(items: Vec<nebula_value::Value>) -> Self {
        Self { items }
    }

    /// Create an empty ListValue
    pub fn empty() -> Self {
        Self { items: Vec::new() }
    }

    /// Add an item to the list
    pub fn push(&mut self, item: nebula_value::Value) {
        self.items.push(item);
    }

    /// Get item count
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Check if the list is empty
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// Parameter for lists - acts as a container with child parameters
#[derive(Serialize)]
pub struct ListParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<nebula_value::Array>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<nebula_value::Array>,

    /// Child parameters in this list
    #[serde(skip)]
    pub children: Vec<Box<dyn Parameter>>,

    /// Template parameter for creating new items (optional)
    #[serde(skip)]
    pub item_template: Option<Box<dyn Parameter>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<ListParameterOptions>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<ParameterValidation>,
}

/// Configuration options for list parameters
#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
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
            .field("value", &self.value)
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

impl HasValue for ListParameter {
    type Value = nebula_value::Array;

    fn get(&self) -> Option<&Self::Value> {
        self.value.as_ref()
    }

    fn get_mut(&mut self) -> Option<&mut Self::Value> {
        self.value.as_mut()
    }

    fn set(&mut self, value: Self::Value) -> Result<(), ParameterError> {
        self.value = Some(value);
        Ok(())
    }

    fn default(&self) -> Option<&Self::Value> {
        self.default.as_ref()
    }

    fn clear(&mut self) {
        self.value = None;
    }
}

#[async_trait::async_trait]
impl Expressible for ListParameter {
    fn to_expression(&self) -> Option<MaybeExpression<Value>> {
        self.value
            .as_ref()
            .map(|arr| MaybeExpression::Value(nebula_value::Value::Array(arr.clone())))
    }

    fn from_expression(
        &mut self,
        value: impl Into<MaybeExpression<Value>> + Send,
    ) -> Result<(), ParameterError> {
        let value = value.into();
        match value {
            MaybeExpression::Value(nebula_value::Value::Array(arr)) => {
                self.value = Some(arr);
                Ok(())
            }
            _ => Err(ParameterError::InvalidValue {
                key: self.metadata.key.clone(),
                reason: "Expected array value for list parameter".to_string(),
            }),
        }
    }
}

impl Validatable for ListParameter {
    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }
    fn is_empty(&self, value: &Self::Value) -> bool {
        value.is_empty()
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
            value: None,
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
    pub fn get_child(&self, index: usize) -> Option<&Box<dyn Parameter>> {
        self.children.get(index)
    }

    /// Get mutable child parameter by index
    pub fn get_child_mut(&mut self, index: usize) -> Option<&mut Box<dyn Parameter>> {
        self.children.get_mut(index)
    }

    /// Get all children
    pub fn children(&self) -> &[Box<dyn Parameter>] {
        &self.children
    }

    /// Get mutable reference to all children
    pub fn children_mut(&mut self) -> &mut Vec<Box<dyn Parameter>> {
        &mut self.children
    }

    /// Get children count
    pub fn children_count(&self) -> usize {
        self.children.len()
    }

    /// Add an item to the list value
    pub fn add_item(&mut self, item: nebula_value::Value) -> Result<(), ParameterError> {
        use crate::ValueRefExt;
        if let Some(items) = &self.value {
            self.value = Some(items.push(item.to_json()));
        } else {
            self.value = Some(nebula_value::Array::from_vec(vec![item.to_json()]));
        }
        Ok(())
    }

    /// Remove an item from the list value by index
    pub fn remove_item(&mut self, index: usize) -> Result<bool, ParameterError> {
        if let Some(items) = &self.value {
            if index < items.len() {
                match items.remove(index) {
                    Ok((new_array, _)) => {
                        self.value = Some(new_array);
                        Ok(true)
                    }
                    Err(_) => Ok(false),
                }
            } else {
                Ok(false)
            }
        } else {
            Ok(false)
        }
    }

    /// Move an item to a different position
    pub fn move_item(&mut self, old_index: usize, new_index: usize) -> Result<(), ParameterError> {
        if let Some(items) = &self.value {
            if old_index < items.len() && new_index < items.len() && old_index != new_index {
                // Remove from old position
                let (items_after_remove, item) =
                    items
                        .remove(old_index)
                        .map_err(|e| ParameterError::InvalidValue {
                            key: self.metadata.key.clone(),
                            reason: format!("Failed to remove item: {}", e),
                        })?;

                // Insert at new position
                let items_after_insert =
                    items_after_remove.insert(new_index, item).map_err(|e| {
                        ParameterError::InvalidValue {
                            key: self.metadata.key.clone(),
                            reason: format!("Failed to insert item: {}", e),
                        }
                    })?;

                self.value = Some(items_after_insert);
                Ok(())
            } else {
                Err(ParameterError::InvalidValue {
                    key: self.metadata.key.clone(),
                    reason: "Invalid indices for item move".to_string(),
                })
            }
        } else {
            Err(ParameterError::InvalidValue {
                key: self.metadata.key.clone(),
                reason: "No items in list".to_string(),
            })
        }
    }

    /// Create a new item from the template parameter
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

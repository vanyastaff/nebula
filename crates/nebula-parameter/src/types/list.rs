//! List parameter type for array/list containers

use serde::{Deserialize, Serialize};

use crate::core::{
    Describable, Displayable, Parameter, ParameterDisplay, ParameterError, ParameterKind,
    ParameterMetadata, ParameterValidation, Validatable,
};
use nebula_value::{Value, ValueKind};

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
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_parameter::prelude::*;
///
/// let param = ListParameter::builder()
///     .key("items")
///     .name("Items")
///     .description("List of items")
///     .options(
///         ListParameterOptions::builder()
///             .min_items(1)
///             .max_items(10)
///             .build()
///     )
///     .build()?;
/// ```
#[derive(Serialize)]
pub struct ListParameter {
    /// Parameter metadata (key, name, description, etc.)
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    /// Default value if parameter is not set
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<nebula_value::Array>,

    /// Child parameters in this list
    #[serde(skip)]
    pub children: Vec<Box<dyn Parameter>>,

    /// Template parameter for creating new items (optional)
    #[serde(skip)]
    pub item_template: Option<Box<dyn Parameter>>,

    /// Configuration options for this parameter type
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<ListParameterOptions>,

    /// Display conditions controlling when this parameter is shown
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    /// Validation rules for this parameter
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<ParameterValidation>,
}

/// Configuration options for list parameters
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListParameterOptions {
    /// Minimum number of items
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_items: Option<usize>,

    /// Maximum number of items
    #[serde(default, skip_serializing_if = "Option::is_none")]
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

// =============================================================================
// ListParameter Builder
// =============================================================================

/// Builder for `ListParameter`
#[derive(Default)]
pub struct ListParameterBuilder {
    // Metadata fields
    key: Option<String>,
    name: Option<String>,
    description: String,
    required: bool,
    placeholder: Option<String>,
    hint: Option<String>,
    // Parameter fields
    default: Option<nebula_value::Array>,
    children: Vec<Box<dyn Parameter>>,
    item_template: Option<Box<dyn Parameter>>,
    options: Option<ListParameterOptions>,
    display: Option<ParameterDisplay>,
    validation: Option<ParameterValidation>,
}

impl ListParameter {
    /// Create a new builder
    #[must_use]
    pub fn builder() -> ListParameterBuilder {
        ListParameterBuilder::new()
    }
}

impl ListParameterBuilder {
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
            default: None,
            children: Vec::new(),
            item_template: None,
            options: None,
            display: None,
            validation: None,
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

    /// Set the default value
    #[must_use]
    pub fn default(mut self, default: nebula_value::Array) -> Self {
        self.default = Some(default);
        self
    }

    /// Set child parameters
    #[must_use]
    pub fn children(mut self, children: Vec<Box<dyn Parameter>>) -> Self {
        self.children = children;
        self
    }

    /// Add a child parameter
    #[must_use]
    pub fn child(mut self, child: Box<dyn Parameter>) -> Self {
        self.children.push(child);
        self
    }

    /// Set item template
    #[must_use]
    pub fn item_template(mut self, template: Box<dyn Parameter>) -> Self {
        self.item_template = Some(template);
        self
    }

    /// Set the options
    #[must_use]
    pub fn options(mut self, options: ListParameterOptions) -> Self {
        self.options = Some(options);
        self
    }

    /// Set display conditions
    #[must_use]
    pub fn display(mut self, display: ParameterDisplay) -> Self {
        self.display = Some(display);
        self
    }

    /// Set validation rules
    #[must_use]
    pub fn validation(mut self, validation: ParameterValidation) -> Self {
        self.validation = Some(validation);
        self
    }

    // -------------------------------------------------------------------------
    // Build
    // -------------------------------------------------------------------------

    /// Build the `ListParameter`
    ///
    /// # Errors
    ///
    /// Returns error if required fields are missing or key format is invalid.
    pub fn build(self) -> Result<ListParameter, ParameterError> {
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

        Ok(ListParameter {
            metadata,
            default: self.default,
            children: self.children,
            item_template: self.item_template,
            options: self.options,
            display: self.display,
            validation: self.validation,
        })
    }
}

// =============================================================================
// ListParameterOptions Builder
// =============================================================================

/// Builder for `ListParameterOptions`
#[derive(Debug, Default)]
pub struct ListParameterOptionsBuilder {
    min_items: Option<usize>,
    max_items: Option<usize>,
    allow_reorder: bool,
    allow_duplicates: bool,
}

impl ListParameterOptions {
    /// Create a new builder
    #[must_use]
    pub fn builder() -> ListParameterOptionsBuilder {
        ListParameterOptionsBuilder {
            allow_reorder: true,
            ..Default::default()
        }
    }
}

impl ListParameterOptionsBuilder {
    /// Set minimum number of items
    #[must_use]
    pub fn min_items(mut self, min_items: usize) -> Self {
        self.min_items = Some(min_items);
        self
    }

    /// Set maximum number of items
    #[must_use]
    pub fn max_items(mut self, max_items: usize) -> Self {
        self.max_items = Some(max_items);
        self
    }

    /// Set whether items can be reordered
    #[must_use]
    pub fn allow_reorder(mut self, allow_reorder: bool) -> Self {
        self.allow_reorder = allow_reorder;
        self
    }

    /// Set whether duplicates are allowed
    #[must_use]
    pub fn allow_duplicates(mut self, allow_duplicates: bool) -> Self {
        self.allow_duplicates = allow_duplicates;
        self
    }

    /// Build the options
    #[must_use]
    pub fn build(self) -> ListParameterOptions {
        ListParameterOptions {
            min_items: self.min_items,
            max_items: self.max_items,
            allow_reorder: self.allow_reorder,
            allow_duplicates: self.allow_duplicates,
        }
    }
}

// =============================================================================
// Trait Implementations
// =============================================================================

// Manual Debug implementation since we skip trait objects
impl std::fmt::Debug for ListParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListParameter")
            .field("metadata", &self.metadata)
            .field("default", &self.default)
            .field(
                "children",
                &format!("Vec<Box<dyn Parameter>> (len: {})", self.children.len()),
            )
            .field("item_template", &"Option<Box<dyn Parameter>>")
            .field("options", &self.options)
            .field("display", &self.display)
            .field("validation", &self.validation)
            .finish()
    }
}

impl Describable for ListParameter {
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
    fn expected_kind(&self) -> Option<ValueKind> {
        Some(ValueKind::Array)
    }

    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }

    fn validate_sync(&self, value: &Value) -> Result<(), ParameterError> {
        // Type check
        if let Some(expected) = self.expected_kind() {
            let actual = value.kind();
            if actual != ValueKind::Null && actual != expected {
                return Err(ParameterError::InvalidType {
                    key: self.metadata.key.clone(),
                    expected_type: expected.name().to_string(),
                    actual_details: actual.name().to_string(),
                });
            }
        }

        // Required check - must come before early return for Null
        if self.is_required() && self.is_empty(value) {
            return Err(ParameterError::MissingValue {
                key: self.metadata.key.clone(),
            });
        }

        let arr = match value {
            Value::Array(a) => a,
            Value::Null => return Ok(()), // Null is allowed for optional
            _ => return Ok(()),           // Type error already handled above
        };

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
        value.is_null() || value.as_array().is_some_and(|arr| arr.is_empty())
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
    /// Set the template parameter for creating new items
    pub fn set_template(&mut self, template: Box<dyn Parameter>) {
        self.item_template = Some(template);
    }

    /// Get the template parameter
    #[must_use]
    pub fn template(&self) -> Option<&dyn Parameter> {
        self.item_template.as_deref()
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
    pub fn get_child(&self, index: usize) -> Option<&dyn Parameter> {
        self.children.get(index).map(|b| b.as_ref())
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

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_parameter_builder() {
        let param = ListParameter::builder()
            .key("items")
            .name("Items")
            .description("List of items")
            .required(true)
            .build()
            .unwrap();

        assert_eq!(param.metadata.key.as_str(), "items");
        assert_eq!(param.metadata.name, "Items");
        assert!(param.metadata.required);
    }

    #[test]
    fn test_list_parameter_with_options() {
        let param = ListParameter::builder()
            .key("tags")
            .name("Tags")
            .options(
                ListParameterOptions::builder()
                    .min_items(1)
                    .max_items(5)
                    .allow_reorder(true)
                    .build(),
            )
            .build()
            .unwrap();

        let opts = param.options.unwrap();
        assert_eq!(opts.min_items, Some(1));
        assert_eq!(opts.max_items, Some(5));
        assert!(opts.allow_reorder);
    }

    #[test]
    fn test_list_parameter_missing_key() {
        let result = ListParameter::builder().name("Test").build();

        assert!(matches!(
            result,
            Err(ParameterError::BuilderMissingField { field }) if field == "key"
        ));
    }

    #[test]
    fn test_list_value() {
        let mut list = ListValue::empty();
        assert!(list.is_empty());

        list.push(nebula_value::Value::text("item1"));
        list.push(nebula_value::Value::text("item2"));

        assert_eq!(list.len(), 2);
        assert!(!list.is_empty());
    }
}

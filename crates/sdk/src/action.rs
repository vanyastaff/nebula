//! Action development utilities.
//!
//! This module provides helpers and wrappers for creating actions.
//!
//! # Examples
//!
//! ```rust,no_run
//! use nebula_core::action_key;
//! use nebula_sdk::action::ActionBuilder;
//!
//! let action = ActionBuilder::new(action_key!("my.action"), "My Action")
//!     .with_description("Does something useful")
//!     .with_version(1, 0)
//!     .build();
//! ```

use nebula_action::ActionMetadata;
use nebula_core::ActionKey;

/// Builder for creating action metadata.
///
/// # Examples
///
/// ```
/// use nebula_core::action_key;
/// use nebula_sdk::action::ActionBuilder;
///
/// let metadata = ActionBuilder::new(action_key!("http.request"), "HTTP Request")
///     .with_description("Makes HTTP requests")
///     .with_version(2, 0)
///     .build();
/// ```
pub struct ActionBuilder {
    key: ActionKey,
    name: String,
    description: String,
    version: (u64, u64),
}

impl ActionBuilder {
    /// Create a new action builder.
    ///
    /// # Arguments
    ///
    /// * `key` - Unique action identifier (e.g., `action_key!("http.request")`)
    /// * `name` - Human-readable name
    pub fn new(key: ActionKey, name: impl Into<String>) -> Self {
        Self {
            key,
            name: name.into(),
            description: String::new(),
            version: (1, 0),
        }
    }

    /// Set the action description.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Set the interface version (major, minor components; patch defaults to 0).
    pub fn with_version(mut self, major: u64, minor: u64) -> Self {
        self.version = (major, minor);
        self
    }

    /// Build the action metadata.
    pub fn build(self) -> ActionMetadata {
        ActionMetadata::new(self.key, self.name, self.description)
            .with_version(self.version.0, self.version.1)
    }
}

/// Helper functions for action development.
pub mod helpers {
    use serde_json::Value;

    fn check_required_fields(input: &Value, required: &[Value]) -> Result<(), String> {
        for req in required {
            if let Some(field) = req.as_str()
                && input.get(field).is_none()
            {
                return Err(format!("Missing required field: {}", field));
            }
        }
        Ok(())
    }

    /// Validate input against a JSON schema.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use nebula_sdk::action::helpers::validate_schema;
    ///
    /// let schema = serde_json::json!({
    ///     "type": "object",
    ///     "properties": {
    ///         "name": { "type": "string" }
    ///     },
    ///     "required": ["name"]
    /// });
    ///
    /// let input = serde_json::json!({ "name": "test" });
    /// assert!(validate_schema(&input, &schema).is_ok());
    /// ```
    pub fn validate_schema(input: &Value, schema: &Value) -> Result<(), String> {
        // Basic validation - in production, use jsonschema crate
        if schema.get("type").and_then(|t| t.as_str()) != Some("object") {
            return Ok(());
        }
        if !input.is_object() {
            return Err("Expected object".to_string());
        }
        if let Some(required) = schema.get("required").and_then(|r| r.as_array()) {
            check_required_fields(input, required)?;
        }
        Ok(())
    }

    /// Parse and validate input.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use nebula_sdk::action::helpers::parse_input;
    ///
    /// #[derive(Deserialize)]
    /// struct MyInput {
    ///     name: String,
    /// }
    ///
    /// let value = serde_json::json!({ "name": "test" });
    /// let input: MyInput = parse_input(&value)?;
    /// ```
    pub fn parse_input<T: serde::de::DeserializeOwned>(input: &Value) -> Result<T, crate::Error> {
        serde_json::from_value(input.clone()).map_err(crate::Error::Serialization)
    }
}

#[cfg(test)]
mod tests {
    use nebula_core::action_key;

    use super::ActionBuilder;

    #[test]
    fn test_action_builder() {
        let metadata = ActionBuilder::new(action_key!("test.action"), "Test Action")
            .with_description("A test action")
            .with_version(2, 1)
            .build();

        assert_eq!(metadata.key, action_key!("test.action"));
        assert_eq!(metadata.name, "Test Action");
        assert_eq!(metadata.description, "A test action");
    }
}

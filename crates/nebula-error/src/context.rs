//! Error context for providing additional information

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

/// Additional context information for errors
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorContext {
    /// Human-readable context description
    pub description: String,
    /// Additional key-value pairs for context
    pub metadata: HashMap<String, String>,
    /// Stack trace or call chain information
    pub stack_trace: Option<String>,
    /// Timestamp when the error occurred
    pub timestamp: Option<chrono::DateTime<chrono::Utc>>,
    /// User ID associated with the error
    pub user_id: Option<String>,
    /// Tenant ID associated with the error
    pub tenant_id: Option<String>,
    /// Request ID or correlation ID
    pub request_id: Option<String>,
    /// Component or module where the error occurred
    pub component: Option<String>,
    /// Operation being performed when the error occurred
    pub operation: Option<String>,
}

impl ErrorContext {
    /// Create a new error context
    pub fn new(description: impl Into<String>) -> Self {
        Self {
            description: description.into(),
            metadata: HashMap::new(),
            stack_trace: None,
            timestamp: Some(chrono::Utc::now()),
            user_id: None,
            tenant_id: None,
            request_id: None,
            component: None,
            operation: None,
        }
    }

    /// Add metadata key-value pair
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Add stack trace information
    pub fn with_stack_trace(mut self, stack_trace: impl Into<String>) -> Self {
        self.stack_trace = Some(stack_trace.into());
        self
    }

    /// Set user ID
    pub fn with_user_id(mut self, user_id: impl Into<String>) -> Self {
        self.user_id = Some(user_id.into());
        self
    }

    /// Set tenant ID
    pub fn with_tenant_id(mut self, tenant_id: impl Into<String>) -> Self {
        self.tenant_id = Some(tenant_id.into());
        self
    }

    /// Set request ID
    pub fn with_request_id(mut self, request_id: impl Into<String>) -> Self {
        self.request_id = Some(request_id.into());
        self
    }

    /// Set component name
    pub fn with_component(mut self, component: impl Into<String>) -> Self {
        self.component = Some(component.into());
        self
    }

    /// Set operation name
    pub fn with_operation(mut self, operation: impl Into<String>) -> Self {
        self.operation = Some(operation.into());
        self
    }

    /// Get metadata value by key
    pub fn get_metadata(&self, key: &str) -> Option<&String> {
        self.metadata.get(key)
    }

    /// Check if context has specific metadata
    pub fn has_metadata(&self, key: &str) -> bool {
        self.metadata.contains_key(key)
    }

    /// Get all metadata keys
    pub fn metadata_keys(&self) -> impl Iterator<Item = &String> {
        self.metadata.keys()
    }

    /// Get all metadata entries
    pub fn metadata_entries(&self) -> impl Iterator<Item = (&String, &String)> {
        self.metadata.iter()
    }
}

impl fmt::Display for ErrorContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.description)?;

        if !self.metadata.is_empty() {
            write!(f, " [")?;
            let mut first = true;
            for (key, value) in &self.metadata {
                if !first {
                    write!(f, ", ")?;
                }
                write!(f, "{}: {}", key, value)?;
                first = false;
            }
            write!(f, "]")?;
        }

        if let Some(ref component) = self.component {
            write!(f, " (Component: {})", component)?;
        }

        if let Some(ref operation) = self.operation {
            write!(f, " (Operation: {})", operation)?;
        }

        if let Some(ref user_id) = self.user_id {
            write!(f, " (User: {})", user_id)?;
        }

        if let Some(ref tenant_id) = self.tenant_id {
            write!(f, " (Tenant: {})", tenant_id)?;
        }

        if let Some(ref request_id) = self.request_id {
            write!(f, " (Request: {})", request_id)?;
        }

        Ok(())
    }
}

impl Default for ErrorContext {
    fn default() -> Self {
        Self {
            description: "Unknown error context".to_string(),
            metadata: HashMap::new(),
            stack_trace: None,
            timestamp: Some(chrono::Utc::now()),
            user_id: None,
            tenant_id: None,
            request_id: None,
            component: None,
            operation: None,
        }
    }
}

/// Builder pattern for ErrorContext
pub struct ErrorContextBuilder {
    context: ErrorContext,
}

impl ErrorContextBuilder {
    /// Create a new builder
    pub fn new(description: impl Into<String>) -> Self {
        Self { context: ErrorContext::new(description) }
    }

    /// Add metadata
    pub fn metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.context = self.context.with_metadata(key, value);
        self
    }

    /// Add stack trace
    pub fn stack_trace(mut self, stack_trace: impl Into<String>) -> Self {
        self.context = self.context.with_stack_trace(stack_trace);
        self
    }

    /// Set user ID
    pub fn user_id(mut self, user_id: impl Into<String>) -> Self {
        self.context = self.context.with_user_id(user_id);
        self
    }

    /// Set tenant ID
    pub fn tenant_id(mut self, tenant_id: impl Into<String>) -> Self {
        self.context = self.context.with_tenant_id(tenant_id);
        self
    }

    /// Set request ID
    pub fn request_id(mut self, request_id: impl Into<String>) -> Self {
        self.context = self.context.with_request_id(request_id);
        self
    }

    /// Set component
    pub fn component(mut self, component: impl Into<String>) -> Self {
        self.context = self.context.with_component(component);
        self
    }

    /// Set operation
    pub fn operation(mut self, operation: impl Into<String>) -> Self {
        self.context = self.context.with_operation(operation);
        self
    }

    /// Build the final ErrorContext
    pub fn build(self) -> ErrorContext {
        self.context
    }
}

impl From<ErrorContext> for ErrorContextBuilder {
    fn from(context: ErrorContext) -> Self {
        Self { context }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_context_creation() {
        let context = ErrorContext::new("Database connection failed");
        assert_eq!(context.description, "Database connection failed");
        assert!(context.timestamp.is_some());
    }

    #[test]
    fn test_error_context_builder() {
        let context = ErrorContextBuilder::new("API call failed")
            .component("user-service")
            .operation("create_user")
            .user_id("user123")
            .tenant_id("tenant456")
            .metadata("endpoint", "/api/users")
            .metadata("method", "POST")
            .build();

        assert_eq!(context.description, "API call failed");
        assert_eq!(context.component, Some("user-service".to_string()));
        assert_eq!(context.operation, Some("create_user".to_string()));
        assert_eq!(context.user_id, Some("user123".to_string()));
        assert_eq!(context.tenant_id, Some("tenant456".to_string()));
        assert_eq!(context.get_metadata("endpoint"), Some(&"/api/users".to_string()));
        assert_eq!(context.get_metadata("method"), Some(&"POST".to_string()));
    }

    #[test]
    fn test_error_context_display() {
        let context = ErrorContextBuilder::new("Validation failed")
            .component("user-validator")
            .metadata("field", "email")
            .metadata("value", "invalid-email")
            .build();

        let display = format!("{}", context);
        assert!(display.contains("Validation failed"));
        assert!(display.contains("field: email"));
        assert!(display.contains("value: invalid-email"));
        assert!(display.contains("Component: user-validator"));
    }

    #[test]
    fn test_error_context_metadata() {
        let mut context = ErrorContext::new("Test context");
        context = context.with_metadata("key1", "value1");
        context = context.with_metadata("key2", "value2");

        assert!(context.has_metadata("key1"));
        assert!(context.has_metadata("key2"));
        assert!(!context.has_metadata("key3"));

        assert_eq!(context.get_metadata("key1"), Some(&"value1".to_string()));
        assert_eq!(context.get_metadata("key2"), Some(&"value2".to_string()));
        assert_eq!(context.get_metadata("key3"), None);

        let keys: Vec<&String> = context.metadata_keys().collect();
        assert_eq!(keys.len(), 2);
        assert!(keys.contains(&&"key1".to_string()));
        assert!(keys.contains(&&"key2".to_string()));
    }
}

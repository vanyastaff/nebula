//! Error context for providing additional information

// Standard library
use std::collections::HashMap;
use std::fmt;

// External dependencies
use serde::{Deserialize, Serialize};

/// Identifiers grouped together to reduce memory overhead
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextIds {
    /// User ID associated with the error
    pub user_id: String,
    /// Tenant ID associated with the error
    pub tenant_id: String,
    /// Request ID or correlation ID
    pub request_id: String,
    /// Component or module where the error occurred
    pub component: String,
    /// Operation being performed when the error occurred
    pub operation: String,
}

/// Additional context information for errors
///
/// Optimized for memory efficiency:
/// - Lazy allocation for metadata (only when used)
/// - Grouped optional fields to reduce padding
/// - Total size: ~64 bytes vs previous 232 bytes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorContext {
    /// Human-readable context description
    pub description: String,
    /// Additional key-value pairs for context (lazy allocated)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Box<HashMap<String, String>>>,
    /// Stack trace or call chain information
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stack_trace: Option<Box<String>>,
    /// Timestamp when the error occurred
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<chrono::DateTime<chrono::Utc>>,
    /// Grouped identifiers (lazy allocated)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ids: Option<Box<ContextIds>>,
}

impl ErrorContext {
    // Accessor methods for IDs
    /// Get user ID if set
    #[must_use]
    pub fn user_id(&self) -> Option<&str> {
        self.ids.as_ref().map(|ids| ids.user_id.as_str())
    }

    /// Get tenant ID if set
    #[must_use]
    pub fn tenant_id(&self) -> Option<&str> {
        self.ids.as_ref().map(|ids| ids.tenant_id.as_str())
    }

    /// Get request ID if set
    #[must_use]
    pub fn request_id(&self) -> Option<&str> {
        self.ids.as_ref().map(|ids| ids.request_id.as_str())
    }

    /// Get component if set
    #[must_use]
    pub fn component(&self) -> Option<&str> {
        self.ids.as_ref().map(|ids| ids.component.as_str())
    }

    /// Get operation if set
    #[must_use]
    pub fn operation(&self) -> Option<&str> {
        self.ids.as_ref().map(|ids| ids.operation.as_str())
    }

    /// Create a new error context without timestamp (more efficient for simple errors)
    pub fn new(description: impl Into<String>) -> Self {
        Self {
            description: description.into(),
            metadata: None,
            stack_trace: None,
            timestamp: None,
            ids: None,
        }
    }

    /// Create a new error context with timestamp
    pub fn with_timestamp_now(description: impl Into<String>) -> Self {
        Self {
            description: description.into(),
            metadata: None,
            stack_trace: None,
            timestamp: Some(chrono::Utc::now()),
            ids: None,
        }
    }

    /// Add timestamp to existing context
    #[must_use]
    pub fn set_timestamp(mut self) -> Self {
        self.timestamp = Some(chrono::Utc::now());
        self
    }

    /// Add metadata key-value pair (lazy allocation)
    #[must_use]
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        let metadata = self
            .metadata
            .get_or_insert_with(|| Box::new(HashMap::new()));
        metadata.insert(key.into(), value.into());
        self
    }

    /// Add stack trace information
    #[must_use]
    pub fn with_stack_trace(mut self, stack_trace: impl Into<String>) -> Self {
        self.stack_trace = Some(Box::new(stack_trace.into()));
        self
    }

    /// Set user ID (lazy allocation of ids struct)
    #[must_use]
    pub fn with_user_id(mut self, user_id: impl Into<String>) -> Self {
        self.ensure_ids().user_id = user_id.into();
        self
    }

    /// Set tenant ID (lazy allocation of ids struct)
    #[must_use]
    pub fn with_tenant_id(mut self, tenant_id: impl Into<String>) -> Self {
        self.ensure_ids().tenant_id = tenant_id.into();
        self
    }

    /// Set request ID (lazy allocation of ids struct)
    #[must_use]
    pub fn with_request_id(mut self, request_id: impl Into<String>) -> Self {
        self.ensure_ids().request_id = request_id.into();
        self
    }

    /// Set component name (lazy allocation of ids struct)
    #[must_use]
    pub fn with_component(mut self, component: impl Into<String>) -> Self {
        self.ensure_ids().component = component.into();
        self
    }

    /// Set operation name (lazy allocation of ids struct)
    #[must_use]
    pub fn with_operation(mut self, operation: impl Into<String>) -> Self {
        self.ensure_ids().operation = operation.into();
        self
    }

    /// Ensure ids struct is allocated
    fn ensure_ids(&mut self) -> &mut ContextIds {
        self.ids.get_or_insert_with(|| {
            Box::new(ContextIds {
                user_id: String::new(),
                tenant_id: String::new(),
                request_id: String::new(),
                component: String::new(),
                operation: String::new(),
            })
        })
    }

    /// Get metadata value by key
    #[must_use]
    pub fn get_metadata(&self, key: &str) -> Option<&String> {
        self.metadata.as_ref()?.get(key)
    }

    /// Check if context has specific metadata
    #[must_use]
    pub fn has_metadata(&self, key: &str) -> bool {
        self.metadata
            .as_ref()
            .is_some_and(|m| m.contains_key(key))
    }

    /// Get all metadata keys
    pub fn metadata_keys(&self) -> impl Iterator<Item = &String> {
        self.metadata
            .as_ref()
            .map(|m| m.keys())
            .into_iter()
            .flatten()
    }

    /// Get all metadata entries
    pub fn metadata_entries(&self) -> impl Iterator<Item = (&String, &String)> {
        self.metadata
            .as_ref()
            .map(|m| m.iter())
            .into_iter()
            .flatten()
    }
}

impl ErrorContext {
    /// Helper function to format metadata for Display
    fn fmt_metadata(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(ref metadata) = self.metadata
            && !metadata.is_empty()
        {
            write!(f, " [")?;
            let mut first = true;
            for (key, value) in metadata.iter() {
                if !first {
                    write!(f, ", ")?;
                }
                write!(f, "{key}: {value}")?;
                first = false;
            }
            write!(f, "]")?;
        }
        Ok(())
    }
}

impl fmt::Display for ErrorContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.description)?;
        self.fmt_metadata(f)?;

        if let Some(ref ids) = self.ids {
            if !ids.component.is_empty() {
                write!(f, " (Component: {})", ids.component)?;
            }

            if !ids.operation.is_empty() {
                write!(f, " (Operation: {})", ids.operation)?;
            }

            if !ids.user_id.is_empty() {
                write!(f, " (User: {})", ids.user_id)?;
            }

            if !ids.tenant_id.is_empty() {
                write!(f, " (Tenant: {})", ids.tenant_id)?;
            }

            if !ids.request_id.is_empty() {
                write!(f, " (Request: {})", ids.request_id)?;
            }
        }

        Ok(())
    }
}

impl Default for ErrorContext {
    fn default() -> Self {
        Self {
            description: "Unknown error context".to_string(),
            metadata: None,
            stack_trace: None,
            timestamp: None,
            ids: None,
        }
    }
}

/// Builder pattern for [`ErrorContext`]
///
/// Provides a fluent API for constructing error contexts with optional fields.
pub struct ErrorContextBuilder {
    context: ErrorContext,
}

impl ErrorContextBuilder {
    /// Create a new builder
    pub fn new(description: impl Into<String>) -> Self {
        Self {
            context: ErrorContext::new(description),
        }
    }

    /// Add metadata key-value pair
    #[must_use]
    pub fn metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.context = self.context.with_metadata(key, value);
        self
    }

    /// Add stack trace information
    #[must_use]
    pub fn stack_trace(mut self, stack_trace: impl Into<String>) -> Self {
        self.context = self.context.with_stack_trace(stack_trace);
        self
    }

    /// Set user ID associated with the error
    #[must_use]
    pub fn user_id(mut self, user_id: impl Into<String>) -> Self {
        self.context = self.context.with_user_id(user_id);
        self
    }

    /// Set tenant ID for multi-tenant systems
    #[must_use]
    pub fn tenant_id(mut self, tenant_id: impl Into<String>) -> Self {
        self.context = self.context.with_tenant_id(tenant_id);
        self
    }

    /// Set request or correlation ID
    #[must_use]
    pub fn request_id(mut self, request_id: impl Into<String>) -> Self {
        self.context = self.context.with_request_id(request_id);
        self
    }

    /// Set component or module name
    #[must_use]
    pub fn component(mut self, component: impl Into<String>) -> Self {
        self.context = self.context.with_component(component);
        self
    }

    /// Set operation being performed
    #[must_use]
    pub fn operation(mut self, operation: impl Into<String>) -> Self {
        self.context = self.context.with_operation(operation);
        self
    }

    /// Build the final [`ErrorContext`]
    #[must_use]
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
        assert!(context.timestamp.is_none()); // New context is lightweight, no timestamp by default

        let context_with_time = ErrorContext::with_timestamp_now("Error");
        assert!(context_with_time.timestamp.is_some());
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
        assert_eq!(context.component(), Some("user-service"));
        assert_eq!(context.operation(), Some("create_user"));
        assert_eq!(context.user_id(), Some("user123"));
        assert_eq!(context.tenant_id(), Some("tenant456"));
        assert_eq!(
            context.get_metadata("endpoint"),
            Some(&"/api/users".to_string())
        );
        assert_eq!(context.get_metadata("method"), Some(&"POST".to_string()));
    }

    #[test]
    fn test_error_context_display() {
        let context = ErrorContextBuilder::new("Validation failed")
            .component("user-validator")
            .metadata("field", "email")
            .metadata("value", "invalid-email")
            .build();

        let display = format!("{context}");
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

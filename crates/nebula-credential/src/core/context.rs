use std::collections::HashMap;

/// Context passed to credential operations
#[derive(Default)]
pub struct CredentialContext {
    /// Additional parameters
    pub params: HashMap<String, serde_json::Value>,

    /// Request metadata
    pub metadata: HashMap<String, String>,
}

impl CredentialContext {
    /// Create new context
    pub fn new() -> Self {
        Self::default()
    }

    /// Set parameter
    pub fn set_param(&mut self, key: impl Into<String>, value: serde_json::Value) {
        self.params.insert(key.into(), value);
    }

    /// Get parameter
    pub fn get_param(&self, key: &str) -> Option<&serde_json::Value> {
        self.params.get(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_context_new() {
        let context = CredentialContext::new();
        assert!(context.params.is_empty());
        assert!(context.metadata.is_empty());
    }

    #[test]
    fn test_context_default() {
        let context = CredentialContext::default();
        assert!(context.params.is_empty());
        assert!(context.metadata.is_empty());
    }

    #[test]
    fn test_context_set_and_get_param() {
        let mut context = CredentialContext::new();
        context.set_param("key1", json!("value1"));
        context.set_param("key2", json!(42));

        assert_eq!(context.get_param("key1"), Some(&json!("value1")));
        assert_eq!(context.get_param("key2"), Some(&json!(42)));
        assert_eq!(context.get_param("nonexistent"), None);
    }

    #[test]
    fn test_context_param_overwrite() {
        let mut context = CredentialContext::new();
        context.set_param("key", json!("first"));
        context.set_param("key", json!("second"));

        assert_eq!(context.get_param("key"), Some(&json!("second")));
    }

    #[test]
    fn test_context_metadata_field() {
        let mut context = CredentialContext::new();
        context.metadata.insert("request_id".to_string(), "abc-123".to_string());
        context.metadata.insert("user_id".to_string(), "user-456".to_string());

        assert_eq!(context.metadata.len(), 2);
        assert_eq!(context.metadata.get("request_id"), Some(&"abc-123".to_string()));
    }

    #[test]
    fn test_context_params_with_complex_json() {
        let mut context = CredentialContext::new();
        context.set_param(
            "config",
            json!({
                "timeout": 30,
                "retries": 3,
                "enabled": true
            }),
        );

        let config = context.get_param("config").unwrap();
        assert_eq!(config["timeout"], 30);
        assert_eq!(config["retries"], 3);
        assert_eq!(config["enabled"], true);
    }

    #[test]
    fn test_context_empty_after_creation() {
        let context = CredentialContext::new();
        assert_eq!(context.params.len(), 0);
        assert_eq!(context.metadata.len(), 0);
    }
}

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
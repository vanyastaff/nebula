//! No-operation validator that always passes

use crate::core::{ConfigResult, ConfigValidator};
use async_trait::async_trait;

/// No-op validator that always passes
#[derive(Debug, Clone, Default)]
pub struct NoOpValidator;

impl NoOpValidator {
    /// Create a new no-op validator
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ConfigValidator for NoOpValidator {
    async fn validate(&self, _data: &serde_json::Value) -> ConfigResult<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_noop_validator_always_passes() {
        let v = NoOpValidator::new();
        assert!(v.validate(&json!(null)).await.is_ok());
        assert!(v.validate(&json!({"any": "data"})).await.is_ok());
        assert!(v.validate(&json!([1, 2, 3])).await.is_ok());
        assert!(v.schema().is_none());
    }
}

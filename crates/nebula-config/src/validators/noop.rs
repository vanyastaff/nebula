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
//! Composite validator that combines multiple validators

use crate::core::{ConfigResult, ConfigValidator};
use async_trait::async_trait;
use std::sync::Arc;

/// Composite validator that runs multiple validators
pub struct CompositeValidator {
    /// List of validators
    validators: Vec<Arc<dyn ConfigValidator>>,
    /// Whether to fail fast on first error
    fail_fast: bool,
    /// Whether to run validators in parallel
    parallel: bool,
}

impl std::fmt::Debug for CompositeValidator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompositeValidator")
            .field("validators", &format!("{} validators", self.validators.len()))
            .field("fail_fast", &self.fail_fast)
            .field("parallel", &self.parallel)
            .finish()
    }
}

impl CompositeValidator {
    /// Create a new composite validator
    pub fn new() -> Self {
        Self {
            validators: Vec::new(),
            fail_fast: true,
            parallel: false,
        }
    }

    /// Set whether to fail fast
    pub fn with_fail_fast(mut self, fail_fast: bool) -> Self {
        self.fail_fast = fail_fast;
        self
    }

    /// Set whether to run validators in parallel
    pub fn with_parallel(mut self, parallel: bool) -> Self {
        self.parallel = parallel;
        self
    }

    /// Add a validator
    pub fn add_validator<V: ConfigValidator + 'static>(mut self, validator: V) -> Self {
        self.validators.push(Arc::new(validator));
        self
    }

    /// Add a shared validator
    pub fn add_shared_validator(mut self, validator: Arc<dyn ConfigValidator>) -> Self {
        self.validators.push(validator);
        self
    }

    /// Run validators sequentially
    async fn validate_sequential(&self, data: &serde_json::Value) -> ConfigResult<()> {
        let mut errors = Vec::new();

        for validator in &self.validators {
            match validator.validate(data).await {
                Ok(()) => {}
                Err(e) => {
                    if self.fail_fast {
                        return Err(e);
                    }
                    errors.push(e);
                }
            }
        }

        if !errors.is_empty() {
            return Err(crate::core::ConfigError::validation_error(
                format!("Validation failed: {} errors", errors.len()),
                None,
            ));
        }

        Ok(())
    }

    /// Run validators in parallel
    async fn validate_parallel(&self, data: &serde_json::Value) -> ConfigResult<()> {
        let futures: Vec<_> = self.validators
            .iter()
            .map(|validator| {
                let validator = Arc::clone(validator);
                let data = data.clone();
                async move { validator.validate(&data).await }
            })
            .collect();

        let results = futures::future::join_all(futures).await;

        let mut errors = Vec::new();
        for result in results {
            if let Err(e) = result {
                if self.fail_fast {
                    return Err(e);
                }
                errors.push(e);
            }
        }

        if !errors.is_empty() {
            return Err(crate::core::ConfigError::validation_error(
                format!("Validation failed: {} errors", errors.len()),
                None,
            ));
        }

        Ok(())
    }
}

impl Default for CompositeValidator {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ConfigValidator for CompositeValidator {
    async fn validate(&self, data: &serde_json::Value) -> ConfigResult<()> {
        if self.validators.is_empty() {
            return Ok(());
        }

        if self.parallel {
            self.validate_parallel(data).await
        } else {
            self.validate_sequential(data).await
        }
    }

    fn schema(&self) -> Option<serde_json::Value> {
        // Return combined schemas if all validators have schemas
        let schemas: Vec<_> = self.validators
            .iter()
            .filter_map(|v| v.schema())
            .collect();

        if schemas.len() == self.validators.len() && !schemas.is_empty() {
            // Combine schemas using allOf
            Some(serde_json::json!({
                "allOf": schemas
            }))
        } else {
            None
        }
    }
}
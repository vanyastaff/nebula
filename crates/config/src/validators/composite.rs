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
            .field(
                "validators",
                &format!("{} validators", self.validators.len()),
            )
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
    #[must_use = "builder methods must be chained or built"]
    pub fn with_fail_fast(mut self, fail_fast: bool) -> Self {
        self.fail_fast = fail_fast;
        self
    }

    /// Set whether to run validators in parallel
    #[must_use = "builder methods must be chained or built"]
    pub fn with_parallel(mut self, parallel: bool) -> Self {
        self.parallel = parallel;
        self
    }

    /// Add a validator
    #[must_use = "builder methods must be chained or built"]
    pub fn add_validator<V: ConfigValidator + 'static>(mut self, validator: V) -> Self {
        self.validators.push(Arc::new(validator));
        self
    }

    /// Add a shared validator
    #[must_use = "builder methods must be chained or built"]
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
        let futures: Vec<_> = self
            .validators
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
        let schemas: Vec<_> = self.validators.iter().filter_map(|v| v.schema()).collect();

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validators::FunctionValidator;
    use serde_json::json;

    #[tokio::test]
    async fn test_composite_empty_passes() {
        let v = CompositeValidator::new();
        assert!(v.validate(&json!({"any": "data"})).await.is_ok());
    }

    #[tokio::test]
    async fn test_composite_all_pass() {
        let v = CompositeValidator::new()
            .add_validator(FunctionValidator::new(|_| Ok(())))
            .add_validator(FunctionValidator::new(|_| Ok(())));
        assert!(v.validate(&json!({})).await.is_ok());
    }

    #[tokio::test]
    async fn test_composite_fail_fast() {
        // fail_fast = true (default): stops at first error
        let v = CompositeValidator::new()
            .with_fail_fast(true)
            .add_validator(FunctionValidator::new(|_| {
                Err(crate::core::ConfigError::validation("error 1"))
            }))
            .add_validator(FunctionValidator::new(|_| {
                Err(crate::core::ConfigError::validation("error 2"))
            }));
        let err = v.validate(&json!({})).await.unwrap_err();
        assert!(err.to_string().contains("error 1"));

        // fail_fast = false: collects all errors
        let v2 = CompositeValidator::new()
            .with_fail_fast(false)
            .add_validator(FunctionValidator::new(|_| {
                Err(crate::core::ConfigError::validation("err a"))
            }))
            .add_validator(FunctionValidator::new(|_| {
                Err(crate::core::ConfigError::validation("err b"))
            }));
        let err = v2.validate(&json!({})).await.unwrap_err();
        assert!(err.to_string().contains("2 errors"));
    }

    #[tokio::test]
    async fn test_composite_schema_combination() {
        let with_schema = FunctionValidator::with_schema(|_| Ok(()), json!({"type": "object"}));
        let no_schema = FunctionValidator::new(|_| Ok(()));

        // All have schemas → allOf
        let v = CompositeValidator::new()
            .add_validator(FunctionValidator::with_schema(
                |_| Ok(()),
                json!({"type": "object"}),
            ))
            .add_validator(FunctionValidator::with_schema(
                |_| Ok(()),
                json!({"required": ["name"]}),
            ));
        let schema = v.schema().unwrap();
        assert!(schema.get("allOf").is_some());

        // Mixed → None
        let v2 = CompositeValidator::new()
            .add_validator(with_schema)
            .add_validator(no_schema);
        assert!(v2.schema().is_none());

        // Empty → None
        assert!(CompositeValidator::new().schema().is_none());
    }
}

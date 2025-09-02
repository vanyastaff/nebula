//! Pipeline stage implementation
//! 
//! This module defines the PipelineStage structure and related types for
//! organizing validators into execution stages within a pipeline.

use super::{StageType, StageConfig, RetryConfig};
use crate::{
    Validatable, ValidatorMetadata, ValidationComplexity, ValidationResult,
    ValidationContext, ErrorCode, ErrorSeverity,
};
use serde_json::Value;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{info, warn, error, instrument};

/// A stage in the validation pipeline
/// 
/// A stage represents a logical grouping of validators that are executed
/// together. The stage type determines how the validators are executed:
/// - Sequential: Validators run one after another
/// - Parallel: Validators run concurrently
/// - Conditional: Validators run based on conditions
#[derive(Debug, Clone)]
pub struct PipelineStage {
    /// Unique identifier for the stage
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Stage description
    pub description: Option<String>,
    /// Stage type determining execution behavior
    pub stage_type: StageType,
    /// Validators in this stage
    pub validators: Vec<Arc<dyn Validatable + Send + Sync>>,
    /// Stage configuration
    pub config: StageConfig,
}

/// Stage execution result
#[derive(Debug, Clone)]
pub struct StageExecutionResult {
    /// Stage ID
    pub stage_id: String,
    /// Whether the stage execution was successful
    pub success: bool,
    /// Total execution time
    pub execution_time: Duration,
    /// Number of validators executed
    pub validators_executed: usize,
    /// Number of successful validations
    pub successful_validations: usize,
    /// Number of failed validations
    pub failed_validations: usize,
    /// Validation results from each validator
    pub validator_results: Vec<ValidatorExecutionResult>,
    /// Stage-level errors
    pub stage_errors: Vec<StageError>,
    /// Stage metrics
    pub metrics: StageMetrics,
}

/// Result from executing a single validator
#[derive(Debug, Clone)]
pub struct ValidatorExecutionResult {
    /// Validator metadata
    pub metadata: ValidatorMetadata,
    /// Whether validation was successful
    pub success: bool,
    /// Execution time for this validator
    pub execution_time: Duration,
    /// Validation result
    pub result: ValidationResult<()>,
    /// Whether this result was cached
    pub cached: bool,
}

/// Error that occurred during stage execution
#[derive(Debug, Clone)]
pub struct StageError {
    /// Error code
    pub code: ErrorCode,
    /// Error message
    pub message: String,
    /// Error severity
    pub severity: ErrorSeverity,
    /// Timestamp when error occurred
    pub timestamp: Instant,
    /// Additional context
    pub context: Option<Value>,
}

/// Metrics for stage execution
#[derive(Debug, Clone, Default)]
pub struct StageMetrics {
    /// Total execution time
    pub total_time: Duration,
    /// Average validator execution time
    pub avg_validator_time: Duration,
    /// Memory usage during execution
    pub memory_usage_bytes: u64,
    /// CPU usage percentage
    pub cpu_usage_percent: f64,
    /// Number of cache hits
    pub cache_hits: usize,
    /// Number of cache misses
    pub cache_misses: usize,
    /// Number of retries performed
    pub retries: usize,
}

impl PipelineStage {
    /// Create a new pipeline stage
    pub fn new(id: String, name: String) -> Self {
        Self {
            id,
            name,
            description: None,
            stage_type: StageType::Parallel,
            validators: Vec::new(),
            config: StageConfig::default(),
        }
    }

    /// Add a validator to this stage
    pub fn add_validator<V>(mut self, validator: V) -> Self
    where
        V: Validatable + Send + Sync + 'static,
    {
        self.validators.push(Arc::new(validator));
        self
    }

    /// Set the stage type
    pub fn with_stage_type(mut self, stage_type: StageType) -> Self {
        self.stage_type = stage_type;
        self
    }

    /// Set the stage description
    pub fn with_description(mut self, description: String) -> Self {
        self.description = Some(description);
        self
    }

    /// Set the stage configuration
    pub fn with_config(mut self, config: StageConfig) -> Self {
        self.config = config;
        self
    }

    /// Get the total complexity of all validators in this stage
    pub fn complexity(&self) -> ValidationComplexity {
        if self.validators.is_empty() {
            return ValidationComplexity::Simple;
        }

        let total_complexity: usize = self.validators.iter()
            .map(|validator| match validator.complexity() {
                ValidationComplexity::Simple => 1,
                ValidationComplexity::Moderate => 2,
                ValidationComplexity::Complex => 3,
                ValidationComplexity::VeryComplex => 4,
            })
            .sum();

        match total_complexity {
            0..=1 => ValidationComplexity::Simple,
            2..=3 => ValidationComplexity::Moderate,
            4..=6 => ValidationComplexity::Complex,
            _ => ValidationComplexity::VeryComplex,
        }
    }

    /// Validate that the stage configuration is correct
    pub fn validate_config(&self) -> ValidationResult<()> {
        if self.validators.is_empty() {
            return ValidationResult::failure(
                ErrorCode::InvalidConfiguration,
                "Stage must have at least one validator".to_string(),
                ErrorSeverity::Error,
            );
        }

        if let Some(max_concurrency) = self.config.max_concurrency {
            if max_concurrency == 0 {
                return ValidationResult::failure(
                    ErrorCode::InvalidConfiguration,
                    "Max concurrency must be greater than 0".to_string(),
                    ErrorSeverity::Error,
                );
            }
        }

        ValidationResult::success(())
    }

    /// Execute the stage with the given value and context
    #[instrument(skip(self, value, context), fields(stage_id = %self.id))]
    pub async fn execute(
        &self,
        value: &Value,
        context: &ValidationContext,
    ) -> StageExecutionResult {
        let start_time = Instant::now();
        let mut validator_results = Vec::new();
        let mut stage_errors = Vec::new();
        let mut successful_validations = 0;
        let mut failed_validations = 0;

        info!("Executing stage: {}", self.name);

        // Validate stage configuration
        let config_validation = self.validate_config();
        if config_validation.is_failure() {
            let error = StageError {
                code: ErrorCode::InvalidConfiguration,
                message: "Stage configuration is invalid".to_string(),
                severity: ErrorSeverity::Error,
                timestamp: Instant::now(),
                context: Some(serde_json::to_value(&self.config).unwrap_or_default()),
            };
            stage_errors.push(error);

            return StageExecutionResult {
                stage_id: self.id.clone(),
                success: false,
                execution_time: start_time.elapsed(),
                validators_executed: 0,
                successful_validations: 0,
                failed_validations: 0,
                validator_results,
                stage_errors,
                metrics: StageMetrics::default(),
            };
        }

        // Execute validators based on stage type
        match self.stage_type {
            StageType::Sequential => {
                self.execute_sequential(value, context, &mut validator_results, &mut successful_validations, &mut failed_validations).await;
            }
            StageType::Parallel => {
                self.execute_parallel(value, context, &mut validator_results, &mut successful_validations, &mut failed_validations).await;
            }
            StageType::Conditional => {
                self.execute_conditional(value, context, &mut validator_results, &mut successful_validations, &mut failed_validations).await;
            }
        }

        let execution_time = start_time.elapsed();
        let success = failed_validations == 0 || !self.config.fail_fast;

        info!(
            "Stage execution completed: {} (success: {}, time: {:?}, validators: {})",
            self.name, success, execution_time, validator_results.len()
        );

        StageExecutionResult {
            stage_id: self.id.clone(),
            success,
            execution_time,
            validators_executed: validator_results.len(),
            successful_validations,
            failed_validations,
            validator_results,
            stage_errors,
            metrics: self.calculate_metrics(&validator_results, execution_time),
        }
    }

    /// Execute validators sequentially
    async fn execute_sequential(
        &self,
        value: &Value,
        context: &ValidationContext,
        validator_results: &mut Vec<ValidatorExecutionResult>,
        successful_validations: &mut usize,
        failed_validations: &mut usize,
    ) {
        for validator in &self.validators {
            let result = self.execute_validator(validator, value, context).await;
            
            if result.success {
                *successful_validations += 1;
            } else {
                *failed_validations += 1;
                if self.config.fail_fast {
                    break;
                }
            }
            
            validator_results.push(result);
        }
    }

    /// Execute validators in parallel
    async fn execute_parallel(
        &self,
        value: &Value,
        context: &ValidationContext,
        validator_results: &mut Vec<ValidatorExecutionResult>,
        successful_validations: &mut usize,
        failed_validations: &mut usize,
    ) {
        let max_concurrency = self.config.max_concurrency.unwrap_or(self.validators.len());
        let semaphore = Arc::new(tokio::sync::Semaphore::new(max_concurrency));
        let mut tasks = Vec::new();

        for validator in &self.validators {
            let permit = semaphore.clone().acquire_owned().await.unwrap();
            let validator = validator.clone();
            let value = value.clone();
            let context = context.clone();

            let task = tokio::spawn(async move {
                let _permit = permit;
                let result = Self::execute_validator_static(&validator, &value, &context).await;
                result
            });

            tasks.push(task);
        }

        // Collect results
        for task in tasks {
            if let Ok(result) = task.await {
                if result.success {
                    *successful_validations += 1;
                } else {
                    *failed_validations += 1;
                }
                validator_results.push(result);
            }
        }
    }

    /// Execute validators conditionally
    async fn execute_conditional(
        &self,
        value: &Value,
        context: &ValidationContext,
        validator_results: &mut Vec<ValidatorExecutionResult>,
        successful_validations: &mut usize,
        failed_validations: &mut usize,
    ) {
        // For now, execute all validators (conditional logic can be added later)
        self.execute_parallel(value, context, validator_results, successful_validations, failed_validations).await;
    }

    /// Execute a single validator
    async fn execute_validator(
        &self,
        validator: &Arc<dyn Validatable + Send + Sync>,
        value: &Value,
        context: &ValidationContext,
    ) -> ValidatorExecutionResult {
        Self::execute_validator_static(validator, value, context).await
    }

    /// Static method to execute a validator (for use in async tasks)
    async fn execute_validator_static(
        validator: &Arc<dyn Validatable + Send + Sync>,
        value: &Value,
        context: &ValidationContext,
    ) -> ValidatorExecutionResult {
        let start_time = Instant::now();
        let metadata = validator.metadata();

        let result = match tokio::time::timeout(
            Duration::from_millis(5000), // 5 second timeout
            validator.validate(value)
        ).await {
            Ok(validation_result) => validation_result,
            Err(_) => ValidationResult::failure(
                ErrorCode::Timeout,
                "Validator execution timed out".to_string(),
                ErrorSeverity::Error,
            ),
        };

        let execution_time = start_time.elapsed();
        let success = result.is_success();

        ValidatorExecutionResult {
            metadata,
            success,
            execution_time,
            result,
            cached: false, // TODO: Implement caching
        }
    }

    /// Calculate metrics for the stage execution
    fn calculate_metrics(
        &self,
        validator_results: &[ValidatorExecutionResult],
        total_time: Duration,
    ) -> StageMetrics {
        if validator_results.is_empty() {
            return StageMetrics::default();
        }

        let total_validator_time: Duration = validator_results
            .iter()
            .map(|result| result.execution_time)
            .sum();

        let avg_validator_time = if validator_results.len() > 0 {
            Duration::from_nanos(total_validator_time.as_nanos() as u64 / validator_results.len() as u64)
        } else {
            Duration::ZERO
        };

        StageMetrics {
            total_time,
            avg_validator_time,
            memory_usage_bytes: 0, // TODO: Implement memory tracking
            cpu_usage_percent: 0.0, // TODO: Implement CPU tracking
            cache_hits: validator_results.iter().filter(|r| r.cached).count(),
            cache_misses: validator_results.iter().filter(|r| !r.cached).count(),
            retries: 0, // TODO: Implement retry tracking
        }
    }
}

impl Default for StageType {
    fn default() -> Self {
        StageType::Parallel
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::create_test_validator;

    #[tokio::test]
    async fn test_stage_creation() {
        let stage = PipelineStage::new("test".to_string(), "Test Stage".to_string());
        assert_eq!(stage.id, "test");
        assert_eq!(stage.name, "Test Stage");
        assert_eq!(stage.stage_type, StageType::Parallel);
        assert!(stage.validators.is_empty());
    }

    #[tokio::test]
    async fn test_stage_add_validator() {
        let validator = create_test_validator();
        let stage = PipelineStage::new("test".to_string(), "Test Stage".to_string())
            .add_validator(validator);

        assert_eq!(stage.validators.len(), 1);
    }

    #[tokio::test]
    async fn test_stage_complexity() {
        let stage = PipelineStage::new("test".to_string(), "Test Stage".to_string());
        assert_eq!(stage.complexity(), ValidationComplexity::Simple);

        let validator = create_test_validator();
        let stage = stage.add_validator(validator);
        assert_eq!(stage.complexity(), ValidationComplexity::Simple);
    }

    #[tokio::test]
    async fn test_stage_config_validation() {
        let stage = PipelineStage::new("test".to_string(), "Test Stage".to_string());
        let result = stage.validate_config();
        assert!(result.is_failure());

        let validator = create_test_validator();
        let stage = stage.add_validator(validator);
        let result = stage.validate_config();
        assert!(result.is_success());
    }

    #[tokio::test]
    async fn test_stage_execution() {
        let validator = create_test_validator();
        let stage = PipelineStage::new("test".to_string(), "Test Stage".to_string())
            .add_validator(validator);

        let value = serde_json::Value::String("test".to_string());
        let context = ValidationContext::default();

        let result = stage.execute(&value, &context).await;
        assert_eq!(result.stage_id, "test");
        assert!(result.success);
        assert_eq!(result.validators_executed, 1);
        assert_eq!(result.successful_validations, 1);
        assert_eq!(result.failed_validations, 0);
    }
}


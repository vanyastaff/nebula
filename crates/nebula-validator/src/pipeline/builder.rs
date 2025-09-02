//! Pipeline builder for constructing validation pipelines
//! 
//! This module provides a fluent builder API for creating complex validation pipelines
//! with multiple stages and validators.

use super::{Pipeline, PipelineStage, PipelineConfig, RetryConfig};
use crate::{
    Validatable, ValidatorMetadata, ValidationComplexity, ValidationResult,
    ErrorCode, ErrorSeverity,
};
use serde_json::Value;
use std::sync::Arc;

/// Fluent builder for creating validation pipelines
/// 
/// The PipelineBuilder provides a type-safe, fluent API for constructing
/// complex validation pipelines with multiple stages and validators.
/// 
/// # Example
/// 
/// ```rust
/// use nebula_validator::pipeline::PipelineBuilder;
/// use nebula_validator::{NotNull, StringLength};
/// 
/// let pipeline = PipelineBuilder::new("user_validation")
///     .with_name("User Registration Validation")
///     .with_description("Validates user registration data")
///     .add_stage("basic_validation")
///         .add_validator(NotNull::new())
///         .add_validator(StringLength::new().min(1).max(100))
///         .build_stage()
///     .add_stage("advanced_validation")
///         .add_validator(EmailValidator::new())
///         .add_validator(PasswordStrengthValidator::new())
///         .build_stage()
///     .with_fail_fast(true)
///     .with_timeout_ms(5000)
///     .build();
/// ```
#[derive(Debug, Clone)]
pub struct PipelineBuilder {
    /// Pipeline ID
    id: String,
    /// Pipeline name
    name: Option<String>,
    /// Pipeline description
    description: Option<String>,
    /// Pipeline stages being built
    stages: Vec<PipelineStage>,
    /// Current stage being built
    current_stage: Option<StageBuilder>,
    /// Pipeline configuration
    config: PipelineConfig,
}

/// Builder for individual pipeline stages
#[derive(Debug, Clone)]
pub struct StageBuilder {
    /// Stage ID
    id: String,
    /// Stage name
    name: Option<String>,
    /// Stage description
    description: Option<String>,
    /// Stage type
    stage_type: StageType,
    /// Validators in this stage
    validators: Vec<Arc<dyn Validatable + Send + Sync>>,
    /// Stage configuration
    config: StageConfig,
}

/// Stage type determines execution behavior
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageType {
    /// Sequential execution - validators run one after another
    Sequential,
    /// Parallel execution - validators run concurrently
    Parallel,
    /// Conditional execution - validators run based on conditions
    Conditional,
}

/// Configuration for individual stages
#[derive(Debug, Clone)]
pub struct StageConfig {
    /// Whether to stop on first validator failure
    pub fail_fast: bool,
    /// Maximum execution time for this stage
    pub timeout_ms: Option<u64>,
    /// Maximum number of concurrent validators (for parallel stages)
    pub max_concurrency: Option<usize>,
    /// Whether to collect detailed metrics for this stage
    pub collect_metrics: bool,
    /// Retry configuration for this stage
    pub retry_config: Option<RetryConfig>,
}

impl Default for StageConfig {
    fn default() -> Self {
        Self {
            fail_fast: true,
            timeout_ms: None,
            max_concurrency: None,
            collect_metrics: true,
            retry_config: None,
        }
    }
}

impl PipelineBuilder {
    /// Create a new pipeline builder with the given ID
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: None,
            description: None,
            stages: Vec::new(),
            current_stage: None,
            config: PipelineConfig::default(),
        }
    }

    /// Set the pipeline name
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set the pipeline description
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Start building a new stage
    pub fn add_stage(mut self, id: impl Into<String>) -> Self {
        // Complete the current stage if it exists
        if let Some(stage_builder) = self.current_stage.take() {
            self.stages.push(stage_builder.build());
        }

        // Start a new stage
        self.current_stage = Some(StageBuilder::new(id));
        self
    }

    /// Add a validator to the current stage
    pub fn add_validator<V>(mut self, validator: V) -> Self
    where
        V: Validatable + Send + Sync + 'static,
    {
        if let Some(ref mut stage_builder) = self.current_stage {
            stage_builder.add_validator(validator);
        }
        self
    }

    /// Set the current stage type
    pub fn with_stage_type(mut self, stage_type: StageType) -> Self {
        if let Some(ref mut stage_builder) = self.current_stage {
            stage_builder.with_stage_type(stage_type);
        }
        self
    }

    /// Set the current stage name
    pub fn with_stage_name(mut self, name: impl Into<String>) -> Self {
        if let Some(ref mut stage_builder) = self.current_stage {
            stage_builder.with_name(name);
        }
        self
    }

    /// Set the current stage description
    pub fn with_stage_description(mut self, description: impl Into<String>) -> Self {
        if let Some(ref mut stage_builder) = self.current_stage {
            stage_builder.with_description(description);
        }
        self
    }

    /// Set the current stage configuration
    pub fn with_stage_config(mut self, config: StageConfig) -> Self {
        if let Some(ref mut stage_builder) = self.current_stage {
            stage_builder.with_config(config);
        }
        self
    }

    /// Complete the current stage and return to pipeline building
    pub fn build_stage(mut self) -> Self {
        if let Some(stage_builder) = self.current_stage.take() {
            self.stages.push(stage_builder.build());
        }
        self
    }

    /// Set pipeline fail-fast behavior
    pub fn with_fail_fast(mut self, fail_fast: bool) -> Self {
        self.config.fail_fast = fail_fast;
        self
    }

    /// Set pipeline timeout
    pub fn with_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.config.timeout_ms = Some(timeout_ms);
        self
    }

    /// Set pipeline configuration
    pub fn with_config(mut self, config: PipelineConfig) -> Self {
        self.config = config;
        self
    }

    /// Enable or disable metrics collection
    pub fn with_metrics(mut self, collect_metrics: bool) -> Self {
        self.config.collect_metrics = collect_metrics;
        self
    }

    /// Enable or disable caching
    pub fn with_caching(mut self, enable_caching: bool) -> Self {
        self.config.enable_caching = enable_caching;
        self
    }

    /// Set maximum concurrency
    pub fn with_max_concurrency(mut self, max_concurrency: usize) -> Self {
        self.config.max_concurrency = max_concurrency;
        self
    }

    /// Set retry configuration
    pub fn with_retry_config(mut self, retry_config: RetryConfig) -> Self {
        self.config.retry_config = Some(retry_config);
        self
    }

    /// Build the final pipeline
    pub fn build(mut self) -> Result<Pipeline, PipelineBuildError> {
        // Complete the current stage if it exists
        if let Some(stage_builder) = self.current_stage.take() {
            self.stages.push(stage_builder.build());
        }

        // Validate the pipeline
        if self.stages.is_empty() {
            return Err(PipelineBuildError::NoStages);
        }

        // Create the pipeline
        let mut pipeline = Pipeline::new(self.id, self.name.unwrap_or_else(|| "Unnamed Pipeline".to_string()));
        
        if let Some(description) = self.description {
            pipeline = pipeline.with_description(description);
        }

        pipeline = pipeline.with_config(self.config);
        
        // Add all stages
        for stage in self.stages {
            pipeline = pipeline.add_stage(stage);
        }

        // Validate the final pipeline
        let validation_result = pipeline.validate_config();
        if validation_result.is_failure() {
            return Err(PipelineBuildError::InvalidConfiguration(
                validation_result.errors().first()
                    .map(|e| e.message.clone())
                    .unwrap_or_else(|| "Unknown validation error".to_string())
            ));
        }

        Ok(pipeline)
    }
}

impl StageBuilder {
    /// Create a new stage builder
    fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: None,
            description: None,
            stage_type: StageType::Parallel,
            validators: Vec::new(),
            config: StageConfig::default(),
        }
    }

    /// Add a validator to this stage
    fn add_validator<V>(&mut self, validator: V)
    where
        V: Validatable + Send + Sync + 'static,
    {
        self.validators.push(Arc::new(validator));
    }

    /// Set the stage type
    fn with_stage_type(mut self, stage_type: StageType) -> Self {
        self.stage_type = stage_type;
        self
    }

    /// Set the stage name
    fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set the stage description
    fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Set the stage configuration
    fn with_config(mut self, config: StageConfig) -> Self {
        self.config = config;
        self
    }

    /// Build the stage
    fn build(self) -> PipelineStage {
        PipelineStage {
            id: self.id,
            name: self.name.unwrap_or_else(|| "Unnamed Stage".to_string()),
            description: self.description,
            stage_type: self.stage_type,
            validators: self.validators,
            config: self.config,
        }
    }
}

/// Error types for pipeline building
#[derive(Debug, thiserror::Error)]
pub enum PipelineBuildError {
    #[error("Pipeline must have at least one stage")]
    NoStages,
    #[error("Invalid pipeline configuration: {0}")]
    InvalidConfiguration(String),
    #[error("Stage validation failed: {0}")]
    StageValidationFailed(String),
    #[error("Builder state error: {0}")]
    BuilderStateError(String),
}

/// Extension trait for adding validators to stages
pub trait StageValidatorExt {
    /// Add a validator to the current stage
    fn add_validator<V>(self, validator: V) -> Self
    where
        V: Validatable + Send + Sync + 'static;
}

impl StageValidatorExt for PipelineBuilder {
    fn add_validator<V>(self, validator: V) -> Self
    where
        V: Validatable + Send + Sync + 'static,
    {
        self.add_validator(validator)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::create_test_validator;

    #[test]
    fn test_pipeline_builder_creation() {
        let builder = PipelineBuilder::new("test");
        assert_eq!(builder.id, "test");
        assert!(builder.name.is_none());
        assert!(builder.stages.is_empty());
    }

    #[test]
    fn test_pipeline_builder_with_name() {
        let builder = PipelineBuilder::new("test")
            .with_name("Test Pipeline");
        assert_eq!(builder.name, Some("Test Pipeline".to_string()));
    }

    #[test]
    fn test_pipeline_builder_add_stage() {
        let builder = PipelineBuilder::new("test")
            .add_stage("stage1");
        assert!(builder.current_stage.is_some());
        assert_eq!(builder.current_stage.as_ref().unwrap().id, "stage1");
    }

    #[test]
    fn test_pipeline_builder_build_empty() {
        let result = PipelineBuilder::new("test").build();
        assert!(result.is_err());
        match result.unwrap_err() {
            PipelineBuildError::NoStages => {},
            _ => panic!("Expected NoStages error"),
        }
    }

    #[test]
    fn test_pipeline_builder_build_success() {
        let validator = create_test_validator();
        let pipeline = PipelineBuilder::new("test")
            .with_name("Test Pipeline")
            .add_stage("stage1")
                .add_validator(validator)
                .build_stage()
            .build()
            .expect("Pipeline should build successfully");

        assert_eq!(pipeline.id, "test");
        assert_eq!(pipeline.name, "Test Pipeline");
        assert_eq!(pipeline.stages.len(), 1);
        assert_eq!(pipeline.stages[0].id, "stage1");
        assert_eq!(pipeline.stages[0].validators.len(), 1);
    }

    #[test]
    fn test_stage_builder() {
        let mut stage_builder = StageBuilder::new("test_stage");
        let validator = create_test_validator();
        stage_builder.add_validator(validator);

        let stage = stage_builder.build();
        assert_eq!(stage.id, "test_stage");
        assert_eq!(stage.validators.len(), 1);
        assert_eq!(stage.stage_type, StageType::Parallel);
    }
}


//! Pipeline validation system
//! 
//! This module provides a pipeline-based validation system that allows chaining
//! multiple validation stages together for complex validation workflows.

pub mod builder;
pub mod stage;
pub mod executor;
pub mod result;
pub mod metrics;

// Re-export main types
pub use builder::PipelineBuilder;
pub use stage::{PipelineStage, StageType, StageConfig};
pub use executor::{PipelineExecutor, ExecutionConfig, ExecutionContext};
pub use result::{PipelineResult, StageResult, PipelineError};
pub use metrics::{PipelineMetrics, StageMetrics, ExecutionMetrics};

use crate::{
    ValidationResult, ValidationContext, Validatable, ValidatorMetadata,
    ValidationComplexity, ValidationError, ErrorCode, ErrorSeverity,
};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn, error, instrument};

/// Pipeline validation system for complex validation workflows
/// 
/// A pipeline consists of multiple stages that are executed in sequence.
/// Each stage can contain one or more validators that run in parallel.
/// The pipeline provides comprehensive error handling, metrics collection,
/// and result aggregation.
#[derive(Debug, Clone)]
pub struct Pipeline {
    /// Unique identifier for the pipeline
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Pipeline description
    pub description: Option<String>,
    /// Validation stages in execution order
    pub stages: Vec<PipelineStage>,
    /// Pipeline configuration
    pub config: PipelineConfig,
    /// Pipeline metadata
    pub metadata: ValidatorMetadata,
}

/// Configuration for pipeline execution
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    /// Maximum execution time for the entire pipeline
    pub timeout_ms: Option<u64>,
    /// Whether to stop on first stage failure
    pub fail_fast: bool,
    /// Whether to collect detailed metrics
    pub collect_metrics: bool,
    /// Whether to enable result caching
    pub enable_caching: bool,
    /// Maximum number of concurrent validators per stage
    pub max_concurrency: usize,
    /// Retry configuration for failed stages
    pub retry_config: Option<RetryConfig>,
}

/// Retry configuration for pipeline stages
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts
    pub max_attempts: u32,
    /// Initial delay between retries in milliseconds
    pub initial_delay_ms: u64,
    /// Maximum delay between retries in milliseconds
    pub max_delay_ms: u64,
    /// Backoff multiplier for delay calculation
    pub backoff_multiplier: f64,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            timeout_ms: Some(30_000), // 30 seconds
            fail_fast: true,
            collect_metrics: true,
            enable_caching: false,
            max_concurrency: 10,
            retry_config: None,
        }
    }
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_delay_ms: 100,
            max_delay_ms: 5_000,
            backoff_multiplier: 2.0,
        }
    }
}

impl Pipeline {
    /// Create a new pipeline with the given ID and name
    pub fn new(id: String, name: String) -> Self {
        Self {
            id,
            name,
            description: None,
            stages: Vec::new(),
            config: PipelineConfig::default(),
            metadata: ValidatorMetadata::new(
                "pipeline".to_string(),
                "Pipeline Validator".to_string(),
                crate::ValidatorCategory::Composite,
            ),
        }
    }

    /// Add a stage to the pipeline
    pub fn add_stage(mut self, stage: PipelineStage) -> Self {
        self.stages.push(stage);
        self
    }

    /// Set pipeline configuration
    pub fn with_config(mut self, config: PipelineConfig) -> Self {
        self.config = config;
        self
    }

    /// Set pipeline description
    pub fn with_description(mut self, description: String) -> Self {
        self.description = Some(description);
        self
    }

    /// Get the total complexity of all stages
    pub fn complexity(&self) -> ValidationComplexity {
        if self.stages.is_empty() {
            return ValidationComplexity::Simple;
        }

        let total_validators: usize = self.stages.iter()
            .map(|stage| stage.validators.len())
            .sum();

        match total_validators {
            0..=1 => ValidationComplexity::Simple,
            2..=5 => ValidationComplexity::Moderate,
            6..=15 => ValidationComplexity::Complex,
            _ => ValidationComplexity::VeryComplex,
        }
    }

    /// Validate that the pipeline configuration is correct
    pub fn validate_config(&self) -> ValidationResult<()> {
        if self.stages.is_empty() {
            return ValidationResult::failure(
                ErrorCode::InvalidConfiguration,
                "Pipeline must have at least one stage".to_string(),
                ErrorSeverity::Error,
            );
        }

        for (index, stage) in self.stages.iter().enumerate() {
            if stage.validators.is_empty() {
                return ValidationResult::failure(
                    ErrorCode::InvalidConfiguration,
                    format!("Stage {} must have at least one validator", index),
                    ErrorSeverity::Error,
                );
            }
        }

        ValidationResult::success(())
    }
}

/// Pipeline execution statistics
#[derive(Debug, Clone, Default)]
pub struct PipelineStats {
    /// Total execution time in milliseconds
    pub total_time_ms: u64,
    /// Number of stages executed
    pub stages_executed: usize,
    /// Number of validators executed
    pub validators_executed: usize,
    /// Number of successful validations
    pub successful_validations: usize,
    /// Number of failed validations
    pub failed_validations: usize,
    /// Number of retries performed
    pub retries_performed: usize,
    /// Cache hit rate (0.0 to 1.0)
    pub cache_hit_rate: f64,
}

impl PipelineStats {
    /// Calculate success rate
    pub fn success_rate(&self) -> f64 {
        let total = self.successful_validations + self.failed_validations;
        if total == 0 {
            0.0
        } else {
            self.successful_validations as f64 / total as f64
        }
    }

    /// Calculate average execution time per validator
    pub fn avg_validator_time_ms(&self) -> f64 {
        if self.validators_executed == 0 {
            0.0
        } else {
            self.total_time_ms as f64 / self.validators_executed as f64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::create_test_validator;

    #[test]
    fn test_pipeline_creation() {
        let pipeline = Pipeline::new("test".to_string(), "Test Pipeline".to_string());
        assert_eq!(pipeline.id, "test");
        assert_eq!(pipeline.name, "Test Pipeline");
        assert!(pipeline.stages.is_empty());
    }

    #[test]
    fn test_pipeline_complexity() {
        let pipeline = Pipeline::new("test".to_string(), "Test Pipeline".to_string());
        assert_eq!(pipeline.complexity(), ValidationComplexity::Simple);
    }

    #[test]
    fn test_pipeline_config_validation() {
        let pipeline = Pipeline::new("test".to_string(), "Test Pipeline".to_string());
        let result = pipeline.validate_config();
        assert!(result.is_failure());
    }

    #[test]
    fn test_pipeline_stats() {
        let stats = PipelineStats {
            successful_validations: 8,
            failed_validations: 2,
            validators_executed: 10,
            total_time_ms: 1000,
            ..Default::default()
        };

        assert_eq!(stats.success_rate(), 0.8);
        assert_eq!(stats.avg_validator_time_ms(), 100.0);
    }
}


//! Pipeline result types and utilities
//! 
//! This module defines result types for pipeline execution, including
//! comprehensive error handling and result aggregation.

use super::{
    StageExecutionResult, StageError, StageMetrics, PipelineStats, PipelineMetrics,
    PipelineError,
};
use crate::{
    ValidationResult, ValidationError, ErrorCode, ErrorSeverity,
    ValidatorMetadata, ValidationComplexity,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tracing::{info, warn, error};

/// Comprehensive result of pipeline execution
/// 
/// This structure contains all information about a pipeline execution,
/// including stage results, errors, metrics, and statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineResult {
    /// Unique execution ID
    pub execution_id: String,
    /// Pipeline ID
    pub pipeline_id: String,
    /// Pipeline name
    pub pipeline_name: String,
    /// Whether the pipeline execution was successful
    pub success: bool,
    /// Total execution time
    pub execution_time: Duration,
    /// Start time of execution
    pub start_time: Instant,
    /// End time of execution
    pub end_time: Instant,
    /// Results from each stage
    pub stage_results: Vec<StageResult>,
    /// Pipeline-level errors
    pub pipeline_errors: Vec<PipelineError>,
    /// Execution statistics
    pub stats: PipelineStats,
    /// Execution metrics
    pub metrics: PipelineMetrics,
    /// Additional metadata
    pub metadata: Value,
}

/// Result of a single stage execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageResult {
    /// Stage ID
    pub stage_id: String,
    /// Stage name
    pub stage_name: String,
    /// Whether the stage execution was successful
    pub success: bool,
    /// Stage execution time
    pub execution_time: Duration,
    /// Number of validators executed
    pub validators_executed: usize,
    /// Number of successful validations
    pub successful_validations: usize,
    /// Number of failed validations
    pub failed_validations: usize,
    /// Validation results from each validator
    pub validator_results: Vec<ValidatorResult>,
    /// Stage-level errors
    pub stage_errors: Vec<StageError>,
    /// Stage metrics
    pub metrics: StageMetrics,
    /// Stage metadata
    pub metadata: Value,
}

/// Result of a single validator execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorResult {
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
    /// Additional validator-specific data
    pub data: Value,
}

/// Error that occurred during pipeline execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineError {
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
    /// Error category
    pub category: ErrorCategory,
    /// Whether the error is retryable
    pub retryable: bool,
}

/// Categories of pipeline errors
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorCategory {
    /// Configuration-related errors
    Configuration,
    /// Execution-related errors
    Execution,
    /// Timeout errors
    Timeout,
    /// Validation errors
    Validation,
    /// System errors
    System,
    /// Cancellation errors
    Cancellation,
}

impl Default for ErrorCategory {
    fn default() -> Self {
        ErrorCategory::Execution
    }
}

impl PipelineResult {
    /// Create a new pipeline result
    pub fn new(
        execution_id: String,
        pipeline_id: String,
        pipeline_name: String,
        start_time: Instant,
    ) -> Self {
        let end_time = Instant::now();
        Self {
            execution_id,
            pipeline_id,
            pipeline_name,
            success: false,
            execution_time: end_time.duration_since(start_time),
            start_time,
            end_time,
            stage_results: Vec::new(),
            pipeline_errors: Vec::new(),
            stats: PipelineStats::default(),
            metrics: PipelineMetrics::default(),
            metadata: Value::Object(serde_json::Map::new()),
        }
    }

    /// Mark the pipeline execution as successful
    pub fn mark_success(mut self) -> Self {
        self.success = true;
        self.end_time = Instant::now();
        self.execution_time = self.end_time.duration_since(self.start_time);
        self
    }

    /// Add a stage result
    pub fn add_stage_result(mut self, stage_result: StageResult) -> Self {
        self.stage_results.push(stage_result);
        self
    }

    /// Add a pipeline error
    pub fn add_pipeline_error(mut self, error: PipelineError) -> Self {
        self.pipeline_errors.push(error);
        self
    }

    /// Set execution statistics
    pub fn with_stats(mut self, stats: PipelineStats) -> Self {
        self.stats = stats;
        self
    }

    /// Set execution metrics
    pub fn with_metrics(mut self, metrics: PipelineMetrics) -> Self {
        self.metrics = metrics;
        self
    }

    /// Set additional metadata
    pub fn with_metadata(mut self, metadata: Value) -> Self {
        self.metadata = metadata;
        self
    }

    /// Get the overall success rate
    pub fn success_rate(&self) -> f64 {
        self.stats.success_rate()
    }

    /// Get the total number of validators executed
    pub fn total_validators_executed(&self) -> usize {
        self.stage_results.iter()
            .map(|stage| stage.validators_executed)
            .sum()
    }

    /// Get the total number of successful validations
    pub fn total_successful_validations(&self) -> usize {
        self.stage_results.iter()
            .map(|stage| stage.successful_validations)
            .sum()
    }

    /// Get the total number of failed validations
    pub fn total_failed_validations(&self) -> usize {
        self.stage_results.iter()
            .map(|stage| stage.failed_validations)
            .sum()
    }

    /// Get all validation errors from all stages
    pub fn get_all_validation_errors(&self) -> Vec<ValidationError> {
        let mut errors = Vec::new();
        
        for stage_result in &self.stage_results {
            for validator_result in &stage_result.validator_results {
                if !validator_result.result.is_success() {
                    errors.extend(validator_result.result.errors().clone());
                }
            }
        }
        
        errors
    }

    /// Get errors by category
    pub fn get_errors_by_category(&self, category: ErrorCategory) -> Vec<&PipelineError> {
        self.pipeline_errors.iter()
            .filter(|error| error.category == category)
            .collect()
    }

    /// Get errors by severity
    pub fn get_errors_by_severity(&self, severity: ErrorSeverity) -> Vec<&PipelineError> {
        self.pipeline_errors.iter()
            .filter(|error| error.severity == severity)
            .collect()
    }

    /// Check if the result has any critical errors
    pub fn has_critical_errors(&self) -> bool {
        self.pipeline_errors.iter()
            .any(|error| error.severity == ErrorSeverity::Critical)
    }

    /// Check if the result has any retryable errors
    pub fn has_retryable_errors(&self) -> bool {
        self.pipeline_errors.iter()
            .any(|error| error.retryable)
    }

    /// Get a summary of the pipeline execution
    pub fn get_summary(&self) -> PipelineSummary {
        PipelineSummary {
            execution_id: self.execution_id.clone(),
            pipeline_id: self.pipeline_id.clone(),
            pipeline_name: self.pipeline_name.clone(),
            success: self.success,
            execution_time: self.execution_time,
            total_stages: self.stage_results.len(),
            successful_stages: self.stage_results.iter().filter(|s| s.success).count(),
            failed_stages: self.stage_results.iter().filter(|s| !s.success).count(),
            total_validators: self.total_validators_executed(),
            successful_validators: self.total_successful_validations(),
            failed_validators: self.total_failed_validations(),
            success_rate: self.success_rate(),
            cache_hit_rate: self.stats.cache_hit_rate,
            has_critical_errors: self.has_critical_errors(),
            has_retryable_errors: self.has_retryable_errors(),
        }
    }

    /// Convert to a simplified result for external consumption
    pub fn to_simple_result(&self) -> SimplePipelineResult {
        SimplePipelineResult {
            success: self.success,
            execution_time: self.execution_time,
            total_validators: self.total_validators_executed(),
            successful_validators: self.total_successful_validations(),
            failed_validators: self.total_failed_validations(),
            success_rate: self.success_rate(),
            errors: self.pipeline_errors.iter()
                .map(|e| SimpleError {
                    code: e.code.clone(),
                    message: e.message.clone(),
                    severity: e.severity,
                })
                .collect(),
        }
    }
}

impl StageResult {
    /// Create a new stage result
    pub fn new(stage_id: String, stage_name: String) -> Self {
        Self {
            stage_id,
            stage_name,
            success: false,
            execution_time: Duration::ZERO,
            validators_executed: 0,
            successful_validations: 0,
            failed_validations: 0,
            validator_results: Vec::new(),
            stage_errors: Vec::new(),
            metrics: StageMetrics::default(),
            metadata: Value::Object(serde_json::Map::new()),
        }
    }

    /// Mark the stage execution as successful
    pub fn mark_success(mut self) -> Self {
        self.success = true;
        self
    }

    /// Add a validator result
    pub fn add_validator_result(mut self, validator_result: ValidatorResult) -> Self {
        self.validator_results.push(validator_result);
        self
    }

    /// Add a stage error
    pub fn add_stage_error(mut self, error: StageError) -> Self {
        self.stage_errors.push(error);
        self
    }

    /// Set execution time
    pub fn with_execution_time(mut self, execution_time: Duration) -> Self {
        self.execution_time = execution_time;
        self
    }

    /// Set metrics
    pub fn with_metrics(mut self, metrics: StageMetrics) -> Self {
        self.metrics = metrics;
        self
    }

    /// Get the success rate for this stage
    pub fn success_rate(&self) -> f64 {
        let total = self.successful_validations + self.failed_validations;
        if total == 0 {
            0.0
        } else {
            self.successful_validations as f64 / total as f64
        }
    }
}

impl ValidatorResult {
    /// Create a new validator result
    pub fn new(metadata: ValidatorMetadata, result: ValidationResult<()>) -> Self {
        let success = result.is_success();
        Self {
            metadata,
            success,
            execution_time: Duration::ZERO,
            result,
            cached: false,
            data: Value::Object(serde_json::Map::new()),
        }
    }

    /// Set execution time
    pub fn with_execution_time(mut self, execution_time: Duration) -> Self {
        self.execution_time = execution_time;
        self
    }

    /// Mark as cached
    pub fn mark_cached(mut self) -> Self {
        self.cached = true;
        self
    }

    /// Set additional data
    pub fn with_data(mut self, data: Value) -> Self {
        self.data = data;
        self
    }
}

impl PipelineError {
    /// Create a new pipeline error
    pub fn new(
        code: ErrorCode,
        message: String,
        severity: ErrorSeverity,
        category: ErrorCategory,
    ) -> Self {
        Self {
            code,
            message,
            severity,
            timestamp: Instant::now(),
            context: None,
            category,
            retryable: Self::is_retryable(&code),
        }
    }

    /// Create a new pipeline error with context
    pub fn with_context(
        code: ErrorCode,
        message: String,
        severity: ErrorSeverity,
        category: ErrorCategory,
        context: Value,
    ) -> Self {
        Self {
            code,
            message,
            severity,
            timestamp: Instant::now(),
            context: Some(context),
            category,
            retryable: Self::is_retryable(&code),
        }
    }

    /// Determine if an error code is retryable
    fn is_retryable(code: &ErrorCode) -> bool {
        matches!(
            code,
            ErrorCode::Timeout
                | ErrorCode::Transient
                | ErrorCode::TooManyRequests
                | ErrorCode::ServiceUnavailable
        )
    }

    /// Set the error as retryable
    pub fn mark_retryable(mut self) -> Self {
        self.retryable = true;
        self
    }

    /// Set the error as non-retryable
    pub fn mark_non_retryable(mut self) -> Self {
        self.retryable = false;
        self
    }
}

/// Summary of pipeline execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineSummary {
    pub execution_id: String,
    pub pipeline_id: String,
    pub pipeline_name: String,
    pub success: bool,
    pub execution_time: Duration,
    pub total_stages: usize,
    pub successful_stages: usize,
    pub failed_stages: usize,
    pub total_validators: usize,
    pub successful_validators: usize,
    pub failed_validators: usize,
    pub success_rate: f64,
    pub cache_hit_rate: f64,
    pub has_critical_errors: bool,
    pub has_retryable_errors: bool,
}

/// Simplified result for external consumption
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimplePipelineResult {
    pub success: bool,
    pub execution_time: Duration,
    pub total_validators: usize,
    pub successful_validators: usize,
    pub failed_validators: usize,
    pub success_rate: f64,
    pub errors: Vec<SimpleError>,
}

/// Simplified error for external consumption
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimpleError {
    pub code: ErrorCode,
    pub message: String,
    pub severity: ErrorSeverity,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::create_test_validator;

    #[test]
    fn test_pipeline_result_creation() {
        let start_time = Instant::now();
        let result = PipelineResult::new(
            "exec-1".to_string(),
            "pipeline-1".to_string(),
            "Test Pipeline".to_string(),
            start_time,
        );

        assert_eq!(result.execution_id, "exec-1");
        assert_eq!(result.pipeline_id, "pipeline-1");
        assert_eq!(result.pipeline_name, "Test Pipeline");
        assert!(!result.success);
    }

    #[test]
    fn test_pipeline_result_success() {
        let start_time = Instant::now();
        let result = PipelineResult::new(
            "exec-1".to_string(),
            "pipeline-1".to_string(),
            "Test Pipeline".to_string(),
            start_time,
        ).mark_success();

        assert!(result.success);
    }

    #[test]
    fn test_stage_result_creation() {
        let result = StageResult::new("stage-1".to_string(), "Test Stage".to_string());
        assert_eq!(result.stage_id, "stage-1");
        assert_eq!(result.stage_name, "Test Stage");
        assert!(!result.success);
    }

    #[test]
    fn test_validator_result_creation() {
        let metadata = ValidatorMetadata::new(
            "test".to_string(),
            "Test Validator".to_string(),
            crate::ValidatorCategory::Basic,
        );
        let validation_result = ValidationResult::success(());
        let result = ValidatorResult::new(metadata, validation_result);

        assert!(result.success);
        assert!(!result.cached);
    }

    #[test]
    fn test_pipeline_error_creation() {
        let error = PipelineError::new(
            ErrorCode::Timeout,
            "Execution timed out".to_string(),
            ErrorSeverity::Error,
            ErrorCategory::Timeout,
        );

        assert_eq!(error.code, ErrorCode::Timeout);
        assert_eq!(error.message, "Execution timed out");
        assert_eq!(error.severity, ErrorSeverity::Error);
        assert_eq!(error.category, ErrorCategory::Timeout);
        assert!(error.retryable);
    }

    #[test]
    fn test_pipeline_summary() {
        let start_time = Instant::now();
        let result = PipelineResult::new(
            "exec-1".to_string(),
            "pipeline-1".to_string(),
            "Test Pipeline".to_string(),
            start_time,
        ).mark_success();

        let summary = result.get_summary();
        assert_eq!(summary.execution_id, "exec-1");
        assert_eq!(summary.pipeline_id, "pipeline-1");
        assert_eq!(summary.pipeline_name, "Test Pipeline");
        assert!(summary.success);
    }
}


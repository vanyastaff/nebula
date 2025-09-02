//! Pipeline executor for running validation pipelines
//! 
//! This module provides the PipelineExecutor for executing validation pipelines
//! with comprehensive error handling, metrics collection, and result aggregation.

use super::{
    Pipeline, PipelineStage, StageExecutionResult, StageError, StageMetrics,
    PipelineConfig, RetryConfig, PipelineStats,
};
use crate::{
    ValidationContext, ValidationResult, ErrorCode, ErrorSeverity,
    ValidatorMetadata, ValidationComplexity,
};
use serde_json::Value;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{RwLock, Semaphore};
use tokio::time::timeout;
use tracing::{info, warn, error, instrument, span, Level};

/// Executor for running validation pipelines
/// 
/// The PipelineExecutor handles the execution of validation pipelines with
/// comprehensive error handling, timeout management, retry logic, and metrics collection.
#[derive(Debug)]
pub struct PipelineExecutor {
    /// Pipeline to execute
    pipeline: Arc<Pipeline>,
    /// Execution configuration
    config: ExecutionConfig,
    /// Metrics collector
    metrics: Arc<RwLock<PipelineMetrics>>,
    /// Execution statistics
    stats: Arc<RwLock<PipelineStats>>,
}

/// Configuration for pipeline execution
#[derive(Debug, Clone)]
pub struct ExecutionConfig {
    /// Maximum execution time for the entire pipeline
    pub timeout_ms: Option<u64>,
    /// Whether to stop on first stage failure
    pub fail_fast: bool,
    /// Whether to collect detailed metrics
    pub collect_metrics: bool,
    /// Whether to enable result caching
    pub enable_caching: bool,
    /// Maximum number of concurrent stages
    pub max_concurrent_stages: usize,
    /// Retry configuration
    pub retry_config: Option<RetryConfig>,
    /// Whether to enable tracing
    pub enable_tracing: bool,
}

/// Execution context for pipeline runs
#[derive(Debug, Clone)]
pub struct ExecutionContext {
    /// Unique execution ID
    pub execution_id: String,
    /// Start time of execution
    pub start_time: Instant,
    /// Validation context
    pub validation_context: ValidationContext,
    /// Additional metadata
    pub metadata: Value,
    /// Cancellation token
    pub cancelled: Arc<RwLock<bool>>,
}

/// Metrics for pipeline execution
#[derive(Debug, Clone, Default)]
pub struct PipelineMetrics {
    /// Total execution time
    pub total_execution_time: Duration,
    /// Stage execution times
    pub stage_times: Vec<Duration>,
    /// Memory usage during execution
    pub peak_memory_usage: u64,
    /// CPU usage during execution
    pub avg_cpu_usage: f64,
    /// Number of cache hits
    pub total_cache_hits: usize,
    /// Number of cache misses
    pub total_cache_misses: usize,
    /// Number of retries performed
    pub total_retries: usize,
    /// Number of timeouts
    pub timeouts: usize,
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            timeout_ms: Some(30_000), // 30 seconds
            fail_fast: true,
            collect_metrics: true,
            enable_caching: false,
            max_concurrent_stages: 1, // Sequential by default
            retry_config: None,
            enable_tracing: true,
        }
    }
}

impl PipelineExecutor {
    /// Create a new pipeline executor
    pub fn new(pipeline: Pipeline) -> Self {
        let config = ExecutionConfig::default();
        Self::with_config(pipeline, config)
    }

    /// Create a new pipeline executor with custom configuration
    pub fn with_config(pipeline: Pipeline, config: ExecutionConfig) -> Self {
        Self {
            pipeline: Arc::new(pipeline),
            config,
            metrics: Arc::new(RwLock::new(PipelineMetrics::default())),
            stats: Arc::new(RwLock::new(PipelineStats::default())),
        }
    }

    /// Execute the pipeline with the given value and context
    #[instrument(skip(self, value, context), fields(pipeline_id = %self.pipeline.id))]
    pub async fn execute(
        &self,
        value: &Value,
        context: &ValidationContext,
    ) -> PipelineExecutionResult {
        let execution_id = uuid::Uuid::new_v4().to_string();
        let start_time = Instant::now();
        
        let exec_context = ExecutionContext {
            execution_id: execution_id.clone(),
            start_time,
            validation_context: context.clone(),
            metadata: serde_json::Value::Object(serde_json::Map::new()),
            cancelled: Arc::new(RwLock::new(false)),
        };

        info!("Starting pipeline execution: {} (execution_id: {})", self.pipeline.name, execution_id);

        // Validate pipeline configuration
        let config_validation = self.pipeline.validate_config();
        if config_validation.is_failure() {
            return PipelineExecutionResult {
                execution_id,
                success: false,
                execution_time: start_time.elapsed(),
                pipeline_id: self.pipeline.id.clone(),
                stage_results: Vec::new(),
                pipeline_errors: vec![PipelineError {
                    code: ErrorCode::InvalidConfiguration,
                    message: "Pipeline configuration is invalid".to_string(),
                    severity: ErrorSeverity::Error,
                    timestamp: start_time,
                    context: Some(serde_json::to_value(&self.pipeline.config).unwrap_or_default()),
                }],
                stats: PipelineStats::default(),
                metrics: PipelineMetrics::default(),
            };
        }

        // Execute pipeline with timeout
        let result = if let Some(timeout_ms) = self.config.timeout_ms {
            match timeout(
                Duration::from_millis(timeout_ms),
                self.execute_pipeline(value, &exec_context)
            ).await {
                Ok(result) => result,
                Err(_) => {
                    warn!("Pipeline execution timed out after {}ms", timeout_ms);
                    PipelineExecutionResult {
                        execution_id,
                        success: false,
                        execution_time: start_time.elapsed(),
                        pipeline_id: self.pipeline.id.clone(),
                        stage_results: Vec::new(),
                        pipeline_errors: vec![PipelineError {
                            code: ErrorCode::Timeout,
                            message: format!("Pipeline execution timed out after {}ms", timeout_ms),
                            severity: ErrorSeverity::Error,
                            timestamp: start_time,
                            context: None,
                        }],
                        stats: PipelineStats::default(),
                        metrics: PipelineMetrics::default(),
                    }
                }
            }
        } else {
            self.execute_pipeline(value, &exec_context).await
        };

        let execution_time = start_time.elapsed();
        info!(
            "Pipeline execution completed: {} (success: {}, time: {:?})",
            self.pipeline.name, result.success, execution_time
        );

        result
    }

    /// Execute the pipeline stages
    async fn execute_pipeline(
        &self,
        value: &Value,
        exec_context: &ExecutionContext,
    ) -> PipelineExecutionResult {
        let mut stage_results = Vec::new();
        let mut pipeline_errors = Vec::new();
        let mut successful_stages = 0;
        let mut failed_stages = 0;
        let mut total_validators_executed = 0;
        let mut total_successful_validations = 0;
        let mut total_failed_validations = 0;
        let mut total_retries = 0;

        // Check if execution is cancelled
        if *exec_context.cancelled.read().await {
            return self.create_cancelled_result(exec_context);
        }

        // Execute stages based on configuration
        if self.config.max_concurrent_stages > 1 {
            // Parallel stage execution
            self.execute_stages_parallel(
                value,
                exec_context,
                &mut stage_results,
                &mut pipeline_errors,
                &mut successful_stages,
                &mut failed_stages,
                &mut total_validators_executed,
                &mut total_successful_validations,
                &mut total_failed_validations,
                &mut total_retries,
            ).await;
        } else {
            // Sequential stage execution
            self.execute_stages_sequential(
                value,
                exec_context,
                &mut stage_results,
                &mut pipeline_errors,
                &mut successful_stages,
                &mut failed_stages,
                &mut total_validators_executed,
                &mut total_successful_validations,
                &mut total_failed_validations,
                &mut total_retries,
            ).await;
        }

        // Calculate final statistics
        let stats = PipelineStats {
            total_time_ms: exec_context.start_time.elapsed().as_millis() as u64,
            stages_executed: stage_results.len(),
            validators_executed: total_validators_executed,
            successful_validations: total_successful_validations,
            failed_validations: total_failed_validations,
            retries_performed: total_retries,
            cache_hit_rate: self.calculate_cache_hit_rate(&stage_results),
        };

        // Calculate metrics
        let metrics = self.calculate_metrics(&stage_results, exec_context.start_time.elapsed());

        // Determine overall success
        let success = failed_stages == 0 || !self.config.fail_fast;

        PipelineExecutionResult {
            execution_id: exec_context.execution_id.clone(),
            success,
            execution_time: exec_context.start_time.elapsed(),
            pipeline_id: self.pipeline.id.clone(),
            stage_results,
            pipeline_errors,
            stats,
            metrics,
        }
    }

    /// Execute stages sequentially
    async fn execute_stages_sequential(
        &self,
        value: &Value,
        exec_context: &ExecutionContext,
        stage_results: &mut Vec<StageExecutionResult>,
        pipeline_errors: &mut Vec<PipelineError>,
        successful_stages: &mut usize,
        failed_stages: &mut usize,
        total_validators_executed: &mut usize,
        total_successful_validations: &mut usize,
        total_failed_validations: &mut usize,
        total_retries: &mut usize,
    ) {
        for stage in &self.pipeline.stages {
            // Check if execution is cancelled
            if *exec_context.cancelled.read().await {
                break;
            }

            let stage_result = self.execute_stage_with_retry(stage, value, &exec_context.validation_context).await;
            
            *total_validators_executed += stage_result.validators_executed;
            *total_successful_validations += stage_result.successful_validations;
            *total_failed_validations += stage_result.failed_validations;
            *total_retries += stage_result.metrics.retries;

            if stage_result.success {
                *successful_stages += 1;
            } else {
                *failed_stages += 1;
                if self.config.fail_fast {
                    break;
                }
            }

            stage_results.push(stage_result);
        }
    }

    /// Execute stages in parallel
    async fn execute_stages_parallel(
        &self,
        value: &Value,
        exec_context: &ExecutionContext,
        stage_results: &mut Vec<StageExecutionResult>,
        pipeline_errors: &mut Vec<PipelineError>,
        successful_stages: &mut usize,
        failed_stages: &mut usize,
        total_validators_executed: &mut usize,
        total_successful_validations: &mut usize,
        total_failed_validations: &mut usize,
        total_retries: &mut usize,
    ) {
        let semaphore = Arc::new(Semaphore::new(self.config.max_concurrent_stages));
        let mut tasks = Vec::new();

        for stage in &self.pipeline.stages {
            let permit = semaphore.clone().acquire_owned().await.unwrap();
            let stage = stage.clone();
            let value = value.clone();
            let context = exec_context.validation_context.clone();
            let cancelled = exec_context.cancelled.clone();

            let task = tokio::spawn(async move {
                let _permit = permit;
                
                // Check if execution is cancelled
                if *cancelled.read().await {
                    return None;
                }

                let result = Self::execute_stage_static(&stage, &value, &context).await;
                Some(result)
            });

            tasks.push(task);
        }

        // Collect results
        for task in tasks {
            if let Ok(Some(stage_result)) = task.await {
                *total_validators_executed += stage_result.validators_executed;
                *total_successful_validations += stage_result.successful_validations;
                *total_failed_validations += stage_result.failed_validations;
                *total_retries += stage_result.metrics.retries;

                if stage_result.success {
                    *successful_stages += 1;
                } else {
                    *failed_stages += 1;
                }

                stage_results.push(stage_result);
            }
        }
    }

    /// Execute a stage with retry logic
    async fn execute_stage_with_retry(
        &self,
        stage: &PipelineStage,
        value: &Value,
        context: &ValidationContext,
    ) -> StageExecutionResult {
        let mut last_result = stage.execute(value, context).await;

        if let Some(retry_config) = &self.config.retry_config {
            let mut attempts = 1;
            let mut delay = Duration::from_millis(retry_config.initial_delay_ms);

            while !last_result.success && attempts < retry_config.max_attempts {
                tokio::time::sleep(delay).await;
                
                last_result = stage.execute(value, context).await;
                attempts += 1;
                
                delay = Duration::from_millis(
                    (delay.as_millis() as f64 * retry_config.backoff_multiplier) as u64
                        .min(retry_config.max_delay_ms)
                );
            }
        }

        last_result
    }

    /// Static method to execute a stage (for use in async tasks)
    async fn execute_stage_static(
        stage: &PipelineStage,
        value: &Value,
        context: &ValidationContext,
    ) -> StageExecutionResult {
        stage.execute(value, context).await
    }

    /// Create a result for cancelled execution
    fn create_cancelled_result(&self, exec_context: &ExecutionContext) -> PipelineExecutionResult {
        PipelineExecutionResult {
            execution_id: exec_context.execution_id.clone(),
            success: false,
            execution_time: exec_context.start_time.elapsed(),
            pipeline_id: self.pipeline.id.clone(),
            stage_results: Vec::new(),
            pipeline_errors: vec![PipelineError {
                code: ErrorCode::Cancelled,
                message: "Pipeline execution was cancelled".to_string(),
                severity: ErrorSeverity::Warning,
                timestamp: exec_context.start_time,
                context: None,
            }],
            stats: PipelineStats::default(),
            metrics: PipelineMetrics::default(),
        }
    }

    /// Calculate cache hit rate
    fn calculate_cache_hit_rate(&self, stage_results: &[StageExecutionResult]) -> f64 {
        let total_hits: usize = stage_results.iter()
            .map(|result| result.metrics.cache_hits)
            .sum();
        let total_misses: usize = stage_results.iter()
            .map(|result| result.metrics.cache_misses)
            .sum();

        let total = total_hits + total_misses;
        if total == 0 {
            0.0
        } else {
            total_hits as f64 / total as f64
        }
    }

    /// Calculate pipeline metrics
    fn calculate_metrics(
        &self,
        stage_results: &[StageExecutionResult],
        total_time: Duration,
    ) -> PipelineMetrics {
        let stage_times: Vec<Duration> = stage_results
            .iter()
            .map(|result| result.execution_time)
            .collect();

        let total_cache_hits: usize = stage_results.iter()
            .map(|result| result.metrics.cache_hits)
            .sum();

        let total_cache_misses: usize = stage_results.iter()
            .map(|result| result.metrics.cache_misses)
            .sum();

        let total_retries: usize = stage_results.iter()
            .map(|result| result.metrics.retries)
            .sum();

        PipelineMetrics {
            total_execution_time: total_time,
            stage_times,
            peak_memory_usage: 0, // TODO: Implement memory tracking
            avg_cpu_usage: 0.0,   // TODO: Implement CPU tracking
            total_cache_hits,
            total_cache_misses,
            total_retries,
            timeouts: 0, // TODO: Track timeouts
        }
    }

    /// Get current execution statistics
    pub async fn get_stats(&self) -> PipelineStats {
        self.stats.read().await.clone()
    }

    /// Get current execution metrics
    pub async fn get_metrics(&self) -> PipelineMetrics {
        self.metrics.read().await.clone()
    }

    /// Cancel pipeline execution
    pub async fn cancel(&self, exec_context: &ExecutionContext) {
        *exec_context.cancelled.write().await = true;
    }
}

/// Result of pipeline execution
#[derive(Debug, Clone)]
pub struct PipelineExecutionResult {
    /// Unique execution ID
    pub execution_id: String,
    /// Whether the pipeline execution was successful
    pub success: bool,
    /// Total execution time
    pub execution_time: Duration,
    /// Pipeline ID
    pub pipeline_id: String,
    /// Results from each stage
    pub stage_results: Vec<StageExecutionResult>,
    /// Pipeline-level errors
    pub pipeline_errors: Vec<PipelineError>,
    /// Execution statistics
    pub stats: PipelineStats,
    /// Execution metrics
    pub metrics: PipelineMetrics,
}

/// Error that occurred during pipeline execution
#[derive(Debug, Clone)]
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::create_test_validator;

    #[tokio::test]
    async fn test_executor_creation() {
        let pipeline = Pipeline::new("test".to_string(), "Test Pipeline".to_string());
        let executor = PipelineExecutor::new(pipeline);
        assert_eq!(executor.pipeline.id, "test");
    }

    #[tokio::test]
    async fn test_executor_with_config() {
        let pipeline = Pipeline::new("test".to_string(), "Test Pipeline".to_string());
        let config = ExecutionConfig {
            timeout_ms: Some(5000),
            fail_fast: false,
            ..Default::default()
        };
        let executor = PipelineExecutor::with_config(pipeline, config);
        assert_eq!(executor.config.timeout_ms, Some(5000));
        assert!(!executor.config.fail_fast);
    }

    #[tokio::test]
    async fn test_execution_context() {
        let context = ValidationContext::default();
        let exec_context = ExecutionContext {
            execution_id: "test-exec".to_string(),
            start_time: Instant::now(),
            validation_context: context,
            metadata: serde_json::Value::Null,
            cancelled: Arc::new(RwLock::new(false)),
        };

        assert_eq!(exec_context.execution_id, "test-exec");
        assert!(!*exec_context.cancelled.read().await);
    }
}


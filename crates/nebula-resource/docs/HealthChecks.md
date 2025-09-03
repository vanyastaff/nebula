---

title: Health Checks
tags: [nebula-resource, how-to, health, monitoring]
status: stable
created: 2025-08-17
---

# Health Checks

Comprehensive guide to implementing advanced health checking for resources with pipeline stages, degraded state handling, and automatic recovery.

## Overview

Health checks ensure resources remain operational and performant. This system provides:

- Multi-stage health evaluation pipelines
- Degraded state handling with policies
- Automatic recovery mechanisms
- Predictive health monitoring
- Cascading failure detection

## Basic Health Check

### Step 1: Implement Simple Health Check

```rust
use nebula_resource::prelude::*;
use nebula_resource::health::*;

pub struct ApiClientInstance {
    client: HttpClient,
    endpoint: String,
    last_check: RwLock<DateTime<Utc>>,
}

#[async_trait]
impl ResourceInstance for ApiClientInstance {
    async fn health_check(&self) -> Result<HealthStatus, ResourceError> {
        // Simple connectivity check
        match self.client
            .get(&format!("{}/health", self.endpoint))
            .timeout(Duration::from_secs(5))
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {
                *self.last_check.write().await = Utc::now();
                Ok(HealthStatus::Healthy)
            }
            Ok(response) => {
                Ok(HealthStatus::Unhealthy {
                    reason: format!("API returned status: {}", response.status()),
                    recoverable: response.status().is_server_error(),
                })
            }
            Err(e) => {
                Ok(HealthStatus::Unhealthy {
                    reason: format!("Connection failed: {}", e),
                    recoverable: true,
                })
            }
        }
    }
}
```

### Step 2: Enhanced Health Status

```rust
#[derive(Debug, Clone)]
pub enum HealthStatus {
    /// Resource is fully operational
    Healthy,
    
    /// Resource is operational but with reduced capacity
    Degraded {
        reason: String,
        /// Performance impact factor (0.0 = no impact, 1.0 = fully degraded)
        performance_impact: f64,
    },
    
    /// Resource is not operational but may recover
    Unhealthy {
        reason: String,
        recoverable: bool,
    },
    
    /// Health status cannot be determined
    Unknown,
}

impl HealthStatus {
    pub fn is_healthy(&self) -> bool {
        matches!(self, HealthStatus::Healthy)
    }
    
    pub fn is_operational(&self) -> bool {
        matches!(self, HealthStatus::Healthy | HealthStatus::Degraded { .. })
    }
    
    pub fn performance_factor(&self) -> f64 {
        match self {
            HealthStatus::Healthy => 1.0,
            HealthStatus::Degraded { performance_impact, .. } => 1.0 - performance_impact,
            _ => 0.0,
        }
    }
}
```

## Advanced: Health Check Pipeline

### Multi-Stage Health Evaluation

```rust
use nebula_resource::health::pipeline::*;

/// Configure health check pipeline with multiple stages
pub struct HealthCheckPipeline {
    stages: Vec<Box<dyn HealthCheckStage>>,
    aggregator: Box<dyn HealthAggregator>,
    config: PipelineConfig,
}

#[derive(Debug, Clone)]
pub struct PipelineConfig {
    /// Stop on first critical failure
    pub fail_fast: bool,
    /// Maximum time for entire pipeline
    pub total_timeout: Duration,
    /// Parallel stage execution
    pub parallel_stages: bool,
    /// Cache results for this duration
    pub cache_duration: Option<Duration>,
}

/// Individual health check stage
#[async_trait]
pub trait HealthCheckStage: Send + Sync {
    /// Unique name for this stage
    fn name(&self) -> &str;
    
    /// Execute the health check
    async fn check(&self, resource: &dyn ResourceInstance) -> Result<StageResult, HealthError>;
    
    /// Whether failure in this stage should stop the pipeline
    fn is_critical(&self) -> bool { false }
    
    /// Weight for aggregation (higher = more important)
    fn weight(&self) -> f64 { 1.0 }
    
    /// Timeout for this specific stage
    fn timeout(&self) -> Duration { Duration::from_secs(10) }
}

#[derive(Debug, Clone)]
pub struct StageResult {
    pub stage_name: String,
    pub status: HealthStatus,
    pub duration: Duration,
    pub metrics: HashMap<String, f64>,
    pub metadata: Option<serde_json::Value>,
}
```

### Built-in Health Check Stages

```rust
/// Basic connectivity check
pub struct ConnectivityCheck {
    endpoint: String,
    timeout: Duration,
}

#[async_trait]
impl HealthCheckStage for ConnectivityCheck {
    fn name(&self) -> &str {
        "connectivity"
    }
    
    async fn check(&self, resource: &dyn ResourceInstance) -> Result<StageResult, HealthError> {
        let start = Instant::now();
        
        // Attempt connection
        let status = match self.test_connection(&self.endpoint).await {
            Ok(latency) => {
                if latency > Duration::from_secs(5) {
                    HealthStatus::Degraded {
                        reason: format!("High latency: {:?}", latency),
                        performance_impact: 0.3,
                    }
                } else {
                    HealthStatus::Healthy
                }
            }
            Err(e) => HealthStatus::Unhealthy {
                reason: e.to_string(),
                recoverable: true,
            },
        };
        
        Ok(StageResult {
            stage_name: self.name().to_string(),
            status,
            duration: start.elapsed(),
            metrics: hashmap! {
                "latency_ms" => start.elapsed().as_millis() as f64,
            },
            metadata: None,
        })
    }
    
    fn is_critical(&self) -> bool {
        true // Can't do anything without connectivity
    }
}

/// Performance metrics check
pub struct PerformanceCheck {
    latency_thresholds: LatencyThresholds,
    throughput_baseline: f64,
}

#[derive(Clone)]
pub struct LatencyThresholds {
    pub healthy_ms: u64,      // < this = healthy
    pub degraded_ms: u64,     // < this = degraded
    pub unhealthy_ms: u64,    // >= this = unhealthy
}

#[async_trait]
impl HealthCheckStage for PerformanceCheck {
    fn name(&self) -> &str {
        "performance"
    }
    
    async fn check(&self, resource: &dyn ResourceInstance) -> Result<StageResult, HealthError> {
        let metrics = resource.metrics();
        let avg_latency = metrics.average_latency_ms;
        
        let status = if avg_latency < self.latency_thresholds.healthy_ms as f64 {
            HealthStatus::Healthy
        } else if avg_latency < self.latency_thresholds.degraded_ms as f64 {
            HealthStatus::Degraded {
                reason: format!("Latency {}ms above healthy threshold", avg_latency),
                performance_impact: 0.2,
            }
        } else {
            HealthStatus::Degraded {
                reason: format!("High latency: {}ms", avg_latency),
                performance_impact: 0.6,
            }
        };
        
        Ok(StageResult {
            stage_name: self.name().to_string(),
            status,
            duration: Duration::from_millis(10),
            metrics: hashmap! {
                "avg_latency_ms" => avg_latency,
                "throughput" => metrics.requests_per_second,
            },
            metadata: None,
        })
    }
}

/// Resource utilization check
pub struct ResourceUtilizationCheck {
    memory_threshold: f64,
    cpu_threshold: f64,
    connection_threshold: f64,
}

/// Dependency health check
pub struct DependencyCheck {
    dependencies: Vec<String>,
    cascade_detection: bool,
}

#[async_trait]
impl HealthCheckStage for DependencyCheck {
    fn name(&self) -> &str {
        "dependencies"
    }
    
    async fn check(&self, resource: &dyn ResourceInstance) -> Result<StageResult, HealthError> {
        let mut unhealthy_deps = Vec::new();
        let mut degraded_deps = Vec::new();
        
        for dep_id in &self.dependencies {
            if let Some(dep_health) = self.get_dependency_health(dep_id).await? {
                match dep_health {
                    HealthStatus::Unhealthy { .. } => unhealthy_deps.push(dep_id.clone()),
                    HealthStatus::Degraded { .. } => degraded_deps.push(dep_id.clone()),
                    _ => {}
                }
            }
        }
        
        let status = if !unhealthy_deps.is_empty() {
            HealthStatus::Unhealthy {
                reason: format!("Dependencies unhealthy: {:?}", unhealthy_deps),
                recoverable: true,
            }
        } else if !degraded_deps.is_empty() {
            HealthStatus::Degraded {
                reason: format!("Dependencies degraded: {:?}", degraded_deps),
                performance_impact: 0.3 * (degraded_deps.len() as f64 / self.dependencies.len() as f64),
            }
        } else {
            HealthStatus::Healthy
        };
        
        Ok(StageResult {
            stage_name: self.name().to_string(),
            status,
            duration: Duration::from_millis(50),
            metrics: hashmap! {
                "healthy_deps" => (self.dependencies.len() - unhealthy_deps.len() - degraded_deps.len()) as f64,
                "degraded_deps" => degraded_deps.len() as f64,
                "unhealthy_deps" => unhealthy_deps.len() as f64,
            },
            metadata: Some(json!({
                "unhealthy": unhealthy_deps,
                "degraded": degraded_deps,
            })),
        })
    }
}
```

### Health Aggregation

```rust
/// Aggregate multiple stage results into overall health
pub trait HealthAggregator: Send + Sync {
    fn aggregate(&self, results: &[StageResult]) -> HealthStatus;
}

/// Weighted average aggregator
pub struct WeightedAggregator {
    stage_weights: HashMap<String, f64>,
}

impl HealthAggregator for WeightedAggregator {
    fn aggregate(&self, results: &[StageResult]) -> HealthStatus {
        let mut total_weight = 0.0;
        let mut health_score = 0.0;
        let mut reasons = Vec::new();
        
        for result in results {
            let weight = self.stage_weights
                .get(&result.stage_name)
                .copied()
                .unwrap_or(1.0);
            
            let score = match &result.status {
                HealthStatus::Healthy => 1.0,
                HealthStatus::Degraded { performance_impact, reason } => {
                    reasons.push(reason.clone());
                    1.0 - performance_impact
                }
                HealthStatus::Unhealthy { reason, .. } => {
                    reasons.push(reason.clone());
                    0.0
                }
                HealthStatus::Unknown => 0.5,
            };
            
            health_score += score * weight;
            total_weight += weight;
        }
        
        let final_score = if total_weight > 0.0 {
            health_score / total_weight
        } else {
            0.0
        };
        
        if final_score >= 0.9 {
            HealthStatus::Healthy
        } else if final_score >= 0.5 {
            HealthStatus::Degraded {
                reason: reasons.join("; "),
                performance_impact: 1.0 - final_score,
            }
        } else {
            HealthStatus::Unhealthy {
                reason: reasons.join("; "),
                recoverable: final_score > 0.0,
            }
        }
    }
}

/// Worst-case aggregator (most conservative)
pub struct WorstCaseAggregator;

impl HealthAggregator for WorstCaseAggregator {
    fn aggregate(&self, results: &[StageResult]) -> HealthStatus {
        results.iter()
            .map(|r| &r.status)
            .min_by_key(|status| match status {
                HealthStatus::Unhealthy { .. } => 0,
                HealthStatus::Unknown => 1,
                HealthStatus::Degraded { .. } => 2,
                HealthStatus::Healthy => 3,
            })
            .cloned()
            .unwrap_or(HealthStatus::Unknown)
    }
}
```

## Degraded State Handling

```rust
/// Policies for handling degraded resources
pub struct DegradedStateHandler {
    policies: HashMap<String, DegradedPolicy>,
    fallback_policy: DegradedPolicy,
}

#[derive(Clone)]
pub struct DegradedPolicy {
    /// Multiply timeouts by this factor
    pub timeout_multiplier: f64,
    
    /// Reduce rate limits by this factor
    pub rate_limit_reduction: f64,
    
    /// Enable circuit breaker with adjusted sensitivity
    pub circuit_breaker_config: Option<CircuitBreakerConfig>,
    
    /// Fallback strategies
    pub fallback_strategies: Vec<FallbackStrategy>,
    
    /// Auto-recovery settings
    pub recovery: RecoveryConfig,
}

#[derive(Clone)]
pub enum FallbackStrategy {
    /// Use cached responses if available
    UseCache { max_age: Duration },
    
    /// Switch to backup resource
    UseBackup { resource_id: String },
    
    /// Reduce functionality
    ReduceFeatures { disabled_features: Vec<String> },
    
    /// Return default/stub responses
    UseDefaults,
    
    /// Queue requests for later processing
    QueueForLater { max_queue_size: usize },
}

#[derive(Clone)]
pub struct RecoveryConfig {
    /// How often to retry health checks
    pub check_interval: Duration,
    
    /// Number of consecutive healthy checks before full recovery
    pub healthy_checks_required: u32,
    
    /// Gradual recovery (slowly increase capacity)
    pub gradual_recovery: bool,
    
    /// Recovery rate (for gradual recovery)
    pub recovery_rate: f64,
}

impl DegradedStateHandler {
    pub async fn apply_policy(
        &self,
        resource: &dyn ResourceInstance,
        health_status: &HealthStatus,
    ) -> Result<DegradedMitigation, HealthError> {
        let policy = self.policies
            .get(resource.resource_type())
            .unwrap_or(&self.fallback_policy);
        
        let mut mitigation = DegradedMitigation::default();
        
        if let HealthStatus::Degraded { performance_impact, .. } = health_status {
            // Adjust timeouts
            mitigation.timeout_adjustment = policy.timeout_multiplier;
            
            // Reduce rate limits
            mitigation.rate_limit_factor = 1.0 - (policy.rate_limit_reduction * performance_impact);
            
            // Apply fallback strategies
            for strategy in &policy.fallback_strategies {
                match strategy {
                    FallbackStrategy::UseCache { max_age } => {
                        mitigation.enable_cache = true;
                        mitigation.cache_max_age = Some(*max_age);
                    }
                    FallbackStrategy::UseBackup { resource_id } => {
                        mitigation.backup_resource = Some(resource_id.clone());
                    }
                    FallbackStrategy::ReduceFeatures { disabled_features } => {
                        mitigation.disabled_features.extend(disabled_features.clone());
                    }
                    _ => {}
                }
            }
            
            // Configure circuit breaker
            if let Some(cb_config) = &policy.circuit_breaker_config {
                mitigation.circuit_breaker = Some(cb_config.clone());
            }
        }
        
        Ok(mitigation)
    }
}
```

## Automatic Recovery

```rust
/// Automatic recovery system for unhealthy resources
pub struct RecoveryEngine {
    strategies: Vec<Box<dyn RecoveryStrategy>>,
    recovery_state: Arc<RwLock<HashMap<ResourceInstanceId, RecoveryState>>>,
}

#[derive(Clone)]
pub struct RecoveryState {
    pub attempts: u32,
    pub last_attempt: DateTime<Utc>,
    pub strategy_index: usize,
    pub consecutive_healthy: u32,
}

#[async_trait]
pub trait RecoveryStrategy: Send + Sync {
    fn name(&self) -> &str;
    
    /// Check if this strategy can handle the issue
    async fn can_recover(&self, health_status: &HealthStatus) -> bool;
    
    /// Attempt recovery
    async fn attempt_recovery(
        &self,
        resource: &mut dyn ResourceInstance,
        context: &RecoveryContext,
    ) -> Result<RecoveryResult, RecoveryError>;
    
    /// Maximum attempts for this strategy
    fn max_attempts(&self) -> u32 { 3 }
    
    /// Delay between attempts
    fn retry_delay(&self) -> Duration { Duration::from_secs(30) }
}

#[derive(Debug)]
pub enum RecoveryResult {
    /// Recovery successful
    Recovered,
    
    /// Partial recovery achieved
    PartiallyRecovered { limitations: Vec<String> },
    
    /// Recovery failed, try next strategy
    Failed { should_retry: bool },
    
    /// Resource needs replacement
    RequiresReplacement,
}

/// Simple restart strategy
pub struct RestartStrategy;

#[async_trait]
impl RecoveryStrategy for RestartStrategy {
    fn name(&self) -> &str {
        "restart"
    }
    
    async fn can_recover(&self, health_status: &HealthStatus) -> bool {
        matches!(health_status, HealthStatus::Unhealthy { recoverable: true, .. })
    }
    
    async fn attempt_recovery(
        &self,
        resource: &mut dyn ResourceInstance,
        context: &RecoveryContext,
    ) -> Result<RecoveryResult, RecoveryError> {
        context.log_info("Attempting resource restart");
        
        // Clean up current state
        resource.cleanup().await?;
        
        // Reinitialize
        resource.initialize().await?;
        
        // Check health
        match resource.health_check().await? {
            HealthStatus::Healthy => Ok(RecoveryResult::Recovered),
            HealthStatus::Degraded { .. } => {
                Ok(RecoveryResult::PartiallyRecovered {
                    limitations: vec!["Running in degraded mode after restart".into()],
                })
            }
            _ => Ok(RecoveryResult::Failed { should_retry: true }),
        }
    }
}

/// Connection reset strategy
pub struct ConnectionResetStrategy;

/// Credential refresh strategy
pub struct CredentialRefreshStrategy;

/// Failover strategy
pub struct FailoverStrategy {
    backup_resources: Vec<String>,
}
```

## Predictive Health Monitoring

```rust
/// Predict health issues before they occur
pub struct PredictiveHealthMonitor {
    model: Arc<HealthPredictionModel>,
    history: Arc<RwLock<HealthHistory>>,
    alert_threshold: f64,
}

impl PredictiveHealthMonitor {
    pub async fn analyze(&self, resource: &dyn ResourceInstance) -> PredictionReport {
        let history = self.history.read().await;
        let current_metrics = resource.metrics();
        
        // Analyze trends
        let latency_trend = history.calculate_trend("latency", Duration::from_hours(1));
        let error_trend = history.calculate_trend("errors", Duration::from_hours(1));
        let utilization_trend = history.calculate_trend("utilization", Duration::from_hours(1));
        
        // Predict future health
        let prediction = self.model.predict(PredictionInput {
            current_metrics: &current_metrics,
            latency_trend,
            error_trend,
            utilization_trend,
            time_of_day: Utc::now().hour(),
            day_of_week: Utc::now().weekday(),
        }).await;
        
        PredictionReport {
            health_score: prediction.health_score,
            risk_level: prediction.risk_level,
            predicted_issues: prediction.issues,
            recommended_actions: prediction.recommendations,
            confidence: prediction.confidence,
        }
    }
}
```

## Using Health Checks

```rust
#[derive(Resource)]
#[resource(
    id = "monitored_api",
    health_pipeline = "comprehensive"
)]
pub struct MonitoredApiResource;

impl MonitoredApiResource {
    pub fn create_health_pipeline() -> HealthCheckPipeline {
        HealthCheckPipeline::builder()
            .add_stage(ConnectivityCheck::new())
            .add_stage(PerformanceCheck::new())
            .add_stage(ResourceUtilizationCheck::new())
            .add_stage(DependencyCheck::new())
            .aggregator(WeightedAggregator::new())
            .config(PipelineConfig {
                fail_fast: true,
                total_timeout: Duration::from_secs(30),
                parallel_stages: true,
                cache_duration: Some(Duration::from_secs(10)),
            })
            .build()
    }
}

// In action
impl ProcessAction for HealthAwareAction {
    async fn execute(&self, input: Input, context: &ExecutionContext) -> Result<Output> {
        let resource = context.get_resource::<MonitoredApiResource>().await?;
        
        // Check health before critical operation
        let health = resource.check_health_cached().await?;
        
        if !health.is_operational() {
            return Err(ActionError::ResourceUnavailable);
        }
        
        // Adjust behavior based on health
        let timeout = if let HealthStatus::Degraded { .. } = health {
            Duration::from_secs(60) // Longer timeout for degraded resource
        } else {
            Duration::from_secs(10)
        };
        
        // Use resource with adjusted parameters
        resource.execute_with_timeout(input, timeout).await
    }
}
```

## Best Practices

1. **Layer health checks** - Use pipeline with multiple stages
2. **Cache results** - Avoid excessive health checking
3. **Handle degraded state** - Don't just fail/succeed
4. **Implement recovery** - Automatic recovery where possible
5. **Monitor trends** - Predictive monitoring prevents issues
6. **Set appropriate timeouts** - Don't let health checks hang
7. **Consider dependencies** - Check dependent resources
8. **Use circuit breakers** - Prevent cascade failures
9. **Log health transitions** - Track state changes
10. **Test failure scenarios** - Ensure graceful degradation
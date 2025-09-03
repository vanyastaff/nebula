---
title: Lifecycle Hooks
tags: [nebula, nebula-resource, docs]
status: draft
created: 2025-08-17
---

# Resource Hooks Reference

## Overview

The hooks system in nebula-resource provides a powerful mechanism for intercepting and extending resource behavior at various lifecycle points. Hooks enable cross-cutting concerns like logging, monitoring, validation, and transformation without modifying resource implementations.

## Core Hook Types

### `ResourceHook`

Base trait for all resource hooks.

```rust
#[async_trait]
pub trait ResourceHook: Send + Sync {
    /// Hook name for identification
    fn name(&self) -> &str;
    
    /// Hook priority (lower executes first)
    fn priority(&self) -> i32 {
        0
    }
    
    /// Check if hook should run for resource
    fn should_run(&self, resource: &dyn Resource) -> bool {
        true
    }
    
    /// Hook-specific error handling
    async fn on_error(&self, error: &Error, context: &HookContext) {
        // Default: log error
        error!("Hook {} error: {}", self.name(), error);
    }
}
```

### `LifecycleHook`

Hooks for resource lifecycle events.

```rust
#[async_trait]
pub trait LifecycleHook: ResourceHook {
    /// Before resource initialization
    async fn pre_initialize(&self, resource: &dyn Resource) -> Result<()> {
        Ok(())
    }
    
    /// After resource initialization
    async fn post_initialize(&self, resource: &dyn Resource, result: &Result<()>) -> Result<()> {
        Ok(())
    }
    
    /// Before resource cleanup
    async fn pre_cleanup(&self, resource: &dyn Resource) -> Result<()> {
        Ok(())
    }
    
    /// After resource cleanup
    async fn post_cleanup(&self, resource: &dyn Resource, result: &Result<()>) -> Result<()> {
        Ok(())
    }
    
    /// On state transition
    async fn on_state_transition(
        &self,
        resource: &dyn Resource,
        from: &LifecycleState,
        to: &LifecycleState,
    ) -> Result<()> {
        Ok(())
    }
    
    /// On resource creation
    async fn on_create(&self, resource: &dyn Resource) -> Result<()> {
        Ok(())
    }
    
    /// On resource destruction
    async fn on_destroy(&self, resource: &dyn Resource) -> Result<()> {
        Ok(())
    }
}
```

### `PoolHook`

Hooks for pool operations.

```rust
#[async_trait]
pub trait PoolHook: ResourceHook {
    /// Before acquiring from pool
    async fn pre_acquire(&self, pool_id: &ResourceId) -> Result<()> {
        Ok(())
    }
    
    /// After acquiring from pool
    async fn post_acquire(&self, resource: &dyn Resource, duration: Duration) -> Result<()> {
        Ok(())
    }
    
    /// Before releasing to pool
    async fn pre_release(&self, resource: &dyn Resource) -> Result<()> {
        Ok(())
    }
    
    /// After releasing to pool
    async fn post_release(&self, resource: &dyn Resource) -> Result<()> {
        Ok(())
    }
    
    /// On pool expansion
    async fn on_pool_expand(&self, pool_id: &ResourceId, new_size: usize) -> Result<()> {
        Ok(())
    }
    
    /// On pool shrink
    async fn on_pool_shrink(&self, pool_id: &ResourceId, new_size: usize) -> Result<()> {
        Ok(())
    }
    
    /// On resource eviction from pool
    async fn on_evict(&self, resource: &dyn Resource, reason: EvictionReason) -> Result<()> {
        Ok(())
    }
    
    /// On pool creation
    async fn on_pool_create(&self, config: &PoolConfig) -> Result<()> {
        Ok(())
    }
    
    /// On pool destruction
    async fn on_pool_destroy(&self, pool_id: &ResourceId) -> Result<()> {
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum EvictionReason {
    Idle(Duration),
    Invalid,
    PoolShrink,
    Manual,
    HealthCheckFailed,
}
```

### `HealthHook`

Hooks for health check operations.

```rust
#[async_trait]
pub trait HealthHook: ResourceHook {
    /// Before health check
    async fn pre_health_check(&self, resource: &dyn Resource) -> Result<()> {
        Ok(())
    }
    
    /// After health check
    async fn post_health_check(
        &self,
        resource: &dyn Resource,
        status: &HealthStatus,
        duration: Duration,
    ) -> Result<()> {
        Ok(())
    }
    
    /// On health status change
    async fn on_health_change(
        &self,
        resource: &dyn Resource,
        old_status: &HealthStatus,
        new_status: &HealthStatus,
    ) -> Result<()> {
        Ok(())
    }
    
    /// On health check failure
    async fn on_health_failure(
        &self,
        resource: &dyn Resource,
        error: &Error,
        consecutive_failures: u32,
    ) -> Result<()> {
        Ok(())
    }
    
    /// Custom health check logic
    async fn custom_health_check(&self, resource: &dyn Resource) -> Option<HealthStatus> {
        None
    }
}
```

### `MetricsHook`

Hooks for metrics collection.

```rust
#[async_trait]
pub trait MetricsHook: ResourceHook {
    /// Collect metrics for resource
    async fn collect_metrics(&self, resource: &dyn Resource) -> ResourceMetrics;
    
    /// On metric recorded
    async fn on_metric_recorded(
        &self,
        resource: &dyn Resource,
        metric_name: &str,
        value: &MetricValue,
    ) -> Result<()> {
        Ok(())
    }
    
    /// Transform metrics before export
    async fn transform_metrics(&self, metrics: &mut ResourceMetrics) -> Result<()> {
        Ok(())
    }
    
    /// Filter metrics for export
    fn should_export_metric(&self, metric_name: &str) -> bool {
        true
    }
    
    /// Aggregate metrics across resources
    async fn aggregate_metrics(&self, metrics: Vec<ResourceMetrics>) -> AggregatedMetrics {
        AggregatedMetrics::from(metrics)
    }
}
```

## Specialized Hooks

### `ValidationHook`

For resource validation.

```rust
#[async_trait]
pub trait ValidationHook: ResourceHook {
    /// Validate resource configuration
    async fn validate_config(&self, config: &Value) -> Result<()>;
    
    /// Validate resource state
    async fn validate_state(&self, resource: &dyn Resource) -> Result<()>;
    
    /// Validate before operation
    async fn validate_operation(
        &self,
        resource: &dyn Resource,
        operation: &Operation,
    ) -> Result<()>;
    
    /// Custom validation rules
    fn validation_rules(&self) -> Vec<ValidationRule> {
        vec![]
    }
}

pub struct ValidationRule {
    pub field: String,
    pub constraint: ValidationConstraint,
    pub message: String,
}
```

### `TransformHook`

For data transformation.

```rust
#[async_trait]
pub trait TransformHook: ResourceHook {
    /// Transform input data
    async fn transform_input<T>(&self, data: &mut T) -> Result<()>
    where
        T: Serialize + DeserializeOwned;
    
    /// Transform output data
    async fn transform_output<T>(&self, data: &mut T) -> Result<()>
    where
        T: Serialize + DeserializeOwned;
    
    /// Transform error responses
    async fn transform_error(&self, error: Error) -> Error {
        error
    }
    
    /// Transform resource configuration
    async fn transform_config(&self, config: &mut Value) -> Result<()> {
        Ok(())
    }
}
```

### `SecurityHook`

For security operations.

```rust
#[async_trait]
pub trait SecurityHook: ResourceHook {
    /// Authenticate access
    async fn authenticate(&self, credentials: &Credentials) -> Result<AuthToken>;
    
    /// Authorize operation
    async fn authorize(
        &self,
        token: &AuthToken,
        resource: &dyn Resource,
        operation: &Operation,
    ) -> Result<bool>;
    
    /// Audit resource access
    async fn audit(
        &self,
        resource: &dyn Resource,
        operation: &Operation,
        result: &Result<()>,
        context: &SecurityContext,
    ) -> Result<()>;
    
    /// Encrypt sensitive data
    async fn encrypt_data(&self, data: &[u8]) -> Result<Vec<u8>>;
    
    /// Decrypt sensitive data
    async fn decrypt_data(&self, data: &[u8]) -> Result<Vec<u8>>;
    
    /// Sanitize output
    async fn sanitize_output<T>(&self, data: &mut T) -> Result<()>
    where
        T: Serialize + DeserializeOwned;
}
```

### `TracingHook`

For distributed tracing.

```rust
#[async_trait]
pub trait TracingHook: ResourceHook {
    /// Start a trace span
    fn start_span(&self, name: &str, resource: &dyn Resource) -> Span;
    
    /// Add span attributes
    fn add_attributes(&self, span: &Span, attributes: HashMap<String, Value>);
    
    /// Record span event
    fn record_event(&self, span: &Span, event: &str, attributes: HashMap<String, Value>);
    
    /// Extract trace context
    fn extract_context(&self, headers: &HeaderMap) -> Option<TraceContext>;
    
    /// Inject trace context
    fn inject_context(&self, context: &TraceContext, headers: &mut HeaderMap);
    
    /// On span complete
    async fn on_span_complete(&self, span: Span, duration: Duration, status: SpanStatus);
}
```

## Hook Context

### `HookContext`

Context passed to hooks.

```rust
#[derive(Debug, Clone)]
pub struct HookContext {
    /// Current operation
    pub operation: Operation,
    
    /// Resource being operated on
    pub resource_id: ResourceId,
    
    /// Resource type
    pub resource_type: String,
    
    /// Execution context
    pub execution_context: ExecutionContext,
    
    /// Hook metadata
    pub metadata: HashMap<String, Value>,
    
    /// Parent span for tracing
    pub span: Option<Span>,
    
    /// Start time of operation
    pub start_time: Instant,
}

#[derive(Debug, Clone)]
pub enum Operation {
    Initialize,
    Cleanup,
    HealthCheck,
    Acquire,
    Release,
    Read,
    Write,
    Custom(String),
}
```

## Hook Registration

### `HookRegistry`

Registry for managing hooks.

```rust
pub struct HookRegistry {
    lifecycle_hooks: Vec<Box<dyn LifecycleHook>>,
    pool_hooks: Vec<Box<dyn PoolHook>>,
    health_hooks: Vec<Box<dyn HealthHook>>,
    metrics_hooks: Vec<Box<dyn MetricsHook>>,
    validation_hooks: Vec<Box<dyn ValidationHook>>,
    transform_hooks: Vec<Box<dyn TransformHook>>,
    security_hooks: Vec<Box<dyn SecurityHook>>,
    tracing_hooks: Vec<Box<dyn TracingHook>>,
    custom_hooks: HashMap<String, Vec<Box<dyn ResourceHook>>>,
}

impl HookRegistry {
    /// Register a lifecycle hook
    pub fn register_lifecycle<H: LifecycleHook + 'static>(&mut self, hook: H) {
        self.lifecycle_hooks.push(Box::new(hook));
        self.sort_hooks();
    }
    
    /// Register a pool hook
    pub fn register_pool<H: PoolHook + 'static>(&mut self, hook: H) {
        self.pool_hooks.push(Box::new(hook));
        self.sort_hooks();
    }
    
    /// Register multiple hooks at once
    pub fn register_many(&mut self, hooks: Vec<Box<dyn ResourceHook>>) {
        for hook in hooks {
            self.register_custom(hook);
        }
    }
    
    /// Unregister hook by name
    pub fn unregister(&mut self, name: &str) {
        self.lifecycle_hooks.retain(|h| h.name() != name);
        self.pool_hooks.retain(|h| h.name() != name);
        // ... other hook types
    }
    
    /// Get all hooks for a resource
    pub fn hooks_for_resource(&self, resource: &dyn Resource) -> HooksForResource {
        HooksForResource {
            lifecycle: self.lifecycle_hooks
                .iter()
                .filter(|h| h.should_run(resource))
                .collect(),
            pool: self.pool_hooks
                .iter()
                .filter(|h| h.should_run(resource))
                .collect(),
            // ... other types
        }
    }
    
    /// Sort hooks by priority
    fn sort_hooks(&mut self) {
        self.lifecycle_hooks.sort_by_key(|h| h.priority());
        self.pool_hooks.sort_by_key(|h| h.priority());
        // ... other types
    }
}
```

## Built-in Hooks

### `LoggingHook`

Logs all resource operations.

```rust
pub struct LoggingHook {
    level: LogLevel,
    include_metrics: bool,
    include_trace: bool,
}

#[async_trait]
impl LifecycleHook for LoggingHook {
    fn name(&self) -> &str {
        "logging"
    }
    
    async fn pre_initialize(&self, resource: &dyn Resource) -> Result<()> {
        info!("Initializing resource: {}", resource.id());
        Ok(())
    }
    
    async fn post_initialize(&self, resource: &dyn Resource, result: &Result<()>) -> Result<()> {
        match result {
            Ok(_) => info!("Resource {} initialized successfully", resource.id()),
            Err(e) => error!("Resource {} initialization failed: {}", resource.id(), e),
        }
        Ok(())
    }
    
    async fn on_state_transition(
        &self,
        resource: &dyn Resource,
        from: &LifecycleState,
        to: &LifecycleState,
    ) -> Result<()> {
        debug!(
            "Resource {} transitioning from {:?} to {:?}",
            resource.id(),
            from,
            to
        );
        Ok(())
    }
}
```

### `MetricsCollectorHook`

Collects metrics for all operations.

```rust
pub struct MetricsCollectorHook {
    registry: Arc<Registry>,
    operation_counter: IntCounterVec,
    operation_duration: HistogramVec,
    resource_gauge: GaugeVec,
}

#[async_trait]
impl MetricsHook for MetricsCollectorHook {
    fn name(&self) -> &str {
        "metrics_collector"
    }
    
    async fn collect_metrics(&self, resource: &dyn Resource) -> ResourceMetrics {
        let metrics = resource.metrics().await.unwrap_or_default();
        
        // Update Prometheus metrics
        self.resource_gauge
            .with_label_values(&[resource.resource_type()])
            .set(metrics.active_operations as f64);
        
        metrics
    }
    
    async fn on_metric_recorded(
        &self,
        resource: &dyn Resource,
        metric_name: &str,
        value: &MetricValue,
    ) -> Result<()> {
        self.operation_counter
            .with_label_values(&[resource.resource_type(), metric_name])
            .inc();
        
        if let MetricValue::Gauge(v) = value {
            self.resource_gauge
                .with_label_values(&[resource.resource_type(), metric_name])
                .set(*v);
        }
        
        Ok(())
    }
}
```

### `CircuitBreakerHook`

Implements circuit breaker pattern.

```rust
pub struct CircuitBreakerHook {
    states: Arc<RwLock<HashMap<ResourceId, CircuitState>>>,
    config: CircuitBreakerConfig,
}

#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    pub failure_threshold: u32,
    pub success_threshold: u32,
    pub timeout: Duration,
    pub half_open_max_calls: u32,
}

#[derive(Debug, Clone)]
enum CircuitState {
    Closed,
    Open { since: Instant },
    HalfOpen { successes: u32, failures: u32 },
}

#[async_trait]
impl HealthHook for CircuitBreakerHook {
    fn name(&self) -> &str {
        "circuit_breaker"
    }
    
    async fn on_health_failure(
        &self,
        resource: &dyn Resource,
        error: &Error,
        consecutive_failures: u32,
    ) -> Result<()> {
        let mut states = self.states.write().await;
        
        if consecutive_failures >= self.config.failure_threshold {
            states.insert(
                resource.id(),
                CircuitState::Open {
                    since: Instant::now(),
                },
            );
            warn!("Circuit breaker opened for resource {}", resource.id());
        }
        
        Ok(())
    }
    
    async fn pre_health_check(&self, resource: &dyn Resource) -> Result<()> {
        let states = self.states.read().await;
        
        if let Some(state) = states.get(&resource.id()) {
            match state {
                CircuitState::Open { since } => {
                    if since.elapsed() < self.config.timeout {
                        return Err(Error::CircuitBreakerOpen);
                    }
                    // Transition to half-open
                    drop(states);
                    let mut states = self.states.write().await;
                    states.insert(
                        resource.id(),
                        CircuitState::HalfOpen {
                            successes: 0,
                            failures: 0,
                        },
                    );
                }
                _ => {}
            }
        }
        
        Ok(())
    }
}
```

### `RateLimitHook`

Rate limiting for resource operations.

```rust
pub struct RateLimitHook {
    limiters: Arc<RwLock<HashMap<ResourceId, RateLimiter>>>,
    config: RateLimitConfig,
}

#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    pub requests_per_second: f64,
    pub burst_size: u32,
    pub wait_timeout: Duration,
}

#[async_trait]
impl ResourceHook for RateLimitHook {
    fn name(&self) -> &str {
        "rate_limit"
    }
}

#[async_trait]
impl LifecycleHook for RateLimitHook {
    async fn pre_initialize(&self, resource: &dyn Resource) -> Result<()> {
        let mut limiters = self.limiters.write().await;
        limiters.insert(
            resource.id(),
            RateLimiter::new(
                self.config.requests_per_second,
                self.config.burst_size,
            ),
        );
        Ok(())
    }
}
```

### `RetryHook`

Automatic retry logic.

```rust
pub struct RetryHook {
    config: RetryConfig,
    retry_predicates: Vec<Box<dyn Fn(&Error) -> bool + Send + Sync>>,
}

impl RetryHook {
    pub fn new(config: RetryConfig) -> Self {
        Self {
            config,
            retry_predicates: vec![
                Box::new(|e| e.is_retryable()),
                Box::new(|e| matches!(e, Error::ConnectionTimeout { .. })),
            ],
        }
    }
    
    pub fn with_predicate<F>(mut self, predicate: F) -> Self
    where
        F: Fn(&Error) -> bool + Send + Sync + 'static,
    {
        self.retry_predicates.push(Box::new(predicate));
        self
    }
    
    async fn should_retry(&self, error: &Error) -> bool {
        self.retry_predicates.iter().any(|p| p(error))
    }
}
```

## Hook Composition

### Chaining Hooks

```rust
pub struct HookChain {
    hooks: Vec<Box<dyn ResourceHook>>,
}

impl HookChain {
    pub fn new() -> Self {
        Self { hooks: Vec::new() }
    }
    
    pub fn add<H: ResourceHook + 'static>(mut self, hook: H) -> Self {
        self.hooks.push(Box::new(hook));
        self
    }
    
    pub async fn execute<F, T>(&self, operation: F) -> Result<T>
    where
        F: FnOnce() -> Future<Output = Result<T>>,
    {
        // Pre-hooks
        for hook in &self.hooks {
            // Execute pre-operation hooks
        }
        
        // Execute operation
        let result = operation().await;
        
        // Post-hooks
        for hook in self.hooks.iter().rev() {
            // Execute post-operation hooks
        }
        
        result
    }
}
```

### Conditional Hooks

```rust
pub struct ConditionalHook<H: ResourceHook> {
    inner: H,
    condition: Box<dyn Fn(&dyn Resource) -> bool + Send + Sync>,
}

impl<H: ResourceHook> ConditionalHook<H> {
    pub fn new<F>(hook: H, condition: F) -> Self
    where
        F: Fn(&dyn Resource) -> bool + Send + Sync + 'static,
    {
        Self {
            inner: hook,
            condition: Box::new(condition),
        }
    }
}

#[async_trait]
impl<H: ResourceHook> ResourceHook for ConditionalHook<H> {
    fn name(&self) -> &str {
        self.inner.name()
    }
    
    fn should_run(&self, resource: &dyn Resource) -> bool {
        (self.condition)(resource) && self.inner.should_run(resource)
    }
}
```

## Custom Hook Implementation

### Example: Custom Monitoring Hook

```rust
pub struct CustomMonitoringHook {
    name: String,
    monitor: Arc<dyn Monitor>,
    alert_threshold: f64,
}

#[async_trait]
impl ResourceHook for CustomMonitoringHook {
    fn name(&self) -> &str {
        &self.name
    }
    
    fn priority(&self) -> i32 {
        10 // Higher priority
    }
}

#[async_trait]
impl MetricsHook for CustomMonitoringHook {
    async fn collect_metrics(&self, resource: &dyn Resource) -> ResourceMetrics {
        let metrics = resource.metrics().await.unwrap_or_default();
        
        // Send to monitoring system
        self.monitor.send_metrics(&metrics).await;
        
        // Check thresholds
        if metrics.error_rate() > self.alert_threshold {
            self.monitor.send_alert(Alert {
                severity: AlertSeverity::High,
                message: format!(
                    "Resource {} error rate {} exceeds threshold {}",
                    resource.id(),
                    metrics.error_rate(),
                    self.alert_threshold
                ),
                resource_id: resource.id(),
            }).await;
        }
        
        metrics
    }
}
```

## Hook Testing

### Testing Hooks

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_lifecycle_hook() {
        let hook = LoggingHook::new(LogLevel::Debug);
        let resource = MockResource::new();
        
        // Test pre-initialize
        assert!(hook.pre_initialize(&resource).await.is_ok());
        
        // Test post-initialize with success
        let result = Ok(());
        assert!(hook.post_initialize(&resource, &result).await.is_ok());
        
        // Test post-initialize with failure
        let result = Err(Error::InitializationFailed {
            resource: resource.id(),
            reason: "Test failure".into(),
        });
        assert!(hook.post_initialize(&resource, &result).await.is_ok());
    }
    
    #[tokio::test]
    async fn test_hook_chain() {
        let chain = HookChain::new()
            .add(LoggingHook::new(LogLevel::Debug))
            .add(MetricsCollectorHook::new())
            .add(TracingHook::new());
        
        let result = chain.execute(|| async {
            // Simulate operation
            Ok::<_, Error>("Success")
        }).await;
        
        assert_eq!(result.unwrap(), "Success");
    }
    
    #[tokio::test]
    async fn test_conditional_hook() {
        let base_hook = LoggingHook::new(LogLevel::Info);
        let conditional = ConditionalHook::new(
            base_hook,
            |resource| resource.resource_type() == "database"
        );
        
        let db_resource = MockResource::with_type("database");
        let cache_resource = MockResource::with_type("cache");
        
        assert!(conditional.should_run(&db_resource));
        assert!(!conditional.should_run(&cache_resource));
    }
}
```

## Best Practices

1. **Keep hooks lightweight** - Don't perform heavy operations in hooks
2. **Handle errors gracefully** - Hooks shouldn't break resource operations
3. **Use appropriate priorities** - Order matters for dependent hooks
4. **Make hooks configurable** - Allow runtime configuration
5. **Test hooks thoroughly** - Include error scenarios
6. **Document hook behavior** - Clear contracts and side effects
7. **Use async carefully** - Avoid blocking operations
8. **Implement timeouts** - Prevent hooks from hanging
9. **Monitor hook performance** - Track hook execution time
10. **Version hook interfaces** - Maintain backward compatibility
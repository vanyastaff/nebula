---
title: Development Approaches
tags: [nebula, nebula-action, crate]
status: draft
created: 2025-08-17
---


## Overview

This guide covers different approaches to developing Nebula actions, from simple prototypes to production-ready implementations. Choose the approach that best fits your use case and requirements.

## Development Strategies

### 1. Rapid Prototyping with Macros

**When to Use**: Quick experiments, simple actions, learning

```rust
use nebula_action::prelude::*;

// Fastest way to create an action
simple_action!(
    QuickPrototype,
    "quick.prototype",
    Input,
    Output,
    |action, input, context| async move {
        // Your logic here
        Ok(Output { result: process(input) })
    }
);

// Even simpler with inline types
simple_action!(
    InlineAction,
    "inline.action",
    serde_json::Value,  // Generic input
    serde_json::Value,  // Generic output
    |action, input, context| async move {
        let result = json!({
            "processed": true,
            "input": input
        });
        Ok(result)
    }
);
```

**Pros**:

- ✅ Minimal boilerplate
- ✅ Quick to write
- ✅ Good for prototypes

**Cons**:

- ❌ Limited customization
- ❌ No lifecycle hooks
- ❌ Basic error handling

---

### 2. Derive Macro Approach

**When to Use**: Balance between simplicity and control

```rust
use nebula_action::prelude::*;
use nebula_action_derive::Action;

#[derive(Action)]
#[action(
    key = "derived.action",
    name = "Derived Action",
    description = "Action using derive macros",
    version = "1.0.0"
)]
pub struct DerivedAction {
    #[action(config)]
    config: ActionConfig,
    
    #[action(inject)]
    logger: Logger,
    
    #[action(resource)]
    db_pool: PgPool,
}

#[derive(Parameters)]
pub struct DerivedInput {
    #[parameter(
        description = "User email",
        validation = "email",
        required = true
    )]
    pub email: String,
    
    #[parameter(
        description = "User age",
        validation = "range(0, 150)",
        default = 18
    )]
    pub age: u8,
    
    #[parameter(
        description = "Preferences",
        ui_hint = "json_editor"
    )]
    pub preferences: Option<Value>,
}

#[derive(Output)]
pub struct DerivedOutput {
    #[output(description = "User ID")]
    pub user_id: String,
    
    #[output(description = "Creation timestamp")]
    pub created_at: DateTime<Utc>,
}

#[async_trait]
impl ProcessAction for DerivedAction {
    type Input = DerivedInput;
    type Output = DerivedOutput;
    
    async fn execute(
        &self,
        input: Self::Input,
        context: &dyn ExecutionContext,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        // Automatic validation via derive
        // Resources injected automatically
        
        let user = self.db_pool
            .create_user(&input.email, input.age)
            .await?;
        
        self.logger.info(&format!("Created user: {}", user.id));
        
        Ok(ActionResult::Success(DerivedOutput {
            user_id: user.id,
            created_at: user.created_at,
        }))
    }
}
```

**Pros**:

- ✅ Reduces boilerplate
- ✅ Automatic validation
- ✅ Resource injection
- ✅ Good IDE support

**Cons**:

- ❌ Macro complexity
- ❌ Compilation overhead
- ❌ Less flexible than manual

---

### 3. Manual Implementation

**When to Use**: Full control, complex requirements, production systems

```rust
use nebula_action::prelude::*;

pub struct ManualAction {
    metadata: ActionMetadata,
    config: Config,
    validator: Validator,
    transformer: Transformer,
    error_handler: ErrorHandler,
}

impl ManualAction {
    pub fn builder() -> ManualActionBuilder {
        ManualActionBuilder::default()
    }
    
    fn validate_input(&self, input: &Input) -> Result<(), ValidationError> {
        self.validator.validate(input)?;
        
        // Custom validation logic
        if input.amount > self.config.max_amount {
            return Err(ValidationError::ExceedsLimit {
                field: "amount",
                limit: self.config.max_amount,
                actual: input.amount,
            });
        }
        
        Ok(())
    }
    
    async fn process_with_retry(
        &self,
        data: Data,
        context: &dyn ExecutionContext,
    ) -> Result<Processed, ActionError> {
        let mut attempts = 0;
        let mut last_error = None;
        
        while attempts < self.config.max_retries {
            match self.transformer.transform(data.clone()).await {
                Ok(result) => return Ok(result),
                Err(e) if e.is_transient() => {
                    attempts += 1;
                    last_error = Some(e);
                    
                    let delay = self.calculate_backoff(attempts);
                    context.log_warning(&format!(
                        "Retry {} after {:?}: {}",
                        attempts, delay, e
                    ));
                    
                    tokio::time::sleep(delay).await;
                }
                Err(e) => return Err(e.into()),
            }
        }
        
        Err(last_error.unwrap().into())
    }
}

// Implement all required traits manually
impl HasMetadata for ManualAction {
    fn metadata(&self) -> &ActionMetadata {
        &self.metadata
    }
}

impl HasType for ManualAction {
    fn r#type(&self) -> ActionType {
        ActionType::Process
    }
}

impl Action for ManualAction {
    async fn initialize(&self, context: &InitContext) -> Result<(), ActionError> {
        // Custom initialization
        self.validator.load_rules().await?;
        self.transformer.warm_cache().await?;
        Ok(())
    }
    
    async fn health_check(&self) -> Result<HealthStatus, ActionError> {
        // Custom health check
        if !self.transformer.is_healthy().await {
            return Ok(HealthStatus::Unhealthy {
                reason: "Transformer unavailable".to_string(),
                recoverable: true,
            });
        }
        Ok(HealthStatus::Healthy)
    }
    
    async fn shutdown(&self, context: &ShutdownContext) -> Result<(), ActionError> {
        // Custom cleanup
        self.transformer.flush_cache().await?;
        Ok(())
    }
}

#[async_trait]
impl ProcessAction for ManualAction {
    type Input = Input;
    type Output = Output;
    
    async fn execute(
        &self,
        input: Self::Input,
        context: &dyn ExecutionContext,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        // Full control over execution
        self.validate_input(&input)?;
        
        let data = self.preprocess(input)?;
        let processed = self.process_with_retry(data, context).await?;
        let output = self.postprocess(processed)?;
        
        Ok(ActionResult::Success(output))
    }
}
```

**Pros**:

- ✅ Full control
- ✅ Custom logic
- ✅ Optimizable
- ✅ Testable components

**Cons**:

- ❌ More boilerplate
- ❌ More complex
- ❌ Longer development time

---

### 4. Composition Pattern

**When to Use**: Combining multiple actions, complex workflows

```rust
use nebula_action::prelude::*;

pub struct CompositeAction {
    metadata: ActionMetadata,
    actions: Vec<Box<dyn Action>>,
    strategy: CompositionStrategy,
}

pub enum CompositionStrategy {
    Sequential,
    Parallel,
    Pipeline,
    Conditional(Box<dyn Fn(&Value) -> String>),
}

impl CompositeAction {
    pub fn sequential(actions: Vec<Box<dyn Action>>) -> Self {
        Self {
            metadata: create_metadata("composite.sequential"),
            actions,
            strategy: CompositionStrategy::Sequential,
        }
    }
    
    pub fn parallel(actions: Vec<Box<dyn Action>>) -> Self {
        Self {
            metadata: create_metadata("composite.parallel"),
            actions,
            strategy: CompositionStrategy::Parallel,
        }
    }
    
    pub fn pipeline() -> PipelineBuilder {
        PipelineBuilder::new()
    }
}

// Pipeline builder for fluent API
pub struct PipelineBuilder {
    stages: Vec<PipelineStage>,
}

pub struct PipelineStage {
    action: Box<dyn Action>,
    error_handler: ErrorStrategy,
    transform: Option<Box<dyn Fn(Value) -> Value>>,
}

impl PipelineBuilder {
    pub fn add_stage<A: Action + 'static>(mut self, action: A) -> Self {
        self.stages.push(PipelineStage {
            action: Box::new(action),
            error_handler: ErrorStrategy::Propagate,
            transform: None,
        });
        self
    }
    
    pub fn add_stage_with_transform<A, F>(
        mut self,
        action: A,
        transform: F,
    ) -> Self
    where
        A: Action + 'static,
        F: Fn(Value) -> Value + 'static,
    {
        self.stages.push(PipelineStage {
            action: Box::new(action),
            error_handler: ErrorStrategy::Propagate,
            transform: Some(Box::new(transform)),
        });
        self
    }
    
    pub fn add_stage_with_error_handler<A>(
        mut self,
        action: A,
        error_handler: ErrorStrategy,
    ) -> Self
    where
        A: Action + 'static,
    {
        self.stages.push(PipelineStage {
            action: Box::new(action),
            error_handler,
            transform: None,
        });
        self
    }
    
    pub fn build(self) -> CompositeAction {
        CompositeAction {
            metadata: create_metadata("composite.pipeline"),
            actions: self.stages.into_iter().map(|s| s.action).collect(),
            strategy: CompositionStrategy::Pipeline,
        }
    }
}

// Usage example
let pipeline = CompositeAction::pipeline()
    .add_stage(FetchDataAction::new())
    .add_stage_with_transform(
        ValidateAction::new(),
        |data| add_metadata(data, "validated", true)
    )
    .add_stage_with_error_handler(
        TransformAction::new(),
        ErrorStrategy::Skip
    )
    .add_stage(StorageAction::new())
    .build();
```

---

### 5. Plugin Architecture

**When to Use**: Extensible systems, third-party integrations

```rust
use nebula_action::prelude::*;

// Plugin trait
pub trait ActionPlugin: Send + Sync {
    fn name(&self) -> &str;
    fn version(&self) -> &str;
    fn initialize(&mut self, config: &PluginConfig) -> Result<(), PluginError>;
    fn create_action(&self) -> Result<Box<dyn Action>, PluginError>;
}

// Plugin manager
pub struct PluginManager {
    plugins: HashMap<String, Box<dyn ActionPlugin>>,
    registry: ActionRegistry,
}

impl PluginManager {
    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
            registry: ActionRegistry::new(),
        }
    }
    
    pub fn load_plugin<P: ActionPlugin + 'static>(&mut self, plugin: P) -> Result<(), PluginError> {
        let name = plugin.name().to_string();
        let mut plugin = Box::new(plugin);
        
        // Initialize plugin
        let config = self.load_config(&name)?;
        plugin.initialize(&config)?;
        
        // Register action
        let action = plugin.create_action()?;
        self.registry.register(action)?;
        
        self.plugins.insert(name, plugin);
        Ok(())
    }
    
    pub fn load_from_directory(&mut self, path: &Path) -> Result<(), PluginError> {
        // Dynamically load plugins from directory
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let path = entry.path();
            
            if path.extension() == Some(OsStr::new("so")) {
                unsafe {
                    let lib = libloading::Library::new(path)?;
                    let create_plugin: libloading::Symbol<fn() -> Box<dyn ActionPlugin>> =
                        lib.get(b"create_plugin")?;
                    
                    let plugin = create_plugin();
                    self.load_plugin(*plugin)?;
                }
            }
        }
        
        Ok(())
    }
}

// Example plugin implementation
pub struct CustomPlugin {
    config: Option<PluginConfig>,
}

impl ActionPlugin for CustomPlugin {
    fn name(&self) -> &str {
        "custom_plugin"
    }
    
    fn version(&self) -> &str {
        "1.0.0"
    }
    
    fn initialize(&mut self, config: &PluginConfig) -> Result<(), PluginError> {
        self.config = Some(config.clone());
        Ok(())
    }
    
    fn create_action(&self) -> Result<Box<dyn Action>, PluginError> {
        Ok(Box::new(CustomAction::new(self.config.clone()?)))
    }
}

// Export function for dynamic loading
#[no_mangle]
pub extern "C" fn create_plugin() -> Box<dyn ActionPlugin> {
    Box::new(CustomPlugin { config: None })
}
```

## Development Workflow

### 1. Project Structure

```
my-actions/
├── Cargo.toml
├── src/
│   ├── lib.rs
│   ├── actions/
│   │   ├── mod.rs
│   │   ├── http/
│   │   │   ├── mod.rs
│   │   │   ├── request.rs
│   │   │   └── webhook.rs
│   │   ├── database/
│   │   │   ├── mod.rs
│   │   │   ├── query.rs
│   │   │   └── migration.rs
│   │   └── transform/
│   │       ├── mod.rs
│   │       └── data.rs
│   ├── common/
│   │   ├── mod.rs
│   │   ├── error.rs
│   │   ├── config.rs
│   │   └── utils.rs
│   └── tests/
│       ├── mod.rs
│       └── integration.rs
├── tests/
│   └── e2e.rs
└── examples/
    └── basic.rs
```

### 2. Development Steps

```mermaid
graph LR
    A[Requirements] --> B[Design]
    B --> C[Prototype]
    C --> D[Implement]
    D --> E[Test]
    E --> F[Optimize]
    F --> G[Deploy]
    G --> H[Monitor]
```

### 3. Development Tools

#### Action Generator CLI

```bash
# Install the generator
cargo install nebula-action-generator

# Generate a new action
nebula-action new my_action --type process

# Generate with template
nebula-action new my_action --template http-client

# Generate full project
nebula-action init my-actions-project
```

#### Development Dependencies

```toml
[dependencies]
nebula-action = "0.1"
serde = { version = "1.0", features = ["derive"] }
async-trait = "0.1"
tokio = { version = "1", features = ["full"] }

[dev-dependencies]
nebula-action-testing = "0.1"
mockall = "0.11"
proptest = "1.0"
criterion = "0.5"
```

## Testing Strategies

### 1. Unit Testing

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use nebula_action::testing::*;
    
    // Test action creation
    #[test]
    fn test_action_creation() {
        let action = MyAction::new(Config::default());
        assert!(action.is_ok());
        
        let action = action.unwrap();
        assert_eq!(action.metadata().key(), "my_action");
    }
    
    // Test input validation
    #[test]
    fn test_input_validation() {
        let action = MyAction::new(Config::default()).unwrap();
        
        let invalid_input = Input {
            email: "not-an-email",
            age: 200,
        };
        
        let result = action.validate_input(&invalid_input);
        assert!(result.is_err());
        
        let err = result.unwrap_err();
        assert!(err.has_field_error("email"));
        assert!(err.has_field_error("age"));
    }
    
    // Test async execution
    #[tokio::test]
    async fn test_execution() {
        let action = MyAction::new(Config::default()).unwrap();
        let context = TestContext::new();
        
        let input = Input {
            email: "test@example.com",
            age: 25,
        };
        
        let result = action.execute(input, &context).await;
        assert!(result.is_ok());
        
        match result.unwrap() {
            ActionResult::Success(output) => {
                assert!(!output.id.is_empty());
            }
            _ => panic!("Expected success"),
        }
    }
}
```

### 2. Integration Testing

```rust
#[cfg(test)]
mod integration_tests {
    use super::*;
    use testcontainers::*;
    
    #[tokio::test]
    async fn test_with_real_database() {
        // Start test container
        let docker = clients::Cli::default();
        let postgres = docker.run(images::postgres::Postgres::default());
        
        let connection_string = format!(
            "postgresql://postgres:postgres@localhost:{}",
            postgres.get_host_port_ipv4(5432)
        );
        
        // Create action with real database
        let action = DatabaseAction::new(DatabaseConfig {
            connection_string,
            pool_size: 5,
        }).unwrap();
        
        let context = IntegrationContext::new();
        
        // Test with real database
        let input = QueryInput {
            query: "SELECT 1".to_string(),
        };
        
        let result = action.execute(input, &context).await.unwrap();
        assert!(matches!(result, ActionResult::Success(_)));
    }
}
```

### 3. Property-Based Testing

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn test_transform_preserves_data(
        data in any::<Value>(),
        format in prop::sample::select(vec![Format::Json, Format::Yaml, Format::Toml])
    ) {
        let action = TransformAction::new();
        
        // Transform to format and back
        let transformed = action.transform(data.clone(), format)?;
        let restored = action.transform(transformed, Format::Json)?;
        
        // Data should be preserved
        prop_assert_eq!(data, restored);
    }
    
    #[test]
    fn test_rate_limiter_bounds(
        requests in 0usize..1000,
        rate in 1.0f64..100.0
    ) {
        let action = RateLimiterAction::new(rate);
        let mut state = RateLimiterState::default();
        
        let allowed = simulate_requests(&action, &mut state, requests);
        
        // Should not exceed rate limit
        prop_assert!(allowed as f64 <= rate * duration_secs);
    }
}
```

### 4. Performance Testing

```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn bench_action_execution(c: &mut Criterion) {
    let action = MyAction::new(Config::default()).unwrap();
    let context = BenchContext::new();
    let input = create_test_input();
    
    c.bench_function("action_execution", |b| {
        b.to_async(tokio::runtime::Runtime::new().unwrap())
            .iter(|| async {
                let result = action.execute(
                    black_box(input.clone()),
                    black_box(&context)
                ).await;
                black_box(result)
            });
    });
}

criterion_group!(benches, bench_action_execution);
criterion_main!(benches);
```

## Error Handling Patterns

### 1. Result Type Pattern

```rust
// Define custom result type
pub type ActionResult<T> = Result<T, ActionError>;

// Use throughout the codebase
impl MyAction {
    pub fn new(config: Config) -> ActionResult<Self> {
        config.validate()?;
        Ok(Self { config })
    }
    
    pub async fn process(&self, data: Data) -> ActionResult<Processed> {
        let validated = self.validate(data)?;
        let transformed = self.transform(validated)?;
        Ok(transformed)
    }
}
```

### 2. Error Context Pattern

```rust
use anyhow::{Context, Result};

impl MyAction {
    pub async fn complex_operation(&self) -> Result<Output> {
        let data = fetch_data()
            .await
            .context("Failed to fetch data")?;
        
        let processed = process_data(data)
            .await
            .with_context(|| format!("Failed to process {} items", data.len()))?;
        
        let stored = store_results(processed)
            .await
            .context("Failed to store results")?;
        
        Ok(stored)
    }
}
```

### 3. Error Recovery Pattern

```rust
impl MyAction {
    pub async fn execute_with_recovery(
        &self,
        input: Input,
        context: &dyn ExecutionContext,
    ) -> ActionResult<Output> {
        match self.try_execute(input.clone(), context).await {
            Ok(output) => Ok(output),
            Err(e) if e.is_recoverable() => {
                context.log_warning(&format!("Recovering from error: {}", e));
                self.recover_from_error(e, input, context).await
            }
            Err(e) => {
                context.log_error(&format!("Unrecoverable error: {}", e));
                Err(e)
            }
        }
    }
    
    async fn recover_from_error(
        &self,
        error: ActionError,
        input: Input,
        context: &dyn ExecutionContext,
    ) -> ActionResult<Output> {
        match error {
            ActionError::NetworkError { .. } => {
                // Try alternative endpoint
                self.try_alternative_endpoint(input, context).await
            }
            ActionError::RateLimited { retry_after } => {
                // Wait and retry
                tokio::time::sleep(retry_after).await;
                self.try_execute(input, context).await
            }
            _ => Err(error),
        }
    }
}
```

## Performance Optimization

### 1. Async Optimization

```rust
impl MyAction {
    // Concurrent execution
    pub async fn process_batch(&self, items: Vec<Item>) -> Vec<Result<Output>> {
        let futures = items
            .into_iter()
            .map(|item| self.process_item(item));
        
        futures::future::join_all(futures).await
    }
    
    // Bounded concurrency
    pub async fn process_batch_bounded(&self, items: Vec<Item>) -> Vec<Result<Output>> {
        use futures::stream::{self, StreamExt};
        
        stream::iter(items)
            .map(|item| self.process_item(item))
            .buffer_unordered(10) // Max 10 concurrent
            .collect()
            .await
    }
}
```

### 2. Caching Strategy

```rust
pub struct CachedAction {
    inner: Arc<InnerAction>,
    cache: Arc<RwLock<LruCache<CacheKey, CachedResult>>>,
}

impl CachedAction {
    pub async fn execute_with_cache(
        &self,
        input: Input,
        context: &dyn ExecutionContext,
    ) -> ActionResult<Output> {
        let cache_key = self.compute_cache_key(&input);
        
        // Check cache
        {
            let cache = self.cache.read().await;
            if let Some(cached) = cache.get(&cache_key) {
                if !cached.is_expired() {
                    context.log_debug("Cache hit");
                    return Ok(cached.value.clone());
                }
            }
        }
        
        // Execute and cache
        let result = self.inner.execute(input, context).await?;
        
        {
            let mut cache = self.cache.write().await;
            cache.put(cache_key, CachedResult {
                value: result.clone(),
                expires_at: Utc::now() + Duration::from_secs(300),
            });
        }
        
        Ok(result)
    }
}
```

### 3. Resource Pooling

```rust
pub struct PooledAction {
    connection_pool: Arc<Pool<ConnectionManager>>,
    semaphore: Arc<Semaphore>,
}

impl PooledAction {
    pub async fn execute_with_pooling(
        &self,
        input: Input,
        context: &dyn ExecutionContext,
    ) -> ActionResult<Output> {
        // Acquire semaphore permit
        let _permit = self.semaphore.acquire().await?;
        
        // Get connection from pool
        let conn = self.connection_pool.get().await?;
        
        // Use connection
        let result = self.process_with_connection(input, conn).await?;
        
        // Connection automatically returned to pool when dropped
        Ok(result)
    }
}
```

## Deployment Considerations

### 1. Configuration Management

```rust
#[derive(Deserialize)]
pub struct ActionConfig {
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
    
    #[serde(default = "default_retry_count")]
    pub max_retries: u32,
    
    #[serde(default)]
    pub features: FeatureFlags,
    
    #[serde(default)]
    pub endpoints: HashMap<String, Endpoint>,
}

impl ActionConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        envy::from_env()
    }
    
    pub fn from_file(path: &Path) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path)?;
        toml::from_str(&content).map_err(Into::into)
    }
    
    pub fn layered() -> Result<Self, ConfigError> {
        let mut config = Config::builder()
            .add_source(config::File::with_name("default"))
            .add_source(config::File::with_name("local").required(false))
            .add_source(config::Environment::with_prefix("ACTION"))
            .build()?;
        
        config.try_deserialize()
    }
}
```

### 2. Monitoring and Metrics

```rust
impl MonitoredAction {
    pub async fn execute_with_monitoring(
        &self,
        input: Input,
        context: &dyn ExecutionContext,
    ) -> ActionResult<Output> {
        let timer = context.start_timer("action_duration");
        let start = Instant::now();
        
        let result = self.inner.execute(input, context).await;
        
        let duration = start.elapsed();
        timer.stop_and_record();
        
        // Record metrics
        match &result {
            Ok(ActionResult::Success(_)) => {
                context.increment_counter("action_success", 1.0, &[]);
            }
            Ok(ActionResult::Skip { .. }) => {
                context.increment_counter("action_skip", 1.0, &[]);
            }
            Err(e) => {
                context.increment_counter("action_error", 1.0, &[
                    ("error_type", e.error_type()),
                ]);
            }
        }
        
        // Record latency histogram
        context.record_histogram("action_latency_ms", duration.as_millis() as f64, &[]);
        
        result
    }
}
```

## Best Practices Summary

### ✅ DO's

1. **Start simple** - Use macros for prototypes
2. **Iterate** - Refactor as requirements grow
3. **Test thoroughly** - Unit, integration, and property tests
4. **Handle errors gracefully** - Use specific error types
5. **Monitor performance** - Add metrics and logging
6. **Document behavior** - Especially edge cases
7. **Version carefully** - Follow semver

### ❌ DON'Ts

1. **Over-engineer** - Start with the simplest solution
2. **Ignore errors** - Always handle or propagate
3. **Block async runtime** - Use spawn_blocking for CPU work
4. **Leak resources** - Always clean up
5. **Hardcode configuration** - Use config files/env vars
6. **Skip tests** - Test critical paths

## Related Documentation

- [[Action Types]] - Different action types
- [[Action Lifecycle]] - Lifecycle management
- [[Testing]] - Testing strategies
- [[Examples]] - Code examples
- [[Best Practices]] - General best practices

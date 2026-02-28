# nebula-resource

Advanced resource management system for Nebula. Provides lifecycle management, pooling, health monitoring, and tier-specific optimizations for long-lived resources.

## Table of Contents

1. [Overview](#overview)
2. [Architecture](#architecture)
3. [Core Concepts](#core-concepts)
4. [Resource Types](#resource-types)
5. [Usage Examples](#usage-examples)
6. [Integration with Actions](#integration-with-actions)
7. [Resource Lifecycle](#resource-lifecycle)
8. [Monitoring & Health](#monitoring--health)
9. [Testing](#testing)
10. [Best Practices](#best-practices)

## Overview

nebula-resource provides:
- **Unified resource management** for databases, APIs, message queues, etc.
- **Automatic lifecycle management** with health checks and recovery
- **Resource pooling** with smart allocation strategies
- **Tier-aware optimizations** from simple to enterprise deployments
- **Memory-efficient operations** with automatic pressure handling

## Architecture

### File Structure

```
nebula-resource/
├── Cargo.toml
├── README.md
├── src/
│   ├── lib.rs                    # Main exports and prelude
│   │
│   ├── core/                     # Core traits and types
│   │   ├── mod.rs
│   │   ├── resource.rs           # Resource trait
│   │   ├── instance.rs           # ResourceInstance
│   │   ├── metadata.rs           # Resource metadata
│   │   ├── health.rs             # Health checking
│   │   └── error.rs              # Error types
│   │
│   ├── manager/                  # Resource manager
│   │   ├── mod.rs
│   │   ├── manager.rs            # Main ResourceManager
│   │   ├── registry.rs           # Resource type registry
│   │   ├── lifecycle.rs          # Lifecycle management
│   │   ├── allocation.rs         # Resource allocation
│   │   └── recovery.rs           # Failure recovery
│   │
│   ├── pool/                     # Resource pooling
│   │   ├── mod.rs
│   │   ├── pool.rs               # Generic resource pool
│   │   ├── strategies.rs         # Pooling strategies
│   │   ├── metrics.rs            # Pool metrics
│   │   └── maintenance.rs        # Pool maintenance
│   │
│   ├── types/                    # Built-in resource types
│   │   ├── mod.rs
│   │   ├── database.rs           # Database connections
│   │   ├── http_client.rs        # HTTP clients
│   │   ├── message_queue.rs      # MQ connections
│   │   ├── cache.rs              # Cache clients
│   │   ├── storage.rs            # Object storage
│   │   └── custom.rs             # Custom resources
│   │
│   ├── health/                   # Health monitoring
│   │   ├── mod.rs
│   │   ├── checker.rs            # Health checker
│   │   ├── strategies.rs         # Check strategies
│   │   ├── aggregator.rs         # Health aggregation
│   │   └── recovery.rs           # Recovery actions
│   │
│   ├── config/                   # Configuration
│   │   ├── mod.rs
│   │   ├── resource_config.rs    # Resource configs
│   │   ├── pool_config.rs        # Pool configs
│   │   ├── tier_config.rs        # Tier-specific configs
│   │   └── validation.rs         # Config validation
│   │
│   ├── monitoring/               # Monitoring & metrics
│   │   ├── mod.rs
│   │   ├── metrics.rs            # Metrics collection
│   │   ├── alerts.rs             # Alert management
│   │   ├── dashboard.rs          # Monitoring dashboard
│   │   └── tracing.rs            # Distributed tracing
│   │
│   ├── optimization/             # Resource optimization
│   │   ├── mod.rs
│   │   ├── memory.rs             # Memory optimization
│   │   ├── connection.rs         # Connection optimization
│   │   ├── caching.rs            # Caching strategies
│   │   └── predictor.rs          # Usage prediction
│   │
│   └── prelude.rs                # Common imports
│
├── examples/
│   ├── database_pool.rs          # Database connection pool
│   ├── http_client_pool.rs       # HTTP client pool
│   ├── multi_resource.rs         # Multiple resource types
│   ├── health_monitoring.rs      # Health check setup
│   └── tier_optimization.rs      # Tier-specific configs
│
└── tests/
    ├── integration/
    └── unit/
```

## Core Concepts

### Resource Trait

The foundation of the resource system:

```rust
#[async_trait]
pub trait Resource: Send + Sync + 'static {
    /// Configuration type for this resource
    type Config: ResourceConfig;
    
    /// Instance type that will be created
    type Instance: ResourceInstance;
    
    /// Metadata about this resource type
    fn metadata(&self) -> ResourceMetadata {
        ResourceMetadata {
            id: std::any::type_name::<Self>(),
            name: "Unknown Resource",
            description: "No description",
            category: ResourceCategory::Other,
            capabilities: ResourceCapabilities::default(),
        }
    }
    
    /// Create a new instance of this resource
    async fn create(
        &self,
        config: &Self::Config,
        context: &ResourceContext,
    ) -> Result<Self::Instance, ResourceError>;
    
    /// Validate configuration before creation
    fn validate_config(&self, config: &Self::Config) -> Result<(), ResourceError> {
        config.validate()
    }
    
    /// Estimate resource requirements
    fn estimate_requirements(
        &self,
        config: &Self::Config,
    ) -> ResourceRequirements {
        ResourceRequirements::default()
    }
    
    /// Check if resource supports pooling
    fn supports_pooling(&self) -> bool {
        true
    }
}

/// Resource instance that can be used
#[async_trait]
pub trait ResourceInstance: Send + Sync + 'static {
    /// Get unique identifier for this instance
    fn id(&self) -> &ResourceInstanceId;
    
    /// Check health of this instance
    async fn health_check(&self) -> Result<HealthStatus, ResourceError>;
    
    /// Cleanup when instance is destroyed
    async fn cleanup(&mut self) -> Result<(), ResourceError> {
        Ok(())
    }
    
    /// Get current metrics
    fn metrics(&self) -> ResourceMetrics {
        ResourceMetrics::default()
    }
    
    /// Check if instance can be reused
    fn is_reusable(&self) -> bool {
        true
    }
    
    /// Reset instance for reuse (if pooled)
    async fn reset(&mut self) -> Result<(), ResourceError> {
        Ok(())
    }
}

/// Resource configuration trait
pub trait ResourceConfig: Serialize + DeserializeOwned + Send + Sync {
    /// Validate configuration
    fn validate(&self) -> Result<(), ResourceError>;
    
    /// Get tier-specific adjustments
    fn adjust_for_tier(&mut self, tier: &DeploymentTier) {
        // Default: no adjustments
    }
}
```

### ResourceManager

Central management for all resources:

```rust
pub struct ResourceManager {
    registry: Arc<ResourceRegistry>,
    pools: Arc<DashMap<ResourceTypeId, Box<dyn ResourcePool>>>,
    health_monitor: Arc<HealthMonitor>,
    lifecycle_manager: Arc<LifecycleManager>,
    optimization_engine: Arc<OptimizationEngine>,
    metrics_collector: Arc<MetricsCollector>,
}

impl ResourceManager {
    /// Register a resource type
    pub async fn register_resource_type<R: Resource>(
        &self,
        resource: R,
    ) -> Result<(), ResourceError> {
        self.registry.register(resource).await
    }
    
    /// Create or get resource instance
    pub async fn get_instance<T>(
        &self,
        resource_type: &str,
        config: &T::Config,
    ) -> Result<Arc<T>, ResourceError>
    where
        T: ResourceInstance + 'static,
    {
        // Check if pooling is enabled
        if let Some(pool) = self.pools.get(resource_type) {
            // Try to get from pool
            if let Some(instance) = pool.try_acquire().await? {
                return Ok(instance);
            }
        }
        
        // Create new instance
        let resource = self.registry.get(resource_type)?;
        let instance = resource.create(config, &self.create_context()).await?;
        
        // Register with health monitor
        self.health_monitor.register(&instance).await;
        
        // Add to pool if supported
        if resource.supports_pooling() {
            self.add_to_pool(resource_type, &instance).await?;
        }
        
        Ok(Arc::new(instance))
    }
    
    /// Get resource with automatic tier optimization
    pub async fn get_optimized<T>(
        &self,
        resource_type: &str,
        base_config: T::Config,
        tier: &DeploymentTier,
    ) -> Result<Arc<T>, ResourceError>
    where
        T: ResourceInstance + 'static,
    {
        // Adjust config for tier
        let mut config = base_config;
        config.adjust_for_tier(tier);
        
        // Apply optimizations
        let optimized_config = self.optimization_engine
            .optimize_config(resource_type, config, tier)
            .await?;
        
        self.get_instance::<T>(resource_type, &optimized_config).await
    }
}
```

### Resource Pooling

Advanced pooling with multiple strategies:

```rust
#[async_trait]
pub trait ResourcePool: Send + Sync {
    /// Try to acquire resource from pool
    async fn try_acquire(&self) -> Result<Option<Arc<dyn ResourceInstance>>, ResourceError>;
    
    /// Return resource to pool
    async fn release(&self, instance: Arc<dyn ResourceInstance>) -> Result<(), ResourceError>;
    
    /// Get pool statistics
    fn stats(&self) -> PoolStats;
    
    /// Perform maintenance
    async fn maintain(&self) -> Result<MaintenanceReport, ResourceError>;
}

/// Generic resource pool implementation
pub struct GenericResourcePool<T: ResourceInstance> {
    config: PoolConfig,
    strategy: Box<dyn PoolingStrategy>,
    instances: Arc<SegQueue<PooledInstance<T>>>,
    health_checker: Arc<HealthChecker>,
    metrics: Arc<PoolMetrics>,
}

#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// Minimum instances to maintain
    pub min_size: usize,
    
    /// Maximum instances allowed
    pub max_size: usize,
    
    /// Instance idle timeout
    pub idle_timeout: Duration,
    
    /// Health check interval
    pub health_check_interval: Duration,
    
    /// Pooling strategy
    pub strategy: PoolingStrategyType,
    
    /// Tier-specific overrides
    pub tier_overrides: HashMap<DeploymentTier, PoolConfigOverride>,
}

#[derive(Debug, Clone)]
pub enum PoolingStrategyType {
    /// First In First Out
    FIFO,
    
    /// Last In First Out
    LIFO,
    
    /// Least Recently Used
    LRU,
    
    /// Least Frequently Used
    LFU,
    
    /// Weighted by health/performance
    Weighted,
    
    /// Custom strategy
    Custom(String),
}
```

## Resource Types

### 1. Database Resource

```rust
use nebula_resource::prelude::*;

#[derive(Resource)]
#[resource(
    id = "postgres",
    name = "PostgreSQL Connection",
    category = "Database"
)]
pub struct PostgresResource;

#[derive(Config, Serialize, Deserialize)]
pub struct PostgresConfig {
    #[config(description = "Connection string", sensitive = true)]
    pub connection_string: String,
    
    #[config(description = "Maximum connections", default = 10)]
    pub max_connections: u32,
    
    #[config(description = "Connection timeout", default = "30s")]
    pub connection_timeout: Duration,
    
    #[config(description = "Idle timeout", default = "10m")]
    pub idle_timeout: Duration,
    
    #[config(description = "Enable SSL", default = true)]
    pub ssl_mode: SslMode,
}

impl ResourceConfig for PostgresConfig {
    fn validate(&self) -> Result<(), ResourceError> {
        if self.connection_string.is_empty() {
            return Err(ResourceError::InvalidConfig(
                "Connection string cannot be empty".into()
            ));
        }
        
        if self.max_connections == 0 {
            return Err(ResourceError::InvalidConfig(
                "Max connections must be greater than 0".into()
            ));
        }
        
        Ok(())
    }
    
    fn adjust_for_tier(&mut self, tier: &DeploymentTier) {
        match tier {
            DeploymentTier::Personal => {
                self.max_connections = self.max_connections.min(5);
                self.idle_timeout = Duration::from_secs(300); // 5 minutes
            }
            DeploymentTier::Enterprise => {
                self.max_connections = self.max_connections.max(20);
                self.idle_timeout = Duration::from_secs(600); // 10 minutes
            }
            DeploymentTier::Cloud => {
                self.max_connections = self.max_connections.max(50);
                self.idle_timeout = Duration::from_secs(900); // 15 minutes
            }
        }
    }
}

pub struct PostgresInstance {
    id: ResourceInstanceId,
    pool: PgPool,
    config: PostgresConfig,
    created_at: Instant,
    last_used: Arc<Mutex<Instant>>,
    query_count: Arc<AtomicU64>,
}

#[async_trait]
impl ResourceInstance for PostgresInstance {
    fn id(&self) -> &ResourceInstanceId {
        &self.id
    }
    
    async fn health_check(&self) -> Result<HealthStatus, ResourceError> {
        match sqlx::query("SELECT 1")
            .fetch_one(&self.pool)
            .await
        {
            Ok(_) => Ok(HealthStatus::Healthy),
            Err(e) => Ok(HealthStatus::Unhealthy {
                reason: e.to_string(),
                recoverable: true,
            }),
        }
    }
    
    async fn cleanup(&mut self) -> Result<(), ResourceError> {
        self.pool.close().await;
        Ok(())
    }
    
    fn metrics(&self) -> ResourceMetrics {
        ResourceMetrics {
            usage_count: self.query_count.load(Ordering::Relaxed),
            last_used: *self.last_used.lock().unwrap(),
            health_score: 1.0, // Would calculate based on errors/latency
            custom_metrics: hashmap! {
                "active_connections" => self.pool.size() as f64,
                "idle_connections" => self.pool.num_idle() as f64,
            },
        }
    }
}

#[async_trait]
impl Resource for PostgresResource {
    type Config = PostgresConfig;
    type Instance = PostgresInstance;
    
    async fn create(
        &self,
        config: &Self::Config,
        context: &ResourceContext,
    ) -> Result<Self::Instance, ResourceError> {
        context.log_info(&format!(
            "Creating PostgreSQL connection pool with {} max connections",
            config.max_connections
        ));
        
        let pool = PgPoolOptions::new()
            .max_connections(config.max_connections)
            .connect_timeout(config.connection_timeout)
            .idle_timeout(Some(config.idle_timeout))
            .connect(&config.connection_string)
            .await
            .map_err(|e| ResourceError::CreationFailed(e.to_string()))?;
        
        // Test connection
        sqlx::query("SELECT version()")
            .fetch_one(&pool)
            .await
            .map_err(|e| ResourceError::CreationFailed(
                format!("Failed to test connection: {}", e)
            ))?;
        
        Ok(PostgresInstance {
            id: ResourceInstanceId::new(),
            pool,
            config: config.clone(),
            created_at: Instant::now(),
            last_used: Arc::new(Mutex::new(Instant::now())),
            query_count: Arc::new(AtomicU64::new(0)),
        })
    }
    
    fn estimate_requirements(
        &self,
        config: &Self::Config,
    ) -> ResourceRequirements {
        ResourceRequirements {
            memory_mb: Some(config.max_connections as usize * 10), // ~10MB per connection
            cpu_shares: Some(0.1 * config.max_connections as f64),
            network_bandwidth_mbps: Some(10.0),
            persistent_storage_mb: None,
        }
    }
}

// Extension trait for easy database operations
impl PostgresInstance {
    pub async fn query<T>(&self, query: &str) -> Result<Vec<T>, sqlx::Error>
    where
        T: for<'r> FromRow<'r, PgRow> + Send + Unpin,
    {
        self.query_count.fetch_add(1, Ordering::Relaxed);
        *self.last_used.lock().unwrap() = Instant::now();
        
        sqlx::query_as(query)
            .fetch_all(&self.pool)
            .await
    }
    
    pub async fn execute(&self, query: &str) -> Result<PgQueryResult, sqlx::Error> {
        self.query_count.fetch_add(1, Ordering::Relaxed);
        *self.last_used.lock().unwrap() = Instant::now();
        
        sqlx::query(query)
            .execute(&self.pool)
            .await
    }
}
```

### 2. HTTP Client Resource

```rust
#[derive(Resource)]
#[resource(
    id = "http_client",
    name = "HTTP Client",
    category = "Network"
)]
pub struct HttpClientResource;

#[derive(Config, Serialize, Deserialize)]
pub struct HttpClientConfig {
    #[config(description = "Base URL")]
    pub base_url: Option<String>,
    
    #[config(description = "Request timeout", default = "30s")]
    pub timeout: Duration,
    
    #[config(description = "Maximum redirects", default = 10)]
    pub max_redirects: usize,
    
    #[config(description = "Enable compression", default = true)]
    pub compression: bool,
    
    #[config(description = "Connection pool size", default = 100)]
    pub pool_max_idle_per_host: usize,
    
    #[config(description = "Default headers")]
    pub default_headers: HashMap<String, String>,
    
    #[config(description = "Retry configuration")]
    pub retry_config: RetryConfig,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct RetryConfig {
    pub max_retries: u32,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
    pub backoff_multiplier: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_secs(10),
            backoff_multiplier: 2.0,
        }
    }
}

pub struct HttpClientInstance {
    id: ResourceInstanceId,
    client: reqwest::Client,
    config: HttpClientConfig,
    request_count: Arc<AtomicU64>,
    error_count: Arc<AtomicU64>,
    total_latency: Arc<AtomicU64>,
}

#[async_trait]
impl ResourceInstance for HttpClientInstance {
    fn id(&self) -> &ResourceInstanceId {
        &self.id
    }
    
    async fn health_check(&self) -> Result<HealthStatus, ResourceError> {
        let check_url = self.config.base_url.as_ref()
            .map(|base| format!("{}/health", base))
            .unwrap_or_else(|| "https://httpbin.org/status/200".to_string());
        
        match self.client.get(&check_url).send().await {
            Ok(response) if response.status().is_success() => {
                Ok(HealthStatus::Healthy)
            }
            Ok(response) => {
                Ok(HealthStatus::Degraded {
                    reason: format!("Health check returned {}", response.status()),
                    performance_impact: 0.5,
                })
            }
            Err(e) => {
                Ok(HealthStatus::Unhealthy {
                    reason: e.to_string(),
                    recoverable: true,
                })
            }
        }
    }
    
    fn metrics(&self) -> ResourceMetrics {
        let request_count = self.request_count.load(Ordering::Relaxed);
        let error_count = self.error_count.load(Ordering::Relaxed);
        let total_latency = self.total_latency.load(Ordering::Relaxed);
        
        let avg_latency = if request_count > 0 {
            total_latency as f64 / request_count as f64
        } else {
            0.0
        };
        
        let error_rate = if request_count > 0 {
            error_count as f64 / request_count as f64
        } else {
            0.0
        };
        
        ResourceMetrics {
            usage_count: request_count,
            last_used: Instant::now(), // Would track actual last use
            health_score: 1.0 - error_rate,
            custom_metrics: hashmap! {
                "error_rate" => error_rate,
                "avg_latency_ms" => avg_latency,
                "total_requests" => request_count as f64,
            },
        }
    }
}

#[async_trait]
impl Resource for HttpClientResource {
    type Config = HttpClientConfig;
    type Instance = HttpClientInstance;
    
    async fn create(
        &self,
        config: &Self::Config,
        context: &ResourceContext,
    ) -> Result<Self::Instance, ResourceError> {
        let mut client_builder = reqwest::Client::builder()
            .timeout(config.timeout)
            .max_tls_version(tls::Version::TLS_1_3)
            .redirect(reqwest::redirect::Policy::limited(config.max_redirects));
        
        if config.compression {
            client_builder = client_builder.gzip(true).brotli(true);
        }
        
        if config.pool_max_idle_per_host > 0 {
            client_builder = client_builder
                .pool_max_idle_per_host(config.pool_max_idle_per_host);
        }
        
        // Add default headers
        if !config.default_headers.is_empty() {
            let mut headers = reqwest::header::HeaderMap::new();
            for (key, value) in &config.default_headers {
                headers.insert(
                    reqwest::header::HeaderName::from_str(key)?,
                    reqwest::header::HeaderValue::from_str(value)?,
                );
            }
            client_builder = client_builder.default_headers(headers);
        }
        
        let client = client_builder
            .build()
            .map_err(|e| ResourceError::CreationFailed(e.to_string()))?;
        
        Ok(HttpClientInstance {
            id: ResourceInstanceId::new(),
            client,
            config: config.clone(),
            request_count: Arc::new(AtomicU64::new(0)),
            error_count: Arc::new(AtomicU64::new(0)),
            total_latency: Arc::new(AtomicU64::new(0)),
        })
    }
}

// Convenience methods
impl HttpClientInstance {
    pub async fn get(&self, url: &str) -> Result<reqwest::Response, reqwest::Error> {
        self.execute_with_retry(self.client.get(url)).await
    }
    
    pub async fn post(&self, url: &str) -> Result<reqwest::RequestBuilder, reqwest::Error> {
        Ok(self.client.post(url))
    }
    
    async fn execute_with_retry(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<reqwest::Response, reqwest::Error> {
        let start = Instant::now();
        self.request_count.fetch_add(1, Ordering::Relaxed);
        
        let mut retries = 0;
        let mut backoff = self.config.retry_config.initial_backoff;
        
        loop {
            match request.try_clone().unwrap().send().await {
                Ok(response) => {
                    let latency = start.elapsed().as_millis() as u64;
                    self.total_latency.fetch_add(latency, Ordering::Relaxed);
                    return Ok(response);
                }
                Err(e) if retries < self.config.retry_config.max_retries => {
                    if e.is_timeout() || e.is_connect() {
                        retries += 1;
                        tokio::time::sleep(backoff).await;
                        backoff = std::cmp::min(
                            backoff.mul_f64(self.config.retry_config.backoff_multiplier),
                            self.config.retry_config.max_backoff,
                        );
                        continue;
                    }
                    self.error_count.fetch_add(1, Ordering::Relaxed);
                    return Err(e);
                }
                Err(e) => {
                    self.error_count.fetch_add(1, Ordering::Relaxed);
                    return Err(e);
                }
            }
        }
    }
}
```

### 3. Message Queue Resource

```rust
#[derive(Resource)]
#[resource(
    id = "rabbitmq",
    name = "RabbitMQ Connection",
    category = "MessageQueue"
)]
pub struct RabbitMQResource;

#[derive(Config, Serialize, Deserialize)]
pub struct RabbitMQConfig {
    #[config(description = "AMQP URL", sensitive = true)]
    pub url: String,
    
    #[config(description = "Virtual host", default = "/")]
    pub vhost: String,
    
    #[config(description = "Prefetch count", default = 10)]
    pub prefetch_count: u16,
    
    #[config(description = "Heartbeat interval", default = "60s")]
    pub heartbeat: Duration,
    
    #[config(description = "Connection name")]
    pub connection_name: Option<String>,
}

pub struct RabbitMQInstance {
    id: ResourceInstanceId,
    connection: lapin::Connection,
    channels: Arc<DashMap<String, lapin::Channel>>,
    config: RabbitMQConfig,
    message_count: Arc<AtomicU64>,
}

#[async_trait]
impl ResourceInstance for RabbitMQInstance {
    fn id(&self) -> &ResourceInstanceId {
        &self.id
    }
    
    async fn health_check(&self) -> Result<HealthStatus, ResourceError> {
        match self.connection.status().state() {
            lapin::ConnectionState::Connected => Ok(HealthStatus::Healthy),
            lapin::ConnectionState::Connecting => Ok(HealthStatus::Degraded {
                reason: "Connection is still establishing".to_string(),
                performance_impact: 0.8,
            }),
            _ => Ok(HealthStatus::Unhealthy {
                reason: "Connection is closed or errored".to_string(),
                recoverable: true,
            }),
        }
    }
    
    async fn cleanup(&mut self) -> Result<(), ResourceError> {
        // Close all channels
        for channel in self.channels.iter() {
            let _ = channel.close(200, "Normal shutdown").await;
        }
        
        // Close connection
        self.connection
            .close(200, "Normal shutdown")
            .await
            .map_err(|e| ResourceError::CleanupFailed(e.to_string()))?;
        
        Ok(())
    }
}

#[async_trait]
impl Resource for RabbitMQResource {
    type Config = RabbitMQConfig;
    type Instance = RabbitMQInstance;
    
    async fn create(
        &self,
        config: &Self::Config,
        context: &ResourceContext,
    ) -> Result<Self::Instance, ResourceError> {
        let options = lapin::ConnectionProperties::default()
            .with_connection_name(
                config.connection_name
                    .clone()
                    .unwrap_or_else(|| "nebula-resource".to_string())
                    .into()
            )
            .with_heartbeat(config.heartbeat.as_secs() as u16);
        
        let connection = lapin::Connection::connect(&config.url, options)
            .await
            .map_err(|e| ResourceError::CreationFailed(e.to_string()))?;
        
        Ok(RabbitMQInstance {
            id: ResourceInstanceId::new(),
            connection,
            channels: Arc::new(DashMap::new()),
            config: config.clone(),
            message_count: Arc::new(AtomicU64::new(0)),
        })
    }
}

// Convenience methods
impl RabbitMQInstance {
    pub async fn get_channel(&self, name: &str) -> Result<lapin::Channel, lapin::Error> {
        if let Some(channel) = self.channels.get(name) {
            if channel.status().connected() {
                return Ok(channel.clone());
            }
        }
        
        // Create new channel
        let channel = self.connection.create_channel().await?;
        channel.basic_qos(self.config.prefetch_count, &Default::default()).await?;
        
        self.channels.insert(name.to_string(), channel.clone());
        Ok(channel)
    }
    
    pub async fn publish(
        &self,
        exchange: &str,
        routing_key: &str,
        payload: &[u8],
    ) -> Result<(), lapin::Error> {
        let channel = self.get_channel("publish").await?;
        self.message_count.fetch_add(1, Ordering::Relaxed);
        
        channel
            .basic_publish(
                exchange,
                routing_key,
                Default::default(),
                payload,
                Default::default(),
            )
            .await?
            .await
    }
}
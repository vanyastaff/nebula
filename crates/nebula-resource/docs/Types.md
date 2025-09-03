# Types Reference

## Core Types

### `ResourceId`

Unique identifier for resources.

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ResourceId(Arc<str>);

impl ResourceId {
    /// Create new resource ID
    pub fn new() -> Self {
        Self(Arc::from(Uuid::new_v4().to_string()))
    }
    
    /// Create from string
    pub fn from_str(s: &str) -> Self {
        Self(Arc::from(s))
    }
    
    /// Create with prefix
    pub fn with_prefix(prefix: &str) -> Self {
        Self(Arc::from(format!("{}_{}", prefix, Uuid::new_v4())))
    }
    
    /// Parse from various formats
    pub fn parse<S: AsRef<str>>(s: S) -> Result<Self> {
        let s = s.as_ref();
        if s.is_empty() {
            return Err(Error::InvalidResourceId("Empty ID".into()));
        }
        Ok(Self::from_str(s))
    }
    
    /// Get as string slice
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for ResourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
```

### `ResourceHandle<R>`

Handle to a managed resource.

```rust
pub struct ResourceHandle<R: Resource> {
    inner: Arc<R>,
    id: ResourceId,
    manager: Weak<ResourceManager>,
    metadata: Arc<RwLock<HandleMetadata>>,
}

pub struct HandleMetadata {
    pub created_at: Instant,
    pub last_accessed: Instant,
    pub access_count: AtomicU64,
    pub tags: HashMap<String, String>,
}

impl<R: Resource> ResourceHandle<R> {
    /// Get reference to resource
    pub fn get(&self) -> &R {
        self.metadata.write().unwrap().last_accessed = Instant::now();
        self.metadata.access_count.fetch_add(1, Ordering::Relaxed);
        &self.inner
    }
    
    /// Get Arc to resource
    pub fn clone_inner(&self) -> Arc<R> {
        self.inner.clone()
    }
    
    /// Get resource ID
    pub fn id(&self) -> &ResourceId {
        &self.id
    }
    
    /// Get handle metadata
    pub fn metadata(&self) -> HandleMetadata {
        self.metadata.read().unwrap().clone()
    }
    
    /// Check if resource is still valid
    pub fn is_valid(&self) -> bool {
        self.manager.upgrade().is_some()
    }
}

impl<R: Resource> Clone for ResourceHandle<R> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            id: self.id.clone(),
            manager: self.manager.clone(),
            metadata: self.metadata.clone(),
        }
    }
}

impl<R: Resource> Deref for ResourceHandle<R> {
    type Target = R;
    
    fn deref(&self) -> &Self::Target {
        self.get()
    }
}
```

### `ResourceMetadata`

Metadata for resources.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceMetadata {
    /// Resource name
    pub name: Option<String>,
    
    /// Resource description
    pub description: Option<String>,
    
    /// Resource version
    pub version: Version,
    
    /// Resource labels
    pub labels: HashMap<String, String>,
    
    /// Resource annotations
    pub annotations: HashMap<String, Value>,
    
    /// Creation timestamp
    pub created_at: DateTime<Utc>,
    
    /// Last modification timestamp
    pub updated_at: DateTime<Utc>,
    
    /// Owner information
    pub owner: Option<String>,
    
    /// Resource dependencies
    pub dependencies: Vec<ResourceId>,
    
    /// Resource tags for categorization
    pub tags: HashSet<String>,
    
    /// Custom metadata fields
    pub custom: HashMap<String, Value>,
}

impl Default for ResourceMetadata {
    fn default() -> Self {
        Self {
            name: None,
            description: None,
            version: Version::parse("1.0.0").unwrap(),
            labels: HashMap::new(),
            annotations: HashMap::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            owner: None,
            dependencies: Vec::new(),
            tags: HashSet::new(),
            custom: HashMap::new(),
        }
    }
}
```

### `ResourceCapabilities`

Resource capabilities descriptor.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceCapabilities {
    /// Can be pooled
    pub poolable: bool,
    
    /// Can be cloned
    pub cloneable: bool,
    
    /// Can be refreshed
    pub refreshable: bool,
    
    /// Supports health checks
    pub health_checkable: bool,
    
    /// Supports metrics
    pub observable: bool,
    
    /// Can be migrated
    pub migratable: bool,
    
    /// Supports transactions
    pub transactional: bool,
    
    /// Thread-safe
    pub thread_safe: bool,
    
    /// Supports streaming
    pub streaming: bool,
    
    /// Custom capabilities
    pub custom: HashSet<String>,
}

impl Default for ResourceCapabilities {
    fn default() -> Self {
        Self {
            poolable: false,
            cloneable: false,
            refreshable: false,
            health_checkable: true,
            observable: true,
            migratable: false,
            transactional: false,
            thread_safe: true,
            streaming: false,
            custom: HashSet::new(),
        }
    }
}
```

## Lifecycle Types

### `LifecycleState`

Resource lifecycle states.

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LifecycleState {
    /// Just created, not initialized
    Created,
    
    /// Being initialized
    Initializing,
    
    /// Ready for use
    Ready,
    
    /// Currently in use
    InUse {
        since: Instant,
        by: Option<String>,
    },
    
    /// Not in use but available
    Idle {
        since: Instant,
    },
    
    /// Being refreshed
    Refreshing,
    
    /// Being drained (preparing for removal)
    Draining {
        reason: String,
    },
    
    /// Being cleaned up
    Cleanup,
    
    /// Terminated
    Terminated {
        at: Instant,
        reason: String,
    },
    
    /// Failed state
    Failed {
        error: String,
        at: Instant,
        recoverable: bool,
    },
}

impl LifecycleState {
    /// Check if resource is available for use
    pub fn is_available(&self) -> bool {
        matches!(self, LifecycleState::Ready | LifecycleState::Idle { .. })
    }
    
    /// Check if resource is in terminal state
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            LifecycleState::Terminated { .. } | LifecycleState::Failed { recoverable: false, .. }
        )
    }
}
```

### `LifecycleEvent`

Lifecycle transition events.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LifecycleEvent {
    Initialize,
    Ready,
    Acquire { by: Option<String> },
    Release,
    Refresh,
    Drain { reason: String },
    Cleanup,
    Fail { error: String },
    Recover,
    Terminate { reason: String },
}
```

## Pool Types

### `PooledResource<R>`

Wrapper for pooled resources.

```rust
pub struct PooledResource<R: Resource> {
    resource: Arc<R>,
    pool: Weak<Pool<R>>,
    acquired_at: Instant,
    acquisition_id: Uuid,
    state: Arc<RwLock<PooledState>>,
}

#[derive(Debug, Clone)]
pub struct PooledState {
    pub in_use: bool,
    pub last_used: Instant,
    pub use_count: u64,
    pub health_status: HealthStatus,
    pub validation_result: Option<ValidationResult>,
}

impl<R: Resource> PooledResource<R> {
    /// Get the inner resource
    pub fn get(&self) -> &R {
        &self.resource
    }
    
    /// Check if still valid
    pub async fn is_valid(&self) -> bool {
        if let Some(pool) = self.pool.upgrade() {
            pool.validate_resource(&self.resource).await.is_ok()
        } else {
            false
        }
    }
    
    /// Get usage statistics
    pub fn stats(&self) -> PooledResourceStats {
        let state = self.state.read().unwrap();
        PooledResourceStats {
            acquired_at: self.acquired_at,
            in_use_duration: self.acquired_at.elapsed(),
            total_use_count: state.use_count,
            health_status: state.health_status.clone(),
        }
    }
}
```

### `PoolConfig`

Pool configuration.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolConfig {
    /// Minimum pool size
    pub min_size: usize,
    
    /// Maximum pool size
    pub max_size: usize,
    
    /// Maximum idle resources
    pub max_idle: usize,
    
    /// Idle timeout
    pub idle_timeout: Duration,
    
    /// Acquisition timeout
    pub acquisition_timeout: Duration,
    
    /// Validation interval
    pub validation_interval: Duration,
    
    /// Connection test query (for databases)
    pub test_query: Option<String>,
    
    /// LIFO or FIFO
    pub strategy: PoolStrategy,
    
    /// Preload on startup
    pub preload: bool,
    
    /// Retry configuration
    pub retry: RetryConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PoolStrategy {
    /// Last In, First Out (better for connection reuse)
    Lifo,
    /// First In, First Out (better for fairness)
    Fifo,
    /// Random selection
    Random,
    /// Least recently used
    Lru,
    /// Least frequently used
    Lfu,
}
```

## Health Types

### `HealthStatus`

Health status information.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HealthStatus {
    /// Resource is healthy
    Healthy {
        message: Option<String>,
        latency: Duration,
        details: HashMap<String, Value>,
    },
    
    /// Resource is degraded but operational
    Degraded {
        reason: String,
        latency: Duration,
        affected_features: Vec<String>,
        details: HashMap<String, Value>,
    },
    
    /// Resource is unhealthy
    Unhealthy {
        error: String,
        since: Instant,
        last_success: Option<Instant>,
        details: HashMap<String, Value>,
    },
    
    /// Health status unknown
    Unknown {
        reason: Option<String>,
    },
}

impl HealthStatus {
    pub fn is_healthy(&self) -> bool {
        matches!(self, HealthStatus::Healthy { .. })
    }
    
    pub fn is_operational(&self) -> bool {
        matches!(self, HealthStatus::Healthy { .. } | HealthStatus::Degraded { .. })
    }
    
    pub fn severity(&self) -> HealthSeverity {
        match self {
            HealthStatus::Healthy { .. } => HealthSeverity::Ok,
            HealthStatus::Degraded { .. } => HealthSeverity::Warning,
            HealthStatus::Unhealthy { .. } => HealthSeverity::Critical,
            HealthStatus::Unknown { .. } => HealthSeverity::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthSeverity {
    Ok,
    Warning,
    Critical,
    Unknown,
}
```

## Metrics Types

### `ResourceMetrics`

Resource metrics data.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceMetrics {
    pub resource_id: ResourceId,
    pub resource_type: String,
    pub created_at: Instant,
    pub last_used: Instant,
    
    // Usage metrics
    pub usage_count: u64,
    pub error_count: u64,
    pub success_rate: f64,
    
    // Performance metrics
    pub avg_latency: Duration,
    pub p50_latency: Duration,
    pub p95_latency: Duration,
    pub p99_latency: Duration,
    pub max_latency: Duration,
    
    // Resource metrics
    pub memory_usage: usize,
    pub cpu_usage: f32,
    pub active_operations: usize,
    pub queued_operations: usize,
    
    // Network metrics (if applicable)
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub connections_active: usize,
    
    // Custom metrics
    pub custom_metrics: HashMap<String, MetricValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MetricValue {
    Counter(u64),
    Gauge(f64),
    Histogram(Vec<f64>),
    Summary {
        count: u64,
        sum: f64,
        quantiles: HashMap<String, f64>,
    },
}
```

## Configuration Types

### `ResourceConfig`

Base configuration for resources.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceConfig {
    /// Resource name
    pub name: Option<String>,
    
    /// Resource type
    pub resource_type: String,
    
    /// Initialization timeout
    pub init_timeout: Duration,
    
    /// Cleanup timeout
    pub cleanup_timeout: Duration,
    
    /// Health check configuration
    pub health_check: HealthCheckConfig,
    
    /// Retry configuration
    pub retry: RetryConfig,
    
    /// Resource-specific configuration
    pub config: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    pub max_attempts: u32,
    pub initial_delay: Duration,
    pub max_delay: Duration,
    pub exponential_base: f64,
    pub jitter: bool,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(30),
            exponential_base: 2.0,
            jitter: true,
        }
    }
}
```

## Scope Types

### `ResourceScope`

Resource scope definition.

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ResourceScope {
    /// Global scope
    Global,
    
    /// Tenant scope
    Tenant {
        tenant_id: TenantId,
    },
    
    /// Workflow scope
    Workflow {
        workflow_id: WorkflowId,
        tenant_id: Option<TenantId>,
    },
    
    /// Action scope
    Action {
        action_id: ActionId,
        workflow_id: Option<WorkflowId>,
        tenant_id: Option<TenantId>,
    },
    
    /// Custom scope
    Custom {
        scope_type: String,
        scope_id: String,
        parent: Option<Box<ResourceScope>>,
    },
}

// Type aliases for IDs
pub type TenantId = String;
pub type WorkflowId = String;
pub type ActionId = String;
```

## Security Types

### `Credentials`

Credential types.

```rust
#[derive(Debug, Clone)]
pub enum Credentials {
    /// Username and password
    UserPassword {
        username: String,
        password: SecureString,
    },
    
    /// API key
    ApiKey(SecureString),
    
    /// Bearer token
    BearerToken(SecureString),
    
    /// OAuth2 credentials
    OAuth2 {
        client_id: String,
        client_secret: SecureString,
        token: Option<OAuth2Token>,
    },
    
    /// AWS credentials
    Aws {
        access_key_id: String,
        secret_access_key: SecureString,
        session_token: Option<SecureString>,
    },
    
    /// Certificate
    Certificate {
        cert: Vec<u8>,
        key: SecureString,
        ca: Option<Vec<u8>>,
    },
    
    /// Custom credentials
    Custom(HashMap<String, SecureString>),
}

/// Secure string that zeroes memory on drop
pub struct SecureString(Zeroizing<String>);

impl SecureString {
    pub fn new(s: String) -> Self {
        Self(Zeroizing::new(s))
    }
    
    pub fn expose(&self) -> &str {
        &self.0
    }
}
```

## Utility Types

### `Duration` Extensions

```rust
pub trait DurationExt {
    fn from_seconds(secs: f64) -> Duration;
    fn from_minutes(mins: f64) -> Duration;
    fn from_hours(hours: f64) -> Duration;
    fn from_days(days: f64) -> Duration;
    
    fn as_seconds_f64(&self) -> f64;
    fn as_minutes_f64(&self) -> f64;
    fn as_hours_f64(&self) -> f64;
    fn as_days_f64(&self) -> f64;
}

impl DurationExt for Duration {
    fn from_seconds(secs: f64) -> Duration {
        Duration::from_secs_f64(secs)
    }
    
    fn from_minutes(mins: f64) -> Duration {
        Duration::from_secs_f64(mins * 60.0)
    }
    
    fn from_hours(hours: f64) -> Duration {
        Duration::from_secs_f64(hours * 3600.0)
    }
    
    fn from_days(days: f64) -> Duration {
        Duration::from_secs_f64(days * 86400.0)
    }
    
    fn as_minutes_f64(&self) -> f64 {
        self.as_secs_f64() / 60.0
    }
    
    fn as_hours_f64(&self) -> f64 {
        self.as_secs_f64() / 3600.0
    }
    
    fn as_days_f64(&self) -> f64 {
        self.as_secs_f64() / 86400.0
    }
}
```

### `Result` Extensions

```rust
pub trait ResultExt<T, E> {
    fn context<C>(self, context: C) -> Result<T, Error>
    where
        C: Display + Send + Sync + 'static;
    
    fn with_context<F, C>(self, f: F) -> Result<T, Error>
    where
        F: FnOnce() -> C,
        C: Display + Send + Sync + 'static;
}

impl<T, E> ResultExt<T, E> for Result<T, E>
where
    E: Into<Error>,
{
    fn context<C>(self, context: C) -> Result<T, Error>
    where
        C: Display + Send + Sync + 'static,
    {
        self.map_err(|e| {
            let base_error: Error = e.into();
            Error::Context {
                message: context.to_string(),
                source: Box::new(base_error),
            }
        })
    }
    
    fn with_context<F, C>(self, f: F) -> Result<T, Error>
    where
        F: FnOnce() -> C,
        C: Display + Send + Sync + 'static,
    {
        self.map_err(|e| {
            let base_error: Error = e.into();
            Error::Context {
                message: f().to_string(),
                source: Box::new(base_error),
            }
        })
    }
}
```

## Type Aliases

Common type aliases used throughout the system:

```rust
/// Result type with Error as error type
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Boxed future
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Boxed stream
pub type BoxStream<'a, T> = Pin<Box<dyn Stream<Item = T> + Send + 'a>>;

/// Resource factory function
pub type FactoryFn<R> = Box<dyn Fn() -> BoxFuture<'static, Result<R>> + Send + Sync>;

/// Health check function
pub type HealthCheckFn = Box<dyn Fn() -> BoxFuture<'static, Result<HealthStatus>> + Send + Sync>;

/// Callback function
pub type Callback<T> = Box<dyn Fn(T) + Send + Sync>;

/// Event handler
pub type EventHandler = Box<dyn Fn(Event) -> BoxFuture<'static, Result<()>> + Send + Sync>;
```

## Best Practices

1. **Use strong types** - Prefer specific types over primitives
2. **Implement Display** - For better error messages
3. **Derive common traits** - Debug, Clone, Serialize, Deserialize
4. **Use newtype pattern** - For domain-specific types
5. **Provide builders** - For complex types
6. **Document invariants** - What valid states are
7. **Use NonZero types** - Where appropriate
8. **Implement From/TryFrom** - For type conversions
9. **Keep types small** - Prefer composition over large structs
10. **Version types** - When they might change
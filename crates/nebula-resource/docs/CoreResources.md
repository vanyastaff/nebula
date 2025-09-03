---
title: Core Resources
tags: [nebula, nebula-resource, docs]
status: draft
created: 2025-08-17
---

# Core Resources Reference

## Overview

Nebula Resource provides a set of built-in resource implementations for common use cases. These resources are production-ready and can be used directly or extended for custom needs.

## Database Resources

### `PostgresResource`

PostgreSQL database connection resource.

```rust
pub struct PostgresResource {
    config: PostgresConfig,
    connection: Arc<RwLock<Option<PgConnection>>>,
    pool: Option<PgPool>,
    metrics: Arc<ResourceMetrics>,
}
```

#### Configuration

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct PostgresConfig {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub password: String,
    pub ssl_mode: SslMode,
    pub pool_size: Option<u32>,
    pub connection_timeout: Duration,
    pub statement_timeout: Duration,
    pub application_name: Option<String>,
}
```

#### Usage Example

```rust
use nebula_resource::resources::PostgresResource;

let config = PostgresConfig {
    host: "localhost".to_string(),
    port: 5432,
    database: "mydb".to_string(),
    username: "user".to_string(),
    password: "pass".to_string(),
    ssl_mode: SslMode::Prefer,
    pool_size: Some(10),
    connection_timeout: Duration::from_secs(10),
    statement_timeout: Duration::from_secs(30),
    application_name: Some("nebula_app".to_string()),
};

let pg_resource = PostgresResource::new(config);
manager.register_resource(pg_resource).await?;
```

#### Methods

- `execute`: Execute a SQL query
- `fetch`: Fetch results from a query
- `transaction`: Start a transaction
- `prepare`: Prepare a statement
- `pool`: Get the underlying connection pool

---

### `MySqlResource`

MySQL/MariaDB database connection resource.

```rust
pub struct MySqlResource {
    config: MySqlConfig,
    connection: Arc<RwLock<Option<MySqlConnection>>>,
    pool: Option<MySqlPool>,
}
```

#### Configuration

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct MySqlConfig {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub password: String,
    pub ssl_mode: MySqlSslMode,
    pub charset: String,
    pub collation: Option<String>,
    pub timezone: Option<String>,
}
```

---

### `MongoResource`

MongoDB connection resource.

```rust
pub struct MongoResource {
    config: MongoConfig,
    client: Arc<RwLock<Option<MongoClient>>>,
    database: Option<Database>,
}
```

#### Configuration

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct MongoConfig {
    pub connection_string: Option<String>,
    pub hosts: Vec<String>,
    pub database: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub replica_set: Option<String>,
    pub auth_source: Option<String>,
    pub tls: bool,
}
```

## Cache Resources

### `RedisResource`

Redis cache resource with support for clustering.

```rust
pub struct RedisResource {
    config: RedisConfig,
    client: Arc<RwLock<RedisClient>>,
    connection_manager: Option<ConnectionManager>,
}
```

#### Configuration

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct RedisConfig {
    pub mode: RedisMode,
    pub host: String,
    pub port: u16,
    pub password: Option<String>,
    pub database: u8,
    pub cluster_nodes: Vec<String>,
    pub sentinel_master: Option<String>,
    pub connection_timeout: Duration,
    pub response_timeout: Duration,
    pub max_connections: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub enum RedisMode {
    Standalone,
    Cluster,
    Sentinel,
}
```

#### Usage Example

```rust
use nebula_resource::resources::{RedisResource, RedisConfig, RedisMode};

let config = RedisConfig {
    mode: RedisMode::Standalone,
    host: "localhost".to_string(),
    port: 6379,
    password: None,
    database: 0,
    cluster_nodes: vec![],
    sentinel_master: None,
    connection_timeout: Duration::from_secs(5),
    response_timeout: Duration::from_secs(5),
    max_connections: 50,
};

let redis = RedisResource::new(config);

// Use the resource
redis.set("key", "value", Some(Duration::from_secs(60))).await?;
let value: String = redis.get("key").await?;
```

#### Methods

- `get`: Get a value
- `set`: Set a value with optional TTL
- `delete`: Delete keys
- `exists`: Check if key exists
- `expire`: Set expiration
- `incr`/`decr`: Increment/decrement
- `hget`/`hset`: Hash operations
- `lpush`/`rpush`: List operations
- `sadd`/`srem`: Set operations

---

### `MemcachedResource`

Memcached cache resource.

```rust
pub struct MemcachedResource {
    config: MemcachedConfig,
    client: Arc<RwLock<MemcachedClient>>,
}
```

## HTTP Client Resources

### `HttpClientResource`

HTTP client with connection pooling and retry logic.

```rust
pub struct HttpClientResource {
    config: HttpClientConfig,
    client: Arc<Client>,
    metrics: Arc<HttpMetrics>,
}
```

#### Configuration

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct HttpClientConfig {
    pub timeout: Duration,
    pub connect_timeout: Duration,
    pub pool_idle_timeout: Duration,
    pub pool_max_idle_per_host: usize,
    pub max_retries: u32,
    pub retry_delay: Duration,
    pub user_agent: Option<String>,
    pub proxy: Option<ProxyConfig>,
    pub tls_config: Option<TlsConfig>,
}
```

#### Usage Example

```rust
use nebula_resource::resources::HttpClientResource;

let client = HttpClientResource::new(HttpClientConfig::default());

// Make requests
let response = client
    .get("https://api.example.com/data")
    .header("Authorization", "Bearer token")
    .query(&[("page", "1")])
    .send()
    .await?;

let data: MyData = response.json().await?;
```

#### Methods

- `get`: GET request
- `post`: POST request
- `put`: PUT request
- `delete`: DELETE request
- `patch`: PATCH request
- `request`: Custom request

---

### `GraphQLClientResource`

GraphQL client resource.

```rust
pub struct GraphQLClientResource {
    config: GraphQLConfig,
    client: Arc<GraphQLClient>,
}
```

## Message Queue Resources

### `KafkaResource`

Apache Kafka producer/consumer resource.

```rust
pub struct KafkaResource {
    config: KafkaConfig,
    producer: Arc<RwLock<Option<Producer>>>,
    consumer: Arc<RwLock<Option<Consumer>>>,
}
```

#### Configuration

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct KafkaConfig {
    pub brokers: Vec<String>,
    pub client_id: String,
    pub group_id: Option<String>,
    pub topics: Vec<String>,
    pub compression: CompressionType,
    pub batch_size: usize,
    pub linger_ms: u64,
    pub acks: AckMode,
    pub retries: u32,
}
```

---

### `RabbitMQResource`

RabbitMQ connection resource.

```rust
pub struct RabbitMQResource {
    config: RabbitMQConfig,
    connection: Arc<RwLock<Option<Connection>>>,
    channel: Arc<RwLock<Option<Channel>>>,
}
```

---

### `NatsResource`

NATS messaging resource.

```rust
pub struct NatsResource {
    config: NatsConfig,
    connection: Arc<RwLock<Option<NatsConnection>>>,
}
```

## Storage Resources

### `S3Resource`

AWS S3 or S3-compatible storage resource.

```rust
pub struct S3Resource {
    config: S3Config,
    client: Arc<S3Client>,
}
```

#### Configuration

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct S3Config {
    pub endpoint: Option<String>,
    pub region: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub bucket: String,
    pub path_prefix: Option<String>,
    pub use_path_style: bool,
}
```

#### Methods

- `get_object`: Download object
- `put_object`: Upload object
- `delete_object`: Delete object
- `list_objects`: List objects
- `create_bucket`: Create bucket
- `generate_presigned_url`: Generate presigned URL

---

### `AzureBlobResource`

Azure Blob Storage resource.

```rust
pub struct AzureBlobResource {
    config: AzureBlobConfig,
    client: Arc<BlobServiceClient>,
}
```

---

### `GcsResource`

Google Cloud Storage resource.

```rust
pub struct GcsResource {
    config: GcsConfig,
    client: Arc<GcsClient>,
}
```

## System Resources

### `FileSystemResource`

Local file system resource with monitoring.

```rust
pub struct FileSystemResource {
    config: FileSystemConfig,
    watcher: Option<FileWatcher>,
}
```

#### Configuration

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct FileSystemConfig {
    pub base_path: PathBuf,
    pub watch: bool,
    pub max_file_size: Option<usize>,
    pub allowed_extensions: Option<Vec<String>>,
    pub temp_dir: Option<PathBuf>,
}
```

---

### `ProcessResource`

System process management resource.

```rust
pub struct ProcessResource {
    config: ProcessConfig,
    process: Arc<RwLock<Option<Child>>>,
}
```

#### Configuration

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct ProcessConfig {
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub working_dir: Option<PathBuf>,
    pub stdout: ProcessOutput,
    pub stderr: ProcessOutput,
    pub restart_policy: RestartPolicy,
}
```

---

### `ThreadPoolResource`

Managed thread pool resource.

```rust
pub struct ThreadPoolResource {
    config: ThreadPoolConfig,
    pool: Arc<ThreadPool>,
}
```

## Monitoring Resources

### `MetricsCollectorResource`

Metrics collection and aggregation resource.

```rust
pub struct MetricsCollectorResource {
    config: MetricsConfig,
    registry: Arc<Registry>,
    exporters: Vec<Box<dyn MetricsExporter>>,
}
```

---

### `LoggerResource`

Structured logging resource.

```rust
pub struct LoggerResource {
    config: LoggerConfig,
    logger: Arc<Logger>,
}
```

#### Configuration

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct LoggerConfig {
    pub level: LogLevel,
    pub format: LogFormat,
    pub outputs: Vec<LogOutput>,
    pub context_fields: HashMap<String, String>,
}
```

---

### `TracerResource`

Distributed tracing resource.

```rust
pub struct TracerResource {
    config: TracerConfig,
    tracer: Arc<Tracer>,
}
```

## Network Resources

### `TcpListenerResource`

TCP server resource.

```rust
pub struct TcpListenerResource {
    config: TcpListenerConfig,
    listener: Arc<RwLock<Option<TcpListener>>>,
}
```

---

### `UdpSocketResource`

UDP socket resource.

```rust
pub struct UdpSocketResource {
    config: UdpSocketConfig,
    socket: Arc<UdpSocket>,
}
```

---

### `WebSocketResource`

WebSocket client/server resource.

```rust
pub struct WebSocketResource {
    config: WebSocketConfig,
    connection: Arc<RwLock<Option<WebSocketConnection>>>,
}
```

## Security Resources

### `CredentialManagerResource`

Credential management resource.

```rust
pub struct CredentialManagerResource {
    config: CredentialConfig,
    store: Arc<dyn CredentialStore>,
}
```

#### Methods

- `get_credential`: Retrieve credential
- `store_credential`: Store credential
- `rotate_credential`: Rotate credential
- `delete_credential`: Delete credential

---

### `EncryptionResource`

Encryption/decryption resource.

```rust
pub struct EncryptionResource {
    config: EncryptionConfig,
    cipher: Arc<dyn Cipher>,
}
```

## Custom Resource Example

### Creating a Custom Resource

```rust
use nebula_resource::prelude::*;
use async_trait::async_trait;

pub struct CustomResource {
    id: ResourceId,
    config: CustomConfig,
    state: Arc<RwLock<CustomState>>,
}

#[async_trait]
impl Resource for CustomResource {
    fn id(&self) -> ResourceId {
        self.id.clone()
    }
    
    fn resource_type(&self) -> &str {
        "custom_resource"
    }
    
    async fn initialize(&self) -> Result<()> {
        let mut state = self.state.write().await;
        // Initialize custom resource
        state.initialized = true;
        Ok(())
    }
    
    async fn cleanup(&self) -> Result<()> {
        let mut state = self.state.write().await;
        // Cleanup custom resource
        state.initialized = false;
        Ok(())
    }
    
    async fn health_check(&self) -> Result<HealthStatus> {
        let state = self.state.read().await;
        if state.initialized {
            Ok(HealthStatus::Healthy {
                message: Some("Custom resource is healthy".to_string()),
                latency: Duration::from_millis(1),
            })
        } else {
            Ok(HealthStatus::Unhealthy {
                error: "Not initialized".to_string(),
                since: Instant::now(),
            })
        }
    }
    
    async fn metrics(&self) -> Option<ResourceMetrics> {
        Some(ResourceMetrics {
            resource_id: self.id.clone(),
            resource_type: self.resource_type().to_string(),
            created_at: Instant::now(),
            last_used: Instant::now(),
            usage_count: 0,
            error_count: 0,
            success_rate: 1.0,
            avg_latency: Duration::from_millis(1),
            p99_latency: Duration::from_millis(2),
            memory_usage: 1024,
            active_operations: 0,
            custom_metrics: HashMap::new(),
        })
    }
    
    fn validate(&self) -> Result<()> {
        // Validate configuration
        Ok(())
    }
}
```

## Resource Lifecycle

All core resources follow the standard lifecycle:

1. **Creation** - Resource is instantiated with configuration
2. **Registration** - Resource is registered with the manager
3. **Initialization** - `initialize()` is called
4. **Ready** - Resource is ready for use
5. **Active** - Resource is being used
6. **Idle** - Resource is not being used
7. **Cleanup** - `cleanup()` is called
8. **Terminated** - Resource is removed

## Best Practices

1. **Use appropriate resource type** - Choose the right built-in resource
2. **Configure properly** - Set appropriate timeouts and limits
3. **Handle errors** - All resources can fail
4. **Monitor health** - Use health checks
5. **Track metrics** - Monitor resource usage
6. **Clean up properly** - Always call cleanup
7. **Use pools** - For frequently used resources
8. **Set timeouts** - Prevent hanging operations
9. **Validate configuration** - Check config before use
10. **Extend carefully** - When creating custom resources
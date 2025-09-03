---
title: Configuration
tags: [nebula, nebula-resource, docs]
status: draft
created: 2025-08-17
---

# Configuration Reference

## Overview

Nebula Resource supports configuration through multiple sources:

- YAML/JSON files
- Environment variables
- Programmatic configuration
- Runtime updates

## Configuration Structure

### Complete Configuration Example

```yaml
# nebula-resource.yaml
resource_manager:
  # Global settings
  global:
    max_resources: 1000
    default_timeout: 30s
    enable_metrics: true
    enable_tracing: true
    
  # Registry configuration
  registry:
    type: in_memory  # in_memory | redis | etcd
    cache_ttl: 60s
    sync_interval: 10s
    
  # Lifecycle settings
  lifecycle:
    auto_initialize: true
    auto_cleanup: true
    max_retry_attempts: 3
    retry_delay: 1s
    
  # Health check configuration
  health:
    enabled: true
    interval: 30s
    timeout: 5s
    failure_threshold: 3
    success_threshold: 2
    
  # Metrics configuration
  metrics:
    enabled: true
    export_interval: 10s
    retention: 24h
    exporters:
      - type: prometheus
        endpoint: /metrics
        port: 9090
      - type: statsd
        host: localhost
        port: 8125
        
# Pool configuration
pools:
  default:
    min_size: 5
    max_size: 20
    max_idle: 10
    idle_timeout: 5m
    acquisition_timeout: 30s
    validation_interval: 1m
    preload: true
    
  database:
    min_size: 10
    max_size: 50
    max_idle: 20
    idle_timeout: 10m
    acquisition_timeout: 10s
    validation_interval: 30s
    connection_test_query: "SELECT 1"
    
  http_client:
    min_size: 2
    max_size: 100
    max_idle: 50
    idle_timeout: 30s
    acquisition_timeout: 5s
    
# Resource-specific configurations
resources:
  database:
    type: postgresql
    config:
      host: ${DB_HOST:localhost}
      port: ${DB_PORT:5432}
      database: ${DB_NAME:nebula}
      username: ${DB_USER:admin}
      password: ${DB_PASSWORD}
      ssl_mode: ${DB_SSL_MODE:prefer}
      max_connections: 100
      connection_timeout: 10s
      statement_timeout: 30s
      
  cache:
    type: redis
    config:
      host: ${REDIS_HOST:localhost}
      port: ${REDIS_PORT:6379}
      password: ${REDIS_PASSWORD}
      database: ${REDIS_DB:0}
      max_connections: 50
      connection_timeout: 5s
      max_retries: 3
      
  message_queue:
    type: kafka
    config:
      brokers:
        - ${KAFKA_BROKER_1:localhost:9092}
        - ${KAFKA_BROKER_2:localhost:9093}
      client_id: nebula_resource
      compression: snappy
      batch_size: 1000
      linger_ms: 10
      
# Quarantine configuration
quarantine:
  enabled: true
  error_threshold: 0.5
  error_window: 60s
  health_check_failures: 3
  auto_recovery: true
  initial_recovery_delay: 30s
  max_recovery_attempts: 5
  backoff_multiplier: 2.0
  max_backoff: 5m
  
# Scoping configuration
scoping:
  default_scope: workflow
  resolution_strategy: most_specific
  lifecycle:
    action:
      auto_cleanup: true
      max_lifetime: 5m
      max_idle: 1m
    workflow:
      auto_cleanup: true
      max_lifetime: 1h
      max_idle: 10m
    tenant:
      auto_cleanup: false
      
# Migration configuration
migration:
  strategy: shortest_path
  auto_migrate:
    enabled: true
    on_load: true
    lazy: false
  backup:
    enabled: true
    retention_days: 30
    
# Security configuration
security:
  encryption:
    enabled: true
    algorithm: AES-256-GCM
    key_rotation_interval: 30d
  authentication:
    type: jwt
    secret: ${JWT_SECRET}
    expiration: 1h
  authorization:
    enabled: true
    policy_file: policies.yaml
    
# Logging configuration
logging:
  level: ${LOG_LEVEL:info}
  format: json
  output:
    - type: console
      level: info
    - type: file
      path: /var/log/nebula/resource.log
      rotation: daily
      retention: 7
    - type: syslog
      host: localhost
      port: 514
```

## Configuration Sections

### Global Configuration

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct GlobalConfig {
    /// Maximum number of resources
    pub max_resources: usize,
    
    /// Default timeout for operations
    pub default_timeout: Duration,
    
    /// Enable metrics collection
    pub enable_metrics: bool,
    
    /// Enable distributed tracing
    pub enable_tracing: bool,
    
    /// Resource naming pattern
    pub naming_pattern: Option<String>,
    
    /// Default retry policy
    pub retry_policy: RetryPolicy,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub initial_delay: Duration,
    pub max_delay: Duration,
    pub exponential_base: f64,
    pub jitter: bool,
}
```

### Pool Configuration

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct PoolConfig {
    /// Minimum pool size
    pub min_size: usize,
    
    /// Maximum pool size
    pub max_size: usize,
    
    /// Maximum idle resources
    pub max_idle: usize,
    
    /// Idle timeout before resource removal
    pub idle_timeout: Duration,
    
    /// Timeout for acquiring a resource
    pub acquisition_timeout: Duration,
    
    /// Interval for validating idle resources
    pub validation_interval: Duration,
    
    /// Preload resources on startup
    pub preload: bool,
    
    /// LIFO or FIFO strategy
    pub strategy: PoolStrategy,
    
    /// Scaling policy
    pub scaling: ScalingPolicy,
}

#[derive(Debug, Clone, Deserialize)]
pub enum PoolStrategy {
    Lifo,  // Last In, First Out (better for connection reuse)
    Fifo,  // First In, First Out (better for fairness)
    Random, // Random selection
    LeastUsed, // Select least used resource
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScalingPolicy {
    pub enabled: bool,
    pub scale_up_threshold: f32,
    pub scale_down_threshold: f32,
    pub scale_factor: f32,
    pub cooldown: Duration,
}
```

### Health Check Configuration

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct HealthCheckConfig {
    /// Enable health checks
    pub enabled: bool,
    
    /// Check interval
    pub interval: Duration,
    
    /// Check timeout
    pub timeout: Duration,
    
    /// Consecutive failures before marking unhealthy
    pub failure_threshold: u32,
    
    /// Consecutive successes before marking healthy
    pub success_threshold: u32,
    
    /// Health check implementation
    pub checker: HealthCheckerConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub enum HealthCheckerConfig {
    Default,
    Http {
        endpoint: String,
        expected_status: u16,
    },
    Tcp {
        port: u16,
    },
    Command {
        command: String,
        args: Vec<String>,
        expected_exit_code: i32,
    },
    Custom {
        class: String,
    },
}
```

### Metrics Configuration

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct MetricsConfig {
    /// Enable metrics collection
    pub enabled: bool,
    
    /// Export interval
    pub export_interval: Duration,
    
    /// Metrics retention period
    pub retention: Duration,
    
    /// Histogram buckets
    pub histogram_buckets: Vec<f64>,
    
    /// Metrics exporters
    pub exporters: Vec<ExporterConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub enum ExporterConfig {
    Prometheus {
        endpoint: String,
        port: u16,
        namespace: String,
    },
    StatsD {
        host: String,
        port: u16,
        prefix: String,
    },
    OpenTelemetry {
        endpoint: String,
        protocol: OtlpProtocol,
    },
    CloudWatch {
        region: String,
        namespace: String,
    },
}
```

## Environment Variables

All configuration values support environment variable substitution:

```yaml
database:
  host: ${DB_HOST:localhost}  # Use DB_HOST env var, default to localhost
  port: ${DB_PORT:5432}        # Use DB_PORT env var, default to 5432
  password: ${DB_PASSWORD}     # Use DB_PASSWORD env var, required
```

### Standard Environment Variables

|Variable|Description|Default|
|---|---|---|
|`NEBULA_CONFIG_PATH`|Path to configuration file|`./nebula-resource.yaml`|
|`NEBULA_ENV`|Environment (dev/staging/prod)|`dev`|
|`NEBULA_LOG_LEVEL`|Logging level|`info`|
|`NEBULA_METRICS_ENABLED`|Enable metrics|`true`|
|`NEBULA_HEALTH_ENABLED`|Enable health checks|`true`|
|`NEBULA_POOL_MIN_SIZE`|Default min pool size|`5`|
|`NEBULA_POOL_MAX_SIZE`|Default max pool size|`20`|
|`NEBULA_TIMEOUT`|Default timeout|`30s`|

## Programmatic Configuration

### Using Builder Pattern

```rust
use nebula_resource::config::*;

let config = ResourceManagerConfigBuilder::default()
    .max_resources(1000)
    .default_timeout(Duration::from_secs(30))
    .enable_metrics(true)
    .pool_config(
        PoolConfigBuilder::default()
            .min_size(10)
            .max_size(50)
            .idle_timeout(Duration::from_secs(300))
            .build()?
    )
    .health_check_config(
        HealthCheckConfigBuilder::default()
            .enabled(true)
            .interval(Duration::from_secs(30))
            .failure_threshold(3)
            .build()?
    )
    .build()?;

let manager = ResourceManager::new(config);
```

### Direct Construction

```rust
let config = ResourceManagerConfig {
    global: GlobalConfig {
        max_resources: 1000,
        default_timeout: Duration::from_secs(30),
        enable_metrics: true,
        enable_tracing: false,
        naming_pattern: None,
        retry_policy: RetryPolicy::default(),
    },
    pools: HashMap::from([
        ("default".to_string(), PoolConfig::default()),
        ("database".to_string(), database_pool_config),
    ]),
    health: HealthCheckConfig::default(),
    metrics: MetricsConfig::default(),
    quarantine: QuarantineConfig::default(),
    scoping: ScopingConfig::default(),
};
```

## Configuration Validation

### Schema Validation

```rust
use nebula_resource::config::validate;

let config_str = fs::read_to_string("config.yaml")?;
let config: ResourceManagerConfig = serde_yaml::from_str(&config_str)?;

// Validate configuration
validate::validate_config(&config)?;
```

### Custom Validators

```rust
use nebula_resource::config::{ConfigValidator, ValidationError};

struct CustomValidator;

impl ConfigValidator for CustomValidator {
    fn validate(&self, config: &ResourceManagerConfig) -> Result<(), ValidationError> {
        // Custom validation logic
        if config.global.max_resources < config.pools.len() {
            return Err(ValidationError::Invalid(
                "max_resources must be >= number of pools".to_string()
            ));
        }
        Ok(())
    }
}

// Register validator
config::register_validator(Box::new(CustomValidator));
```

## Runtime Configuration Updates

### Hot Reload

```rust
use nebula_resource::config::ConfigWatcher;

// Watch for configuration changes
let watcher = ConfigWatcher::new("config.yaml");

watcher.on_change(|new_config| {
    println!("Configuration updated: {:?}", new_config);
    // Apply new configuration
    manager.update_config(new_config)?;
    Ok(())
});

watcher.start().await?;
```

### Dynamic Updates

```rust
// Update specific configuration
manager.update_pool_config("database", PoolConfig {
    max_size: 100,  // Increase pool size
    ..existing_config
}).await?;

// Update health check interval
manager.update_health_check_interval(Duration::from_secs(60)).await?;
```

## Configuration Profiles

### Profile-based Configuration

```yaml
# base.yaml
defaults:
  timeout: 30s
  metrics: true

# dev.yaml
extends: base
database:
  host: localhost
  port: 5432

# prod.yaml
extends: base
database:
  host: prod-db.example.com
  port: 5432
  ssl_mode: require
```

### Loading Profiles

```rust
use nebula_resource::config::ProfileLoader;

let profile = env::var("NEBULA_PROFILE").unwrap_or("dev".to_string());
let config = ProfileLoader::load(&profile)?;
```

## Best Practices

1. **Use environment variables for secrets** - Never hardcode passwords
2. **Profile-based configuration** - Separate dev/staging/prod configs
3. **Validate on startup** - Catch configuration errors early
4. **Set reasonable defaults** - Make configuration optional where possible
5. **Document configuration** - Include examples and descriptions
6. **Version configuration** - Track changes over time
7. **Monitor configuration changes** - Log when configuration is updated
8. **Test configuration** - Unit test configuration loading and validation
9. **Use strong typing** - Leverage Rust's type system
10. **Keep configuration DRY** - Use inheritance and references

## Configuration Migration

When configuration schema changes:

```rust
use nebula_resource::config::ConfigMigration;

// Define migration from v1 to v2
struct ConfigMigrationV1ToV2;

impl ConfigMigration for ConfigMigrationV1ToV2 {
    fn from_version(&self) -> Version {
        Version::parse("1.0.0").unwrap()
    }
    
    fn to_version(&self) -> Version {
        Version::parse("2.0.0").unwrap()
    }
    
    fn migrate(&self, config: &mut Value) -> Result<()> {
        // Rename old fields
        if let Some(old_field) = config.get("old_field") {
            config["new_field"] = old_field.clone();
            config.as_object_mut().unwrap().remove("old_field");
        }
        
        // Add new fields with defaults
        config["new_feature"] = json!({
            "enabled": false,
            "config": {}
        });
        
        Ok(())
    }
}
```
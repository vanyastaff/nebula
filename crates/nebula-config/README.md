# Nebula Config

Production-ready configuration management for Rust applications, fully integrated with the Nebula ecosystem.

## Overview

`nebula-config` provides a flexible and extensible configuration management system with support for multiple sources, formats, validation, hot-reloading, and seamless integration with other Nebula crates.

## Key Features

### 🔧 Configuration Sources
- **File-based**: TOML, YAML, JSON configuration files
- **Environment Variables**: Automatic detection and parsing
- **Programmatic**: Set defaults and overrides in code
- **Composite**: Combine multiple sources with priority ordering

### 🌟 Ecosystem Integration
- **NebulaValue**: Native support for `nebula-value` types for dynamic configuration
- **NebulaError**: Unified error handling with `nebula-error`
- **Structured Logging**: Built-in logging with `nebula-log`
- **Type Safety**: Full Rust type system integration

### 📊 Advanced Features
- **Hot Reloading**: Automatic configuration updates without restarts
- **Validation**: Built-in and custom configuration validation
- **Path Queries**: Dot notation for nested configuration access
- **Merging**: Intelligent configuration source merging
- **Watching**: File system monitoring for configuration changes

## Quick Start

Add to your `Cargo.toml`:

```toml
[dependencies]
nebula-config = "0.1.0"
nebula-log = "0.1.0"  # For logging
tokio = { version = "1.0", features = ["full"] }
```

## Basic Usage

```rust
use nebula_config::prelude::*;

#[tokio::main]
async fn main() -> ConfigResult<()> {
    // Initialize logging
    nebula_log::auto_init()?;

    // Build configuration from multiple sources
    let config = ConfigBuilder::new()
        .with_source(ConfigSource::File("config.toml".into()))
        .with_source(ConfigSource::Env)
        .with_hot_reload(true)
        .build()
        .await?;

    // Get typed configuration
    let port: u16 = config.get("server.port").await?;
    let database_url: String = config.get("database.url").await?;

    info!(port = port, database_url = %database_url, "Configuration loaded");
    Ok(())
}
```

## Configuration Sources

### File-based Configuration

```rust
use nebula_config::prelude::*;

let config = ConfigBuilder::new()
    .with_source(ConfigSource::File("app.toml".into()))
    .with_source(ConfigSource::File("database.yaml".into()))
    .build()
    .await?;
```

**TOML Example (`app.toml`):**
```toml
[app]
name = "my-service"
port = 8080
debug = false

[database]
host = "localhost"
port = 5432
name = "mydb"

[features]
enabled = ["logging", "metrics"]
```

### Environment Variables

```rust
use nebula_config::prelude::*;

// Automatic environment variable detection
let config = ConfigBuilder::new()
    .with_source(ConfigSource::Env)
    .build()
    .await?;

// With prefix filtering
let config = ConfigBuilder::new()
    .with_source(ConfigSource::EnvWithPrefix("MYAPP".to_string()))
    .build()
    .await?;
```

**Environment Variables:**
```bash
MYAPP_SERVER_PORT=8080
MYAPP_DATABASE_HOST=localhost
MYAPP_FEATURES_DEBUG=true
```

### Composite Configuration

```rust
use nebula_config::prelude::*;

let config = ConfigBuilder::new()
    // Defaults (lowest priority)
    .with_defaults(serde_json::json!({
        "app": {
            "name": "default-app",
            "port": 3000
        }
    }))
    // Configuration file (medium priority)
    .with_source(ConfigSource::File("config.toml".into()))
    // Environment variables (highest priority)
    .with_source(ConfigSource::Env)
    .build()
    .await?;
```

## NebulaValue Integration

### Working with Dynamic Configuration

```rust
use nebula_config::prelude::*;

// Get configuration as NebulaValue
let app_config = config.get_value("app").await?;

// Set dynamic configuration
let dynamic_value = NebulaValue::Object({
    let mut obj = nebula_value::Object::new();
    obj.insert("feature_flags".to_string(), NebulaValue::Array(vec![
        NebulaValue::Text("feature_a".to_string().into()),
        NebulaValue::Text("feature_b".to_string().into()),
    ].into()));
    obj.insert("timeout".to_string(), NebulaValue::Integer(30));
    obj
});

config.set_value("runtime", dynamic_value).await?;
```

### Typed Configuration with NebulaValue

```rust
use nebula_config::prelude::*;

#[derive(serde::Serialize, serde::Deserialize)]
struct DatabaseConfig {
    host: String,
    port: u16,
    ssl: bool,
}

// Set typed configuration
let db_config = DatabaseConfig {
    host: "localhost".to_string(),
    port: 5432,
    ssl: true,
};
config.set_typed("database", &db_config).await?;

// Get typed configuration
let loaded_config: DatabaseConfig = config.get_typed("database").await?;
```

## Configuration Validation

### Built-in Validation

```rust
use nebula_config::prelude::*;

let config = ConfigBuilder::new()
    .with_source(ConfigSource::File("config.toml".into()))
    .with_validator(SchemaValidator::from_file("schema.json")?)
    .build()
    .await?;
```

### Custom Validation

```rust
use nebula_config::prelude::*;

async fn validate_database_config(config: &Config) -> ConfigResult<()> {
    let host: String = config.get("database.host").await?;
    if host.is_empty() {
        return Err(ConfigError::validation_with_field(
            "Database host cannot be empty",
            "database.host"
        ));
    }

    let port: u16 = config.get("database.port").await?;
    if port < 1024 {
        return Err(ConfigError::validation_with_field(
            "Database port must be >= 1024",
            "database.port"
        ));
    }

    Ok(())
}

// Use custom validation
validate_database_config(&config).await?;
```

## Hot Reloading

```rust
use nebula_config::prelude::*;

let config = ConfigBuilder::new()
    .with_source(ConfigSource::File("config.toml".into()))
    .with_hot_reload(true)
    .with_auto_reload_interval(Duration::from_secs(5))
    .build()
    .await?;

// Configuration will automatically reload when files change
// Access updated values normally
let current_port: u16 = config.get("server.port").await?;
```

## Error Handling

Full integration with `nebula-error`:

```rust
use nebula_config::prelude::*;

match config.get::<String>("missing.key").await {
    Ok(value) => info!(value = %value, "Configuration found"),
    Err(config_err) => {
        // ConfigError automatically converts to NebulaError
        let nebula_error: NebulaError = config_err.into();

        if nebula_error.is_not_found() {
            warn!("Configuration key not found, using default");
        } else if nebula_error.is_validation_error() {
            error!(error = %nebula_error, "Configuration validation failed");
        }
    }
}
```

## Structured Logging

All configuration operations include structured logging:

```rust
use nebula_config::prelude::*;

// Initialize logging once
nebula_log::auto_init()?;

// All operations automatically log with structured fields
let config = ConfigBuilder::new()
    .with_source(ConfigSource::File("config.toml".into()))
    .build()
    .await?;

// Logs include:
// - Configuration source loading status
// - Validation results
// - Hot reload events
// - Error details with context
```

## Advanced Usage

### Configuration Debugging

```rust
use nebula_config::prelude::*;

// Get flat configuration map for debugging
let flat_map = config.flatten().await;
for (key, value) in &flat_map {
    debug!(config_key = %key, config_value = %value, "Configuration entry");
}

// Check configuration sources
for source in config.sources() {
    info!(source = %source, priority = source.priority(), "Configuration source");
}
```

### Configuration Merging

```rust
use nebula_config::prelude::*;

// Merge additional configuration at runtime
let override_config = NebulaValue::Object({
    let mut obj = nebula_value::Object::new();
    obj.insert("debug".to_string(), NebulaValue::Bool(true));
    obj.insert("log_level".to_string(), NebulaValue::Text("debug".to_string().into()));
    obj
});

config.merge(override_config).await?;
```

### Configuration Watching

```rust
use nebula_config::prelude::*;

let config = ConfigBuilder::new()
    .with_source(ConfigSource::File("config.toml".into()))
    .with_watcher(FileWatcher::new())
    .build()
    .await?;

// Set up watch callback
config.watch(|event| async move {
    match event.event_type {
        ConfigWatchEventType::Changed => {
            info!(path = %event.path, "Configuration file changed");
        }
        ConfigWatchEventType::Deleted => {
            warn!(path = %event.path, "Configuration file deleted");
        }
    }
}).await?;
```

## Configuration Formats

### TOML
```toml
[server]
host = "0.0.0.0"
port = 8080

[database]
url = "postgresql://localhost/mydb"
pool_size = 10

[features]
enabled = ["auth", "metrics"]
```

### YAML
```yaml
server:
  host: "0.0.0.0"
  port: 8080

database:
  url: "postgresql://localhost/mydb"
  pool_size: 10

features:
  enabled:
    - auth
    - metrics
```

### JSON
```json
{
  "server": {
    "host": "0.0.0.0",
    "port": 8080
  },
  "database": {
    "url": "postgresql://localhost/mydb",
    "pool_size": 10
  },
  "features": {
    "enabled": ["auth", "metrics"]
  }
}
```

## Best Practices

1. **Use Type Safety**: Always use typed configuration access when possible
2. **Structured Logging**: Initialize `nebula-log` for comprehensive observability
3. **Validation**: Implement configuration validation for critical settings
4. **Environment Overrides**: Use environment variables for deployment-specific settings
5. **Hot Reloading**: Enable for development and staging environments
6. **Error Handling**: Leverage `NebulaError` integration for consistent error handling

## Examples

See the `examples/` directory for complete examples:

- `ecosystem_integration.rs` - Full ecosystem integration demo
- `basic_usage.rs` - Simple configuration loading
- `hot_reload.rs` - Hot reloading demonstration
- `validation.rs` - Configuration validation examples

## Integration with Other Nebula Crates

### With nebula-resilience

```rust
use nebula_config::prelude::*;
use nebula_resilience::prelude::*;

// Load resilience configuration
let resilience_config = config.get_typed::<DynamicConfig>("resilience").await?;

// Use with resilience patterns
let circuit_config = resilience_config.get_config::<CircuitBreakerConfig>("circuit_breaker")?;
let circuit_breaker = CircuitBreaker::with_config(circuit_config);
```

### With nebula-log

```rust
use nebula_config::prelude::*;

// Configure logging from configuration
let log_level: String = config.get("logging.level").await.unwrap_or_else(|_| "info".to_string());
let log_format: String = config.get("logging.format").await.unwrap_or_else(|_| "json".to_string());

nebula_log::builder()
    .with_level(&log_level)
    .with_format(&log_format)
    .init()?;
```

## License

Licensed under either of Apache License, Version 2.0 or MIT license at your option.
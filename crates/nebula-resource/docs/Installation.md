---
title: Installation
tags: [nebula, nebula-resource, docs]
status: draft
created: 2025-08-17
---


# Installation

## Prerequisites

### System Requirements

- **Rust**: 1.75+ (для async/await и последних features)
- **Tokio Runtime**: Асинхронная среда выполнения
- **Operating System**: Linux, macOS, Windows 10+
- **Memory**: Минимум 512MB RAM для development
- **Optional**: Docker для интеграционного тестирования

### Development Tools

```bash
# Проверка версии Rust
rustc --version  # должна быть 1.75+

# Установка Rust (если не установлен)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Установка cargo-watch для hot-reload
cargo install cargo-watch

# Установка cargo-expand для отладки макросов
cargo install cargo-expand

# Установка cargo-audit для проверки безопасности
cargo install cargo-audit
```

## Basic Installation

### Adding to Your Project

```toml
[dependencies]
nebula-resource = "0.2"
nebula-action = "0.2"
tokio = { version = "1", features = ["full"] }
async-trait = "0.1"
serde = { version = "1", features = ["derive"] }
```

### Feature Flags

```toml
[dependencies]
nebula-resource = { 
    version = "0.2",
    features = [
        # Core features
        "full",           # Все возможности
        "derive",         # Derive макросы
        
        # Resource types
        "http-client",    # HTTP клиент с circuit breaker
        "database",       # PostgreSQL, MySQL, SQLite
        "cache",          # Redis, Memcached
        "message-queue",  # Kafka, RabbitMQ
        "storage",        # S3, MinIO, GCS
        
        # Capabilities
        "observability",  # Logging, metrics, tracing
        "stateful",       # Stateful resources с версионированием
        "pooling",        # Advanced pooling
        "plugins",        # Plugin system
        "testing",        # Testing utilities и моки
        
        # Integrations
        "credential",     # nebula-credential integration
        "workflow",       # nebula-workflow integration
    ]
}
```

### Minimal Setup

```toml
# Минимальная конфигурация для начала работы
[dependencies]
nebula-resource = { 
    version = "0.2",
    default-features = false,
    features = ["derive", "tokio"]
}
```

## Workspace Setup

### Recommended Project Structure

```
my-nebula-project/
├── Cargo.toml                 # Workspace configuration
├── resources/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs            # Custom resources
│       └── builtin/          # Built-in resource configs
├── actions/
│   ├── Cargo.toml
│   └── src/
│       └── lib.rs            # Actions using resources
├── workflows/
│   ├── Cargo.toml
│   └── src/
│       └── lib.rs            # Workflow definitions
└── configs/
    ├── resources/
    │   ├── database.toml     # Database configuration
    │   ├── cache.yaml        # Cache configuration
    │   └── http.json         # HTTP client configuration
    └── credentials/
        └── .env              # Development credentials
```

### Workspace Cargo.toml

```toml
[workspace]
members = ["resources", "actions", "workflows"]
resolver = "2"

[workspace.dependencies]
nebula-resource = "0.2"
nebula-action = "0.2"
nebula-workflow = "0.2"
nebula-credential = "0.2"
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
async-trait = "0.1"
thiserror = "1.0"
tracing = "0.1"

[profile.release]
opt-level = 3
lto = true
codegen-units = 1

[profile.dev]
opt-level = 0
debug = true
```

## Environment Configuration

### Development Environment

```bash
# .env.development
RUST_LOG=debug
NEBULA_ENV=development
NEBULA_RESOURCE_DIR=./configs/resources
NEBULA_CREDENTIAL_DIR=./configs/credentials
NEBULA_PLUGIN_DIR=./target/debug/plugins

# Resource defaults
DATABASE_POOL_MIN=2
DATABASE_POOL_MAX=10
HTTP_CLIENT_TIMEOUT=30s
CACHE_TTL=300s

# Health checks
HEALTH_CHECK_INTERVAL=30s
HEALTH_CHECK_TIMEOUT=5s

# Circuit breaker
CIRCUIT_BREAKER_THRESHOLD=5
CIRCUIT_BREAKER_TIMEOUT=60s
```

### Production Environment

```bash
# .env.production
RUST_LOG=info
NEBULA_ENV=production
NEBULA_RESOURCE_DIR=/etc/nebula/resources
NEBULA_CREDENTIAL_DIR=/var/lib/nebula/credentials
NEBULA_PLUGIN_DIR=/usr/lib/nebula/plugins

# Resource optimization
DATABASE_POOL_MIN=10
DATABASE_POOL_MAX=100
HTTP_CLIENT_TIMEOUT=10s
CACHE_TTL=3600s

# Advanced features
ENABLE_PREDICTIVE_SCALING=true
ENABLE_CIRCUIT_BREAKER=true
ENABLE_RESOURCE_SHARDING=true
ENABLE_METRICS_COLLECTION=true

# Performance tuning
POOL_WARMING_STRATEGY=predictive
POOL_SCALE_UP_THRESHOLD=0.8
POOL_SCALE_DOWN_THRESHOLD=0.3
```

## Verification

### Test Installation

```rust
// src/main.rs
use nebula_resource::prelude::*;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt::init();
    
    // Initialize resource manager
    let manager = ResourceManager::builder()
        .with_default_resources()
        .with_health_check_interval(Duration::from_secs(30))
        .build()
        .await?;
    
    println!("✅ Nebula Resource installed successfully!");
    println!("Version: {}", nebula_resource::VERSION);
    println!("Available resources: {:?}", manager.list());
    
    // Test basic resource
    let logger = manager.get::<LoggerResource>("default").await?;
    logger.info("Test log message");
    
    // Verify health
    let health = manager.health_check_all().await;
    println!("Health status: {:?}", health);
    
    Ok(())
}
```

### Run Test

```bash
cargo run

# Expected output:
# ✅ Nebula Resource installed successfully!
# Version: 0.2.0
# Available resources: ["HttpClient", "Logger", "MetricsCollector"]
# [INFO] Test log message
# Health status: HealthReport { healthy: 3, degraded: 0, unhealthy: 0 }
```

## Platform-Specific Instructions

### Linux

```bash
# Install system dependencies
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libssl-dev

# For PostgreSQL support
sudo apt-get install -y libpq-dev

# For Redis support
sudo apt-get install -y redis-tools
```

### macOS

```bash
# Install Homebrew (if not installed)
/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"

# Install dependencies
brew install rust
brew install postgresql
brew install redis
```

### Windows

```powershell
# Install Visual Studio Build Tools
winget install Microsoft.VisualStudio.2022.BuildTools

# Install Rust
winget install Rustlang.Rust.MSVC

# For PostgreSQL support
winget install PostgreSQL.PostgreSQL
```

## Docker Setup

### Development with Docker

```dockerfile
# Dockerfile.dev
FROM rust:1.75

WORKDIR /app

# Install dependencies
RUN apt-get update && apt-get install -y \
    libpq-dev \
    libssl-dev \
    pkg-config

# Copy project files
COPY . .

# Build project
RUN cargo build --release

# Run application
CMD ["cargo", "run"]
```

### Docker Compose

```yaml
# docker-compose.yml
version: '3.8'

services:
  app:
    build:
      context: .
      dockerfile: Dockerfile.dev
    environment:
      - RUST_LOG=debug
      - DATABASE_URL=postgres://user:pass@postgres/nebula
      - REDIS_URL=redis://redis:6379
    depends_on:
      - postgres
      - redis
    volumes:
      - ./src:/app/src
      - ./configs:/app/configs

  postgres:
    image: postgres:15
    environment:
      POSTGRES_USER: user
      POSTGRES_PASSWORD: pass
      POSTGRES_DB: nebula
    volumes:
      - postgres_data:/var/lib/postgresql/data

  redis:
    image: redis:7-alpine
    volumes:
      - redis_data:/data

volumes:
  postgres_data:
  redis_data:
```

## Troubleshooting

### Common Issues

#### 1. Compilation Errors

```bash
# Clear cargo cache
cargo clean

# Update dependencies
cargo update

# Check for conflicting versions
cargo tree -d

# Verbose compilation
cargo build --verbose
```

#### 2. Missing Features

```toml
# Ensure all required features are enabled
[dependencies]
nebula-resource = { 
    version = "0.2",
    features = ["full"]  # Or specific features you need
}
```

#### 3. Async Runtime Issues

```rust
// Ensure tokio runtime is properly configured
#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    // ...
}

// Or use custom runtime
let runtime = tokio::runtime::Builder::new_multi_thread()
    .worker_threads(4)
    .enable_all()
    .build()?;
```

#### 4. Linking Errors

```bash
# Linux: Install missing libraries
sudo apt-get install -y libssl-dev pkg-config

# macOS: Update Xcode tools
xcode-select --install

# Windows: Install Visual C++ redistributables
winget install Microsoft.VCRedist.2022.x64
```

## Performance Optimization

### Compilation Optimization

```toml
# Cargo.toml
[profile.release]
opt-level = 3         # Maximum optimization
lto = true           # Link-time optimization
codegen-units = 1    # Single codegen unit
strip = true         # Strip symbols
panic = "abort"      # Smaller binary

[profile.bench]
inherits = "release"
debug = true         # Include debug symbols for profiling
```

### Runtime Optimization

```rust
// Pre-warm resource pools
manager.warm_pools().await?;

// Enable predictive scaling
manager.enable_predictive_scaling(
    PredictiveConfig {
        history_window: Duration::from_hours(24),
        prediction_horizon: Duration::from_hours(1),
        scale_factor: 1.2,
    }
).await?;

// Configure circuit breakers
manager.configure_circuit_breakers(
    CircuitBreakerConfig {
        failure_threshold: 5,
        timeout: Duration::from_secs(60),
        half_open_max_calls: 3,
    }
).await?;
```

## Security Considerations

### Credential Management

```bash
# Never commit credentials
echo ".env" >> .gitignore
echo "configs/credentials/" >> .gitignore

# Use environment-specific files
.env.development
.env.staging
.env.production

# Encrypt sensitive data
nebula-credential encrypt --input .env --output .env.encrypted
```

### Network Security

```toml
# Enable TLS for all connections
[dependencies]
nebula-resource = { 
    version = "0.2",
    features = ["tls-native"]
}
```

## Next Steps

- [[QuickStart|Quick Start Guide]] - Создайте первый ресурс за 5 минут
- [[BasicConcepts|Basic Concepts]] - Изучите основные концепции
- [[FirstResource|First Resource]] - Пошаговое создание ресурса
- [[Examples/HttpClient|HTTP Client Example]] - Практический пример

## Support

- [GitHub Issues](https://github.com/nebula/nebula-resource/issues)
- [Discord Community](https://discord.gg/nebula)
- [Stack Overflow](https://stackoverflow.com/questions/tagged/nebula-resource)
- [Documentation](https://docs.nebula.dev/resource)

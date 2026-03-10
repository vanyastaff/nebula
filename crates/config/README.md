# Nebula Config

Production-ready configuration management for Rust applications in the Nebula ecosystem.

## Overview

`nebula-config` provides typed configuration loading from **TOML files** and **environment variables**, with deterministic source precedence, async-safe access, validation, and optional hot reload.

Current format policy is intentionally strict:

- File-based configuration: **TOML only**
- Environment overrides: **ENV / prefixed ENV**

## Key Features

- TOML + ENV source composition with predictable precedence
- Typed reads via `serde` (`config.get::<T>(...)`)
- Defaults + runtime overrides (`set_value`, `set_typed`, `merge`)
- Validation via `ConfigValidator` trait (with `nebula-validator` bridge)
- File watcher and interval-based reload options
- Strict/permissive environment value parsing

## Installation

```toml
[dependencies]
nebula-config = "0.1.0"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

Default crate features already include `toml` and `env`.

## Quick Start

```rust
use nebula_config::prelude::*;

#[tokio::main]
async fn main() -> ConfigResult<()> {
    let config = ConfigBuilder::new()
        .with_defaults_json(serde_json::json!({
            "server": { "port": 3000 },
            "database": { "host": "localhost" }
        }))
        .with_source(ConfigSource::File("config.toml".into()))
        .with_source(ConfigSource::Env)
        .build()
        .await?;

    let port: u16 = config.get("server.port").await?;
    let db_host: String = config.get("database.host").await?;

    println!("server.port={port}, database.host={db_host}");
    Ok(())
}
```

## Sources and Precedence

Available `ConfigSource` variants:

- `ConfigSource::Default`
- `ConfigSource::File(path)`
- `ConfigSource::FileAuto(path)`
- `ConfigSource::Directory(path)`
- `ConfigSource::Env`
- `ConfigSource::EnvWithPrefix(prefix)`

Effective precedence (higher wins):

1. Environment (`Env`, `EnvWithPrefix`)
2. Directory
3. File / FileAuto
4. Defaults

Optional-source failures (`Env`, `EnvWithPrefix`, `Default`) are skipped by default.
Enable strict behavior in both `build()` and `reload()`:

```rust
let config = ConfigBuilder::new()
    .with_source(ConfigSource::File("config.toml".into()))
    .with_source(ConfigSource::Env)
    .with_fail_on_missing(true)
    .build()
    .await?;
```

## TOML Example

`config.toml`:

```toml
[server]
host = "0.0.0.0"
port = 8080

[database]
host = "localhost"
port = 5432

[features]
enabled = ["logging", "metrics"]
```

## Environment Mapping

Environment keys are converted into nested paths using `_` as separator.

- `SERVER_PORT=9000` → `server.port = 9000`
- `DATABASE_HOST=db.internal` → `database.host = "db.internal"`
- `FEATURES_ENABLED=auth,metrics` → `features.enabled = ["auth", "metrics"]` (permissive mode)

With a prefix:

```rust
let config = ConfigBuilder::new()
    .with_source(ConfigSource::EnvWithPrefix("MYAPP".to_string()))
    .build()
    .await?;
```

Example input: `MYAPP_SERVER_PORT=8081`.

## Environment Parse Modes

By default, env parsing is permissive (`bool`, numbers, JSON snippets, CSV arrays).

Use strict mode to keep all env values as strings:

```rust
use nebula_config::EnvParseMode;

let config = ConfigBuilder::new()
    .with_source(ConfigSource::Env)
    .with_env_parse_mode(EnvParseMode::Strict)
    .build()
    .await?;
```

Shortcut:

```rust
let config = ConfigBuilder::new()
    .with_source(ConfigSource::Env)
    .with_env_strict_parsing()
    .build()
    .await?;
```

## Validation

Schema validation (JSON schema represented as `serde_json::Value`):

```rust
use nebula_config::prelude::*;
use std::sync::Arc;

Any type implementing `nebula_validator::Validate<Value>` automatically satisfies `ConfigValidator`
via the blanket impl in `core::traits`.

```rust
use nebula_config::{ConfigBuilder, ConfigSource, ConfigValidator};
use std::sync::Arc;

// Any nebula_validator::Validate<Value> impl works as ConfigValidator
let config = ConfigBuilder::new()
    .with_source(ConfigSource::File("config.toml".into()))
    .with_validator(Arc::new(my_validator))
    .build()
    .await?;
```

## Hot Reload

```rust
use nebula_config::prelude::*;
use std::sync::Arc;
use std::time::Duration;

let config = ConfigBuilder::new()
    .with_source(ConfigSource::File("config.toml".into()))
    .with_watcher(Arc::new(FileWatcher::new_noop()))
    .with_hot_reload(true)
    .with_auto_reload_interval(Duration::from_secs(10))
    .build()
    .await?;

config.reload().await?;
```

## Runtime API Highlights

- Typed access: `get`, `get_all`, `get_opt`, `get_or`, `get_or_else`, `has`
- Raw access: `get_raw`, `get_value`, `as_value`, `keys`, `flatten`
- Mutation: `set_value`, `set_typed`, `merge`
- Lifecycle: `reload`, `start_watching`, `stop_watching`, `is_watching`

## Notes

- `Config::get_path` is deprecated; use `get`.
- `ConfigFormat` still includes non-TOML variants for compatibility/metadata, but file loading is TOML-only.
- If you need to parse config text manually, use `nebula_config::utils::parse_config_string` with `ConfigFormat::Toml`.

## License

Licensed under either Apache-2.0 or MIT, at your option.
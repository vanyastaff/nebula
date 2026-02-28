# API

## Public Surface

- stable APIs:
  - `ConfigBuilder`
  - `Config`
  - `ConfigSource`, `ConfigFormat`, `SourceMetadata`
  - `ConfigLoader`, `ConfigValidator`, `ConfigWatcher`
  - `ConfigError`, `ConfigResult`
- experimental APIs:
  - source variants not fully implemented by default loaders (`Remote`, `Database`, `KeyValue`)
- hidden/internal APIs:
  - parser internals in loader modules

## Usage Patterns

- startup assembly:
  - build once via `ConfigBuilder`, then pass `Config` as shared dependency
- layered precedence:
  - defaults < files < env < inline high-priority overrides
- typed retrieval:
  - use `get<T>` and keep path constants centralized in consuming crates

## Minimal Example

```rust
use nebula_config::prelude::*;

let cfg = ConfigBuilder::new()
    .with_source(ConfigSource::File("config.toml".into()))
    .with_source(ConfigSource::Env)
    .build()
    .await?;

let port: u16 = cfg.get("server.port").await?;
```

## Advanced Example

```rust
use nebula_config::prelude::*;
use std::sync::Arc;

let cfg = ConfigBuilder::new()
    .with_defaults_json(serde_json::json!({ "server": { "port": 3000 }}))
    .with_source(ConfigSource::File("config.toml".into()))
    .with_validator(Arc::new(SchemaValidator::new(serde_json::json!({ "type": "object" }))))
    .with_hot_reload(true)
    .build()
    .await?;

cfg.reload().await?;
```

## Error Semantics

- retryable errors:
  - typically source IO/availability errors are caller-retryable.
- fatal errors:
  - invalid format, invalid path/type conversion for required values.
- validation errors:
  - validator-reported errors that block config activation.

## Compatibility Rules

- major bump when:
  - source precedence semantics change
  - path traversal behavior changes
  - validation contract semantics change
- deprecation policy:
  - keep aliases/deprecated accessors for at least one minor cycle where possible

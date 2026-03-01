# API Contract

## Main initialization surface

| API | Use case | Notes |
|---|---|---|
| `auto_init()` | Fast startup with env/preset resolution | Uses precedence contract |
| `init()` | Default setup | Equivalent to `init_with(Config::default())` |
| `init_with(Config)` | Deterministic explicit configuration | Preferred for production |
| `LoggerBuilder::from_config(...).build()` | Advanced setup path | Used internally by top-level APIs |

## Startup precedence

Order is strict and stable:
1. explicit config
2. environment config
3. preset fallback

This behavior is part of the crate contract.

## Public config types

- `Config`
- `Format`
- `WriterConfig`
- `DestinationFailurePolicy`
- `Level`
- `Rolling`

## Writer failure policies

| Policy | Behavior |
|---|---|
| `BestEffort` | Continue when at least one destination succeeds |
| `FailFast` | Stop on first destination error |
| `PrimaryWithFallback` | Prefer primary, fallback to secondary on primary failure |

## Observability API surface

Exposed via `observability` module and `prelude` re-exports:
- hook registration
- event emission
- typed operation events
- operation tracker helpers

## Error contract

Initialization APIs return `LogError` for:
- invalid filter directives
- writer initialization failures
- telemetry exporter/layer setup failures

No panic-based control flow is part of normal API usage.

## Minimal production example

```rust
use nebula_log::{Config, Format, WriterConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut cfg = Config::production();
    cfg.format = Format::Json;
    cfg.writer = WriterConfig::Stderr;

    let _guard = nebula_log::init_with(cfg)?;
    Ok(())
}
```

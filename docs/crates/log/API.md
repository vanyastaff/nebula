# API

## Public Surface

- **Stable APIs:** `auto_init`, `init`, `init_with`, `Config`, `Format`, `Level`, `WriterConfig`, `Context`, `Timer`, `TimerGuard`, `LogError`, `LogResult`, tracing macros (`info!`, `error!`, `span!`, etc.), `ObservabilityEvent`, `ObservabilityHook`, `register_hook`, `emit_event`, `shutdown_hooks`, `OperationTracker`, `OperationStarted`, `OperationCompleted`, `OperationFailed`
- **Experimental APIs:** `ResourceAwareHook`, `EventFilter`, `FilteredHook`, `current_contexts`
- **Hidden/internal APIs:** builder internals, layer composition, registry implementation

## Usage Patterns

- **Quick start:** `auto_init()` for env/dev/prod auto-detection
- **Production:** `init_with(Config::production())` with custom fields and reload
- **Tests:** `init_test()` or `Config::test()` for captured output

## Minimal Example

```rust
use nebula_log::prelude::*;

fn main() -> LogResult<()> {
    nebula_log::auto_init()?;
    info!(port = 8080, "Server starting");
    Ok(())
}
```

## Advanced Example

```rust
use nebula_log::{Config, init_with};
use serde_json::json;

fn main() -> LogResult<()> {
    let mut config = Config::production();
    config.fields.service = Some("api-gateway".to_string());
    config.fields.env = Some("production".to_string());
    config.fields.custom.insert("datacenter".to_string(), json!("us-west-2"));
    config.reloadable = true;

    let _guard = init_with(config)?;

    tracing::info!(
        endpoint = "/api/v1/users",
        method = "GET",
        "Request received"
    );
    Ok(())
}
```

## Error Semantics

- **Retryable errors:** None; init is typically one-shot. I/O errors from file writer may be retried by caller.
- **Fatal errors:** `LogError::Config`, `LogError::Filter` — invalid config; `LogError::Telemetry` — telemetry setup failure.
- **Validation errors:** `LogError::Filter` for invalid `RUST_LOG`/`NEBULA_LOG` filter strings.

## Compatibility Rules

- **Major version bump:** Removal of deprecated APIs, config schema breaking changes, trait signature changes.
- **Deprecation policy:** Minimum 6 months with `#[deprecated]` and migration guide in MIGRATION.md.

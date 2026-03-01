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

## Non-Blocking File Writer Policy

When `WriterConfig::File { non_blocking: true, .. }` is used:

- writes are dispatched through `tracing_appender::non_blocking(...)`
- events are enqueued into a bounded background queue
- under sustained overload, queue saturation may lead to dropped log events
- `WorkerGuard` must be held for the process lifetime to flush buffered events on shutdown

Current contract:

- default behavior is best-effort throughput with bounded memory
- no per-event backpressure signal is exposed through `nebula-log` public API
- applications requiring strict durability should use `non_blocking: false` and accept sync I/O latency

## Compatibility Rules

- **Major version bump:** Removal of deprecated APIs, config schema breaking changes, trait signature changes.
- **Deprecation policy:** Minimum 6 months with `#[deprecated]` and migration guide in MIGRATION.md.

Hook policy note:

- `set_hook_policy(HookPolicy::Bounded { ... })` currently keeps inline dispatch and adds budget-overrun diagnostics.
- async hook offload is planned separately and is not part of current contract.

Hook shutdown ordering contract:

- `shutdown_hooks()` drains registered hooks in reverse registration order (LIFO).
- registry is quiesced before shutdown callbacks run, so new dispatches are not started during shutdown.

## Observability Payload Contract

`ObservabilityEvent` payloads are emitted via visitor API:

- `fn visit_fields(&self, visitor: &mut dyn ObservabilityFieldVisitor)`
- value types: `ObservabilityFieldValue::{Str, Bool, I64, U64, F64}`

Compatibility helper:

- `event_data_json(&dyn ObservabilityEvent) -> Option<serde_json::Value>`
- use only when JSON payload is required by a consumer; this path allocates

Hot-path guidance:

- hooks should prefer direct visitor processing (or `EventFields` display wrapper)
- avoid building per-event JSON objects in latency-sensitive hooks

## Env and Precedence Contract

Startup precedence is deterministic:

1. explicit config passed to `init_with`/`resolve_startup(Some(...))`
2. environment overrides on top of preset
3. preset (`development` in debug builds, `production` in release builds)

Source marker returned by resolution:

- `ResolvedSource::Explicit`
- `ResolvedSource::Environment`
- `ResolvedSource::Preset`

Environment variable contract:

| Variable | Target field | Accepted values | Notes |
|---|---|---|---|
| `NEBULA_LOG` | `Config.level` | any tracing filter string | takes precedence over `RUST_LOG` |
| `RUST_LOG` | `Config.level` | any tracing filter string | used only when `NEBULA_LOG` is unset |
| `NEBULA_LOG_FORMAT` | `Config.format` | `pretty`, `compact`, `json`, `logfmt` | case-insensitive; invalid value is ignored |
| `NEBULA_LOG_TIME` | `Config.display.time` | boolean-ish string | `0/false/FALSE/False` => `false`, everything else => `true` |
| `NEBULA_LOG_SOURCE` | `Config.display.source` | boolean-ish string | same bool parsing rules |
| `NEBULA_LOG_COLORS` | `Config.display.colors` | boolean-ish string | same bool parsing rules |
| `NEBULA_SERVICE` | `Config.fields.service` | string | via `Fields::from_env()` |
| `NEBULA_ENV` | `Config.fields.env` | string | via `Fields::from_env()` |
| `NEBULA_VERSION` | `Config.fields.version` | string | fallback to crate version when env is unset |
| `NEBULA_INSTANCE` | `Config.fields.instance` | string | via `Fields::from_env()` |
| `NEBULA_REGION` | `Config.fields.region` | string | via `Fields::from_env()` |

Contract notes:

- env override is applied as a single layer; if any supported env var is applied, source is `Environment`.
- field overrides from `Fields::from_env()` replace `Config.fields` as a whole.
- explicit runtime config always wins and bypasses env/preset resolution.

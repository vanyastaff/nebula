# API

## Context type

The execution context passed to `execute_action` is currently **NodeContext** (from nebula-action, deprecated there). The target is **ActionContext** (or TriggerContext for triggers) and `&impl Context`; see [INTERACTIONS.md](./INTERACTIONS.md#context-contract-current-vs-target) and CONSTITUTION P-001.

## Public Surface

### Stable APIs

- `ActionRuntime` — main orchestrator
- `ActionRuntime::new(registry, sandbox, data_policy, event_bus, metrics)`
- `ActionRuntime::execute_action(action_key, input, context) -> Result<ActionResult, RuntimeError>` (context: NodeContext today; ActionContext planned)
- `ActionRuntime::registry()`, `ActionRuntime::data_policy()`
- `ActionRegistry` — `new()`, `register()`, `get()`, `contains()`, `remove()`, `len()`, `is_empty()`, `keys()`
- `DataPassingPolicy` — `max_node_output_bytes`, `max_total_execution_bytes`, `large_data_strategy`
- `DataPassingPolicy::check_output_size(output) -> Result<u64, (u64, u64)>`
- `LargeDataStrategy` — Reject, SpillToBlob
- `RuntimeError` — ActionNotFound, ActionError, DataLimitExceeded, Internal
- `RuntimeError::is_retryable()`

### Experimental / TODO

- Isolation level routing (SandboxedContext)
- SpillToBlob implementation
- max_total_execution_bytes enforcement

## Usage Patterns

### Construction

```rust
use nebula_runtime::{ActionRuntime, DataPassingPolicy};
use nebula_runtime::registry::ActionRegistry;
use nebula_telemetry::event::EventBus;
use nebula_telemetry::metrics::MetricsRegistry;

let registry = Arc::new(ActionRegistry::new());
registry.register(Arc::new(my_handler));

let runtime = Arc::new(ActionRuntime::new(
    registry,
    sandbox,
    DataPassingPolicy::default(),
    Arc::new(EventBus::new(64)),
    Arc::new(MetricsRegistry::new()),
));
```

### Execution (from engine)

```rust
// context is NodeContext today; may become ActionContext
let result = runtime
    .execute_action("http.request", input, context)
    .await?;
```

### Data policy customization

```rust
let policy = DataPassingPolicy {
    max_node_output_bytes: 5 * 1024 * 1024, // 5 MB
    max_total_execution_bytes: 50 * 1024 * 1024, // 50 MB
    large_data_strategy: LargeDataStrategy::Reject,
};
```

## Minimal Example

```rust
let registry = Arc::new(ActionRegistry::new());
registry.register(Arc::new(EchoHandler { meta }));

let runtime = ActionRuntime::new(
    registry,
    sandbox,
    DataPassingPolicy::default(),
    event_bus,
    metrics,
);

let result = runtime
    .execute_action("test.echo", json!({"x": 1}), context)
    .await?;
```

## Advanced Example

```rust
// Custom data policy with SpillToBlob (when implemented)
let policy = DataPassingPolicy {
    max_node_output_bytes: 1024 * 1024,
    max_total_execution_bytes: 10 * 1024 * 1024,
    large_data_strategy: LargeDataStrategy::SpillToBlob,
};

let runtime = ActionRuntime::new(registry, sandbox, policy, event_bus, metrics);

// Error handling
match runtime.execute_action(key, input, ctx).await {
    Ok(ActionResult::Success { output }) => { /* use output */ }
    Err(RuntimeError::ActionNotFound { key }) => { /* handle */ }
    Err(RuntimeError::DataLimitExceeded { limit_bytes, actual_bytes }) => { /* handle */ }
    Err(RuntimeError::ActionError(e)) if e.is_retryable() => { /* retry */ }
    Err(e) => return Err(e.into()),
}
```

## Error Semantics

- **ActionNotFound:** Handler not registered for key; non-retryable.
- **ActionError:** From action execution; may be retryable (e.g. timeout).
- **DataLimitExceeded:** Output exceeded max_node_output_bytes with Reject strategy; non-retryable.
- **Internal:** Unexpected runtime error; non-retryable.

## Compatibility Rules

- **Major version bump:** Breaking execute_action signature; registry API removal.
- **Deprecation policy:** Minimum 2 minor releases.

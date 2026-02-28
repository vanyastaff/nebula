# API

## Public Surface (Current — nebula-execution)

### Stable APIs

- `IdempotencyKey` — deterministic key from execution_id, node_id, attempt
- `IdempotencyKey::generate(execution_id, node_id, attempt)` — key generation
- `IdempotencyKey::as_str()` — string representation
- `IdempotencyManager` — in-memory deduplication
- `IdempotencyManager::new()` — create empty
- `IdempotencyManager::check_and_mark(key)` — returns true if new, false if duplicate
- `IdempotencyManager::is_seen(key)` — check without marking
- `IdempotencyManager::clear()` — reset
- `ExecutionError::DuplicateIdempotencyKey(String)` — duplicate key error

### Usage in NodeAttempt

- `NodeAttempt::new(attempt_number, idempotency_key)` — each attempt has key
- `NodeAttempt::idempotency_key` — field access

## Public Surface (Planned)

### Target APIs

- `IdempotencyStorage` trait — get, set, delete
- `IdempotencyConfig` — key strategy, TTL, conflict behavior
- `IdempotentAction` trait — idempotency_config, is_safe_to_retry
- `IdempotencyLayer` — axum middleware for Idempotency-Key header
- Storage backends: `MemoryStorage`, `PostgresStorage`, `RedisStorage`

## Usage Patterns (Current)

### Key Generation

```rust
use nebula_execution::{IdempotencyKey, IdempotencyManager};
use nebula_core::{ExecutionId, NodeId};

let key = IdempotencyKey::generate(execution_id, node_id, 0);
let mut mgr = IdempotencyManager::new();
if mgr.check_and_mark(&key) {
    // First time — execute
} else {
    // Duplicate — skip or return cached
}
```

### NodeAttempt

```rust
let attempt = NodeAttempt::new(0, IdempotencyKey::generate(exec_id, node_id, 0));
// attempt.idempotency_key used for journaling
```

## Minimal Example (Target)

```rust
// Future: action-level
#[idempotent]
impl IdempotentAction for SendEmailAction {
    fn idempotency_config(&self) -> IdempotencyConfig {
        IdempotencyConfig::default()
    }
}
```

## Advanced Example (Target)

```rust
// Future: HTTP layer
let app = Router::new()
    .route("/api/orders", post(create_order))
    .layer(IdempotencyLayer::new(storage).ttl(Duration::from_hours(24)));
```

## Error Semantics

- **DuplicateIdempotencyKey:** Returned when key already used; caller should return cached result or reject
- **Storage errors:** Future; retry or fail depending on backend

## Compatibility Rules

- **Key format:** `{execution_id}:{node_id}:{attempt}` stable for node-level
- **Major bump:** Key format change; storage schema change

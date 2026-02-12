# API Contract: nebula-resilience

**Crate**: nebula-resilience
**Migration Phase**: Phase 2.2
**Date**: 2026-02-11

## Public API Stability

### No Breaking Changes

The `nebula-resilience` crate uses `Value` for policy configuration serialization. **All public APIs remain unchanged.**

### Internal Changes

| Component | Change | Impact |
|-----------|--------|--------|
| Imports | `nebula_value::Value` → `serde_json::Value` | Internal only |
| Policy configs | Serialized as `Value` | Same JSON structure |
| Error types | Add `ResilienceError::Json(#[from] serde_json::Error)` | New error variant |

### Behavioral Guarantees

- Retry policies serialize/deserialize identically
- Circuit breaker configurations remain compatible
- Rate limiting configs unchanged
- Error messages may differ slightly (serde_json wording)

### Testing Coverage

All existing tests MUST pass without modification. Policy configurations remain compatible.

---

## Migration Details

### Public API Surface

No changes to public types (RetryPolicy, CircuitBreakerPolicy, etc.). Migration affects internal Value usage only.

### Dependencies Updated

```toml
# Removed
nebula-value = { path = "../nebula-value", features = ["serde"] }

# Ensured present
serde_json = { workspace = true }
```

### Error Handling

```rust
#[derive(Debug, Error)]
pub enum ResilienceError {
    // ✅ NEW: Handle serde_json errors
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    // Existing variants unchanged
    #[error("Invalid policy configuration: {0}")]
    InvalidPolicy(String),

    #[error("Policy violation: {0}")]
    PolicyViolation(String),
}
```

### Affected Components

- **Dynamic policy config**: Serializes policy parameters as `serde_json::Value`
- **Config builders**: Use `serde_json::json!` macro for configuration literals
- **Validation**: Type checks use `value.is_i64()` instead of `value.is_integer()`

---

## Serialization Format

### Before/After (Identical JSON)

```json
{
  "max_retries": 3,
  "backoff_ms": 1000,
  "timeout_ms": 5000
}
```

Both nebula-value and serde_json produce identical JSON. Migration transparent.

---

## Validation

- [ ] `cargo check -p nebula-resilience` - compiles
- [ ] `cargo test -p nebula-resilience` - 100% pass rate
- [ ] `cargo clippy -p nebula-resilience -- -D warnings` - no warnings
- [ ] Policy configurations serialize/deserialize identically (tested with sample policies)
- [ ] No changes to public API (verified by code review)

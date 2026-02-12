# API Contract: nebula-config

**Crate**: nebula-config
**Migration Phase**: Phase 2.1
**Date**: 2026-02-11

## Public API Stability

### No Breaking Changes

The `nebula-config` crate uses `Value` only internally for configuration data structures. **All public APIs remain unchanged.**

### Internal Changes Only

| Component | Change | Impact |
|-----------|--------|--------|
| Imports | `nebula_value::Value` → `serde_json::Value` | Internal only |
| Config parsing | TOML/YAML → `Value` | Same behavior |
| Error types | Add `ConfigError::Json(#[from] serde_json::Error)` | New error variant (transparent to users) |

### Behavioral Guarantees

- Configuration file parsing remains identical
- Error messages may differ slightly (serde_json vs nebula-value wording)
- Serialization/deserialization format unchanged (TOML/YAML → same structure)

### Testing Coverage

All existing tests MUST pass without modification. Configuration files remain compatible.

---

## Migration Details

### Public API Surface

No changes to public types or functions. Migration is entirely internal.

### Dependencies Updated

```toml
# Removed
nebula-value = { path = "../nebula-value" }

# Ensured present (likely already exists)
serde_json = { workspace = true }
```

### Error Handling

```rust
#[derive(Debug, Error)]
pub enum ConfigError {
    // ✅ NEW: Handle serde_json errors
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    // Existing variants unchanged
    #[error("Invalid configuration: {0}")]
    Invalid(String),

    #[error("Missing required field: {0}")]
    MissingField(String),
}
```

---

## Validation

- [ ] `cargo check -p nebula-config` - compiles
- [ ] `cargo test -p nebula-config` - 100% pass rate
- [ ] `cargo clippy -p nebula-config -- -D warnings` - no warnings
- [ ] No changes to public API (verified by code review)
- [ ] Configuration files parse identically (tested with sample configs)

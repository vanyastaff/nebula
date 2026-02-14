# Dynamic Configuration Type Safety Analysis

## Current Implementation

**File:** `src/core/dynamic.rs` (364 LOC)

### Architecture

```rust
pub struct DynamicConfig {
    values: Object,              // nebula_value::Object
    schema_version: String,
    last_updated: Option<String>,
}
```

**Key Methods:**
- `set_value(path: &str, value: Value)` - Runtime string path parsing
- `get_value(path: &str) -> Value` - Runtime type checking
- `set_config<T: ResilienceConfig>(path, config)` - Generic with runtime conversion
- `get_config<T: ResilienceConfig>(path)` - Runtime deserialization

## Issues Identified

### 1. **Runtime Path Parsing**

```rust
// Current: Runtime errors only
config.set_value("retry.max_attempts", Value::from(3))?;
config.set_value("retrr.max_attempts", Value::from(3))?; // Typo not caught!
```

**Problem:** Typos and invalid paths only detected at runtime.

### 2. **No Compile-Time Type Checking**

```rust
// Current: Type mismatch only at runtime
config.set_value("retry.max_attempts", Value::from("invalid"))?; // Wrong type!
```

**Problem:** Type errors not caught until config is used.

### 3. **Weak IDE Support**

- No autocomplete for config paths
- No type hints for values
- Manual string construction prone to errors

### 4. **Limited Validation**

```rust
// Current: Basic path validation only
if path.is_empty() {
    return Err(ConfigError::validation("Empty path not allowed"));
}
```

**Problem:** No validation of value types or ranges.

## Proposed Solutions

### Option 1: Type-Safe Builder Pattern ⭐ (Recommended)

**Pros:**
- Compile-time type checking
- IDE autocomplete support
- Chainable API
- Backward compatible (can coexist with current API)

**Cons:**
- More boilerplate code
- Larger API surface

**Implementation:**

```rust
/// Type-safe builder for dynamic configuration
pub struct DynamicConfigBuilder {
    inner: DynamicConfig,
}

impl DynamicConfigBuilder {
    pub fn new() -> Self {
        Self {
            inner: DynamicConfig::new(),
        }
    }

    /// Type-safe retry configuration
    pub fn retry(mut self) -> RetryConfigBuilder {
        RetryConfigBuilder::new(self)
    }

    /// Type-safe circuit breaker configuration
    pub fn circuit_breaker(mut self) -> CircuitBreakerConfigBuilder {
        CircuitBreakerConfigBuilder::new(self)
    }

    /// Type-safe bulkhead configuration
    pub fn bulkhead(mut self) -> BulkheadConfigBuilder {
        BulkheadConfigBuilder::new(self)
    }

    /// Build final configuration
    pub fn build(self) -> DynamicConfig {
        self.inner
    }
}

/// Builder for retry configuration
pub struct RetryConfigBuilder {
    parent: DynamicConfigBuilder,
    max_attempts: Option<usize>,
    base_delay: Option<Duration>,
}

impl RetryConfigBuilder {
    fn new(parent: DynamicConfigBuilder) -> Self {
        Self {
            parent,
            max_attempts: None,
            base_delay: None,
        }
    }

    /// Set maximum retry attempts (type-safe!)
    pub fn max_attempts(mut self, attempts: usize) -> Self {
        self.max_attempts = Some(attempts);
        self
    }

    /// Set base delay between retries (type-safe!)
    pub fn base_delay(mut self, delay: Duration) -> Self {
        self.base_delay = Some(delay);
        self
    }

    /// Finish retry configuration and return to parent
    pub fn done(mut self) -> Result<DynamicConfigBuilder, ConfigError> {
        // Validate required fields
        let max_attempts = self.max_attempts
            .ok_or_else(|| ConfigError::validation("max_attempts is required"))?;
        let base_delay = self.base_delay
            .ok_or_else(|| ConfigError::validation("base_delay is required"))?;

        // Set values in parent config
        self.parent.inner.set_value(
            "retry.max_attempts",
            Value::from(max_attempts as i64)
        )?;
        self.parent.inner.set_value(
            "retry.base_delay_ms",
            Value::from(base_delay.as_millis() as i64)
        )?;

        Ok(self.parent)
    }
}
```

**Usage Example:**

```rust
// Type-safe API
let config = DynamicConfigBuilder::new()
    .retry()
        .max_attempts(3)                    // ✓ Type-safe usize
        .base_delay(Duration::from_millis(100))  // ✓ Type-safe Duration
        .done()?
    .circuit_breaker()
        .failure_threshold(5)               // ✓ Type-safe usize
        .reset_timeout(Duration::from_secs(30))  // ✓ Type-safe Duration
        .done()?
    .build();

// Compile-time errors:
config.retry()
    .max_attempts("invalid");  // ❌ Compile error: expected usize, found &str
config.retry()
    .max_attemps(3);          // ❌ Compile error: no method 'max_attemps' (typo caught!)
```

### Option 2: Macro-Based DSL

**Pros:**
- Declarative syntax
- Compile-time validation
- Very clean API

**Cons:**
- Complex macro implementation
- Harder to debug
- Steeper learning curve

**Implementation:**

```rust
// Macro-based DSL
dynamic_config! {
    retry {
        max_attempts: 3,
        base_delay: 100ms,
    },
    circuit_breaker {
        failure_threshold: 5,
        reset_timeout: 30s,
    }
}
```

### Option 3: Hybrid Approach ⭐⭐ (Best of Both Worlds)

Keep both APIs:
1. **String-based API** - For runtime dynamic configs (external sources)
2. **Builder API** - For compile-time type safety (code configuration)

```rust
// Runtime dynamic config (keep existing)
let mut config = DynamicConfig::new();
config.set_value("retry.max_attempts", external_value)?;

// Compile-time type-safe config (new)
let config = DynamicConfigBuilder::new()
    .retry()
        .max_attempts(3)
        .done()?
    .build();
```

## Comparison Matrix

| Feature | Current | Builder | Macro | Hybrid |
|---------|---------|---------|-------|--------|
| Type Safety | ❌ Runtime | ✅ Compile | ✅ Compile | ✅ Both |
| IDE Support | ❌ None | ✅ Full | ⚠️ Limited | ✅ Full |
| Runtime Flexibility | ✅ High | ❌ Low | ❌ Low | ✅ High |
| Learning Curve | ✅ Low | ✅ Low | ❌ High | ⚠️ Medium |
| Implementation Effort | - | ⭐⭐ Medium | ⭐⭐⭐ High | ⭐⭐ Medium |
| Backward Compatible | - | ✅ Yes | ❌ No | ✅ Yes |

## Recommendations

### Phase 1: Builder Pattern (P2 - 2-3 days) ⭐ Recommended

**Implement:**
1. `DynamicConfigBuilder` with type-safe methods
2. Specific builders: `RetryConfigBuilder`, `CircuitBreakerConfigBuilder`, `BulkheadConfigBuilder`
3. Validation in `done()` methods
4. Comprehensive tests

**Benefits:**
- ✅ Compile-time type checking
- ✅ IDE autocomplete
- ✅ Backward compatible
- ✅ Gradual migration path

**Files to Create:**
- `src/core/dynamic/builder.rs` - Main builder
- `src/core/dynamic/retry_builder.rs` - Retry config builder
- `src/core/dynamic/circuit_breaker_builder.rs` - CB builder
- `src/core/dynamic/bulkhead_builder.rs` - Bulkhead builder
- `tests/dynamic_builder_tests.rs` - Comprehensive tests

### Phase 2: Enhanced Validation (P3 - 1 day)

**Add:**
- Range validation for numeric values
- Format validation for strings
- Cross-field validation (e.g., min < max)

### Phase 3: Macro DSL (Future - Optional)

**Consider if:**
- User feedback requests simpler syntax
- Configuration complexity grows significantly
- Team has macro expertise

## Migration Strategy

### For Existing Code:

```rust
// Old API (still works)
let mut config = DynamicConfig::new();
config.set_value("retry.max_attempts", Value::from(3))?;

// New API (recommended for new code)
let config = DynamicConfigBuilder::new()
    .retry().max_attempts(3).done()?
    .build();

// Conversion helper
impl From<DynamicConfigBuilder> for DynamicConfig {
    fn from(builder: DynamicConfigBuilder) -> Self {
        builder.build()
    }
}
```

### Deprecation Timeline:

- **Now:** Add builder API
- **3 months:** Deprecate string-based API for static configs
- **6 months:** Optional: Remove deprecated APIs (breaking change)

## Implementation Checklist

- [ ] Create `dynamic/builder.rs` module
- [ ] Implement `DynamicConfigBuilder`
- [ ] Implement `RetryConfigBuilder`
- [ ] Implement `CircuitBreakerConfigBuilder`
- [ ] Implement `BulkheadConfigBuilder`
- [ ] Add validation in `done()` methods
- [ ] Write comprehensive tests
- [ ] Update documentation
- [ ] Add examples
- [ ] Update migration guide

## Estimated Impact

**Effort:** Medium (2-3 days)
**Impact:** High

**Benefits:**
- 🔒 Type safety at compile time
- 🚀 Better IDE support
- 🐛 Fewer runtime errors
- 📚 Improved documentation
- ✅ Better DX (Developer Experience)

**Trade-offs:**
- Slightly larger codebase
- More API surface to maintain
- Both APIs supported (at least initially)

## Conclusion

**Recommendation:** Implement **Hybrid Approach** (Option 3)

1. Keep existing string-based API for runtime flexibility
2. Add type-safe builder API for static configurations
3. Gradually migrate examples and documentation to builder API
4. Provide clear migration guide

This approach provides the best balance of:
- Type safety for static configs
- Runtime flexibility for dynamic configs
- Backward compatibility
- Gradual migration path

**Priority:** P2 (Medium priority, high impact)
**Timeline:** 2-3 days for Phase 1

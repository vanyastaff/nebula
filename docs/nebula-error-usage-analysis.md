# Nebula-Error Usage Analysis

**Date**: 2025-10-09
**Purpose**: Analyze how `nebula-error` is used across the workspace and identify gaps

## Executive Summary

`nebula-error` is used by **13 crates** in the Nebula workspace. Analysis reveals:
- ✅ Core error infrastructure is solid (0 warnings)
- ⚠️ **Some crates define their own error types instead of using ErrorKind variants**
- ⚠️ **Missing error kinds for domain-specific operations**
- ⚠️ **Inconsistent conversion patterns**

## Current Usage Map

### Direct Dependencies (13 crates)

1. **nebula-memory** - Memory management errors
2. **nebula-expression** - Expression evaluation errors
3. **nebula-parameter** - Parameter handling errors
4. **nebula-validator** - Validation errors
5. **nebula-resource** - Resource management errors
6. **nebula-credential** - Credential management errors
7. **nebula-config** - Configuration errors
8. **nebula-system** - System-level errors
9. **nebula-resilience** - Resilience/retry errors
10. **nebula-log** - Logging errors
11. **nebula-value** - Value type errors
12. **nebula-core** - Core functionality errors
13. **root crate** - Workspace-level integration

---

## Problem Analysis

### Pattern 1: Crates Define Their Own Error Enums

Many crates define comprehensive custom error enums instead of using `ErrorKind` variants:

#### ❌ **nebula-memory** (699 LOC error.rs)
```rust
pub enum MemoryErrorCode {
    AllocationFailed,      // Maps to SystemError::ResourceExhausted
    InvalidLayout,         // Maps to SystemError::ResourceExhausted
    PoolExhausted,         // Maps to SystemError::ResourceExhausted
    ArenaExhausted,        // Maps to SystemError::ResourceExhausted
    CacheMiss,             // Maps to SystemError::ResourceExhausted
    BudgetExceeded,        // Maps to SystemError::ResourceExhausted
    // ... 22 total variants ALL map to same SystemError!
}
```

**Problem**:
- 22 distinct error codes all forced into `SystemError::ResourceExhausted`
- Loss of type information at NebulaError level
- Custom `MemoryError` wrapper adds complexity

#### ❌ **nebula-credential** (448 LOC error.rs)
```rust
pub enum CredentialError {
    NotFound { id },               // Has dedicated method ✅
    Expired { id },                // Maps to credential_invalid
    AuthenticationFailed,          // Has dedicated method ✅
    RefreshFailed,                 // Maps to credential_invalid
    StorageFailed,                 // Maps to database
    NetworkFailed,                 // Has dedicated method ✅
    CacheFailed,                   // Maps to internal
    LockFailed,                    // Maps to internal
    PermissionDenied,              // Has dedicated method ✅
    // ... 18 total variants
}
```

**Problem**:
- Mixes storage/infrastructure errors with credential-specific errors
- Some have dedicated constructors, others improvised
- Inconsistent mapping strategy

#### ❌ **nebula-parameter** (267 LOC error.rs)
```rust
pub enum ParameterError {
    InvalidKeyFormat,          // Maps to validation
    NotFound { key },          // Maps to not_found ✅
    AlreadyExists { key },     // Maps to validation
    DeserializationError,      // Maps to validation
    SerializationError,        // Maps to internal
    InvalidType,               // Maps to validation
    ValidationError,           // Maps to validation
    MissingValue,              // Maps to validation
    InvalidValue,              // Maps to validation
}
```

**Observation**: Better design - most errors are client validation errors

#### ❌ **nebula-resource** (378 LOC error.rs)
```rust
pub enum ResourceError {
    Configuration,                 // Client error
    Initialization,                // System error
    Unavailable { retryable },     // Server error
    HealthCheck { attempt },       // Server error
    MissingCredential,             // Client error
    Cleanup,                       // System error
    Timeout,                       // System error
    CircuitBreakerOpen,            // Server error (retryable)
    PoolExhausted,                 // Server error (retryable)
    DependencyFailure,             // Server error
    CircularDependency,            // Client error (config)
    InvalidStateTransition,        // Client error
    Internal,                      // Server error
}
```

**Problem**: ALL map to `SystemError::ResourceExhausted` - loss of semantics!

#### ✅ **nebula-expression** (99 LOC error.rs)
```rust
pub trait ExpressionErrorExt {
    fn expression_syntax_error(...) -> Self;
    fn expression_parse_error(...) -> Self;
    fn expression_eval_error(...) -> Self;
    fn expression_type_error(...) -> Self;
    fn expression_variable_not_found(...) -> Self;
    fn expression_function_not_found(...) -> Self;
    // ...
}
```

**Best Practice**: Uses extension trait pattern, leverages existing methods

#### ❌ **nebula-config** (462 LOC error.rs)
```rust
pub enum ConfigError {
    FileNotFound,              // Maps to not_found ✅
    FileReadError,             // Maps to internal
    ParseError,                // Maps to validation ✅
    ValidationError,           // Maps to validation ✅
    SourceError,               // Maps to internal
    EnvVarNotFound,            // Maps to not_found ✅
    EnvVarParseError,          // Maps to validation ✅
    ReloadError,               // Maps to internal
    WatchError,                // Maps to internal
    MergeError,                // Maps to internal
    TypeError,                 // Maps to validation ✅
    PathError,                 // Maps to validation
    FormatNotSupported,        // Maps to validation ✅
    EncryptionError,           // Maps to internal
    DecryptionError,           // Maps to internal
}
```

**Observation**: Reasonable mapping, mostly client/validation errors

---

## Missing Error Kinds

Based on usage analysis, these error scenarios exist but lack dedicated `ErrorKind` variants:

### 1. **Memory Errors** (Missing)
Current state: All 22 memory errors forced into `SystemError::ResourceExhausted`

**Needed**:
```rust
#[non_exhaustive]
pub enum MemoryError {
    AllocationFailed { size: usize, align: usize },
    PoolExhausted { pool_id: String, capacity: usize },
    ArenaExhausted { arena_id: String, requested: usize, available: usize },
    CacheMiss { key: String },
    BudgetExceeded { used: usize, limit: usize },
    InvalidLayout { reason: String },
    Corruption { component: String, details: String },
}
```

**Impact**: 699 LOC in nebula-memory/error.rs could be reduced significantly

---

### 2. **Resource Management Errors** (Missing)
Current state: All map to `SystemError::ResourceExhausted`

**Needed**:
```rust
#[non_exhaustive]
pub enum ResourceError {
    Unavailable { resource_id: String, reason: String, retryable: bool },
    HealthCheckFailed { resource_id: String, attempt: u32, reason: String },
    CircuitBreakerOpen { resource_id: String, retry_after_ms: Option<u64> },
    PoolExhausted { resource_id: String, current_size: usize, max_size: usize, waiters: usize },
    DependencyFailure { resource_id: String, dependency_id: String, reason: String },
    CircularDependency { cycle: String },
    InvalidStateTransition { resource_id: String, from: String, to: String },
}
```

**Impact**: 378 LOC in nebula-resource/error.rs could be removed

---

### 3. **Configuration Errors** (Partially Covered)
Current state: Maps to `not_found`, `validation`, `internal`

**Assessment**: ✅ Current mapping is acceptable
- FileNotFound → not_found ✅
- ParseError → validation ✅
- ValidationError → validation ✅

**Potential Improvement**: Add dedicated `ConfigError` variant for clarity

---

### 4. **Parameter Errors** (Mostly Covered)
Current state: Maps to `not_found`, `validation`, `internal`

**Assessment**: ✅ Current mapping is acceptable for client errors

---

### 5. **Expression Errors** (Well Handled)
Current state: Extension trait using existing methods

**Assessment**: ✅ Best practice - no changes needed

---

## Gap Analysis Summary

| Domain | LOC in Custom Error | Current Mapping | Recommendation |
|--------|-------------------|-----------------|----------------|
| **Memory** | 699 | SystemError::ResourceExhausted | ⚠️ Add MemoryError kind |
| **Resource** | 378 | SystemError::ResourceExhausted | ⚠️ Add ResourceError kind |
| **Credential** | 448 | Mixed (good methods exist) | ✅ OK - use existing methods |
| **Config** | 462 | not_found/validation/internal | ✅ OK - good mapping |
| **Parameter** | 267 | not_found/validation | ✅ OK - good mapping |
| **Expression** | 99 | Extension trait pattern | ✅ BEST PRACTICE |

---

## Recommendations

### Priority 1: Add Missing ErrorKind Variants

Add to `crates/nebula-error/src/kinds/mod.rs`:

```rust
#[non_exhaustive]
#[derive(Error, Debug, Clone, Serialize, Deserialize)]
pub enum ErrorKind {
    Client(#[from] ClientError),
    Server(#[from] ServerError),
    System(#[from] SystemError),
    Workflow(#[from] WorkflowError),
    Node(#[from] NodeError),
    Trigger(#[from] TriggerError),
    Connector(#[from] ConnectorError),
    Credential(#[from] CredentialError),
    Execution(#[from] ExecutionError),

    // NEW: Domain-specific error kinds
    Memory(#[from] MemoryError),        // ⭐ NEW
    Resource(#[from] ResourceError),    // ⭐ NEW
    Config(#[from] ConfigError),        // ⭐ NEW (optional)
}
```

### Priority 2: Create New Error Type Files

**File**: `crates/nebula-error/src/kinds/memory.rs`
```rust
/// Memory management errors
#[non_exhaustive]
#[derive(Error, Debug, Clone, Serialize, Deserialize)]
pub enum MemoryError {
    #[error("Memory allocation failed: {size} bytes with {align} alignment")]
    AllocationFailed { size: usize, align: usize },

    #[error("Memory pool exhausted: {pool_id} ({capacity} capacity)")]
    PoolExhausted { pool_id: String, capacity: usize },

    #[error("Memory arena exhausted: {arena_id} (requested {requested}, available {available})")]
    ArenaExhausted {
        arena_id: String,
        requested: usize,
        available: usize,
    },

    #[error("Cache miss: {key}")]
    CacheMiss { key: String },

    #[error("Memory budget exceeded: used {used}, limit {limit}")]
    BudgetExceeded { used: usize, limit: usize },

    #[error("Invalid memory layout: {reason}")]
    InvalidLayout { reason: String },

    #[error("Memory corruption detected in {component}: {details}")]
    Corruption { component: String, details: String },

    #[error("Invalid memory configuration: {reason}")]
    InvalidConfig { reason: String },

    #[error("Memory system initialization failed: {reason}")]
    InitializationFailed { reason: String },
}

impl ErrorCode for MemoryError {
    fn error_code(&self) -> &str {
        match self {
            MemoryError::AllocationFailed { .. } => "MEMORY_ALLOCATION_FAILED",
            MemoryError::PoolExhausted { .. } => "MEMORY_POOL_EXHAUSTED",
            MemoryError::ArenaExhausted { .. } => "MEMORY_ARENA_EXHAUSTED",
            MemoryError::CacheMiss { .. } => "MEMORY_CACHE_MISS",
            MemoryError::BudgetExceeded { .. } => "MEMORY_BUDGET_EXCEEDED",
            MemoryError::InvalidLayout { .. } => "MEMORY_INVALID_LAYOUT",
            MemoryError::Corruption { .. } => "MEMORY_CORRUPTION",
            MemoryError::InvalidConfig { .. } => "MEMORY_INVALID_CONFIG",
            MemoryError::InitializationFailed { .. } => "MEMORY_INITIALIZATION_FAILED",
        }
    }

    fn error_category(&self) -> &'static str {
        "MEMORY"
    }
}

impl MemoryError {
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            MemoryError::AllocationFailed { .. }
                | MemoryError::PoolExhausted { .. }
                | MemoryError::ArenaExhausted { .. }
        )
    }
}
```

**File**: `crates/nebula-error/src/kinds/resource.rs`
```rust
/// Resource management errors
#[non_exhaustive]
#[derive(Error, Debug, Clone, Serialize, Deserialize)]
pub enum ResourceError {
    #[error("Resource unavailable: {resource_id} - {reason}")]
    Unavailable {
        resource_id: String,
        reason: String,
        retryable: bool,
    },

    #[error("Health check failed for {resource_id} (attempt {attempt}): {reason}")]
    HealthCheckFailed {
        resource_id: String,
        attempt: u32,
        reason: String,
    },

    #[error("Circuit breaker open for {resource_id}")]
    CircuitBreakerOpen {
        resource_id: String,
        retry_after_ms: Option<u64>,
    },

    #[error("Resource pool exhausted: {resource_id} ({current_size}/{max_size}, {waiters} waiting)")]
    PoolExhausted {
        resource_id: String,
        current_size: usize,
        max_size: usize,
        waiters: usize,
    },

    #[error("Dependency failure: {resource_id} depends on {dependency_id} - {reason}")]
    DependencyFailure {
        resource_id: String,
        dependency_id: String,
        reason: String,
    },

    #[error("Circular dependency detected: {cycle}")]
    CircularDependency { cycle: String },

    #[error("Invalid state transition for {resource_id}: {from} -> {to}")]
    InvalidStateTransition {
        resource_id: String,
        from: String,
        to: String,
    },

    #[error("Resource initialization failed: {resource_id} - {reason}")]
    InitializationFailed {
        resource_id: String,
        reason: String,
    },

    #[error("Resource cleanup failed: {resource_id} - {reason}")]
    CleanupFailed {
        resource_id: String,
        reason: String,
    },
}

impl ErrorCode for ResourceError {
    fn error_code(&self) -> &str {
        match self {
            ResourceError::Unavailable { .. } => "RESOURCE_UNAVAILABLE",
            ResourceError::HealthCheckFailed { .. } => "RESOURCE_HEALTH_CHECK_FAILED",
            ResourceError::CircuitBreakerOpen { .. } => "RESOURCE_CIRCUIT_BREAKER_OPEN",
            ResourceError::PoolExhausted { .. } => "RESOURCE_POOL_EXHAUSTED",
            ResourceError::DependencyFailure { .. } => "RESOURCE_DEPENDENCY_FAILURE",
            ResourceError::CircularDependency { .. } => "RESOURCE_CIRCULAR_DEPENDENCY",
            ResourceError::InvalidStateTransition { .. } => "RESOURCE_INVALID_STATE_TRANSITION",
            ResourceError::InitializationFailed { .. } => "RESOURCE_INITIALIZATION_FAILED",
            ResourceError::CleanupFailed { .. } => "RESOURCE_CLEANUP_FAILED",
        }
    }

    fn error_category(&self) -> &'static str {
        "RESOURCE"
    }
}

impl ResourceError {
    pub fn is_retryable(&self) -> bool {
        match self {
            ResourceError::Unavailable { retryable, .. } => *retryable,
            ResourceError::CircuitBreakerOpen { .. } => true,
            ResourceError::PoolExhausted { .. } => true,
            _ => false,
        }
    }
}
```

### Priority 3: Add Convenience Constructors

Add to `crates/nebula-error/src/core/error.rs`:

```rust
impl NebulaError {
    // Memory errors
    pub fn memory_allocation_failed(size: usize, align: usize) -> Self {
        Self::new(ErrorKind::Memory(MemoryError::AllocationFailed { size, align }))
            .with_retry_info(true, Some(Duration::from_millis(100)))
    }

    pub fn memory_pool_exhausted(pool_id: impl Into<String>, capacity: usize) -> Self {
        Self::new(ErrorKind::Memory(MemoryError::PoolExhausted {
            pool_id: pool_id.into(),
            capacity,
        }))
        .with_retry_info(true, Some(Duration::from_millis(500)))
    }

    pub fn memory_cache_miss(key: impl Into<String>) -> Self {
        Self::new(ErrorKind::Memory(MemoryError::CacheMiss {
            key: key.into(),
        }))
        .with_retry_info(false, None)
    }

    // Resource errors
    pub fn resource_unavailable(
        resource_id: impl Into<String>,
        reason: impl Into<String>,
        retryable: bool,
    ) -> Self {
        Self::new(ErrorKind::Resource(ResourceError::Unavailable {
            resource_id: resource_id.into(),
            reason: reason.into(),
            retryable,
        }))
        .with_retry_info(retryable, Some(Duration::from_secs(5)))
    }

    pub fn resource_circuit_breaker_open(
        resource_id: impl Into<String>,
        retry_after_ms: Option<u64>,
    ) -> Self {
        let retry_after = retry_after_ms.map(Duration::from_millis);
        Self::new(ErrorKind::Resource(ResourceError::CircuitBreakerOpen {
            resource_id: resource_id.into(),
            retry_after_ms,
        }))
        .with_retry_info(true, retry_after)
    }

    pub fn resource_pool_exhausted(
        resource_id: impl Into<String>,
        current_size: usize,
        max_size: usize,
        waiters: usize,
    ) -> Self {
        Self::new(ErrorKind::Resource(ResourceError::PoolExhausted {
            resource_id: resource_id.into(),
            current_size,
            max_size,
            waiters,
        }))
        .with_retry_info(true, Some(Duration::from_millis(200)))
    }
}
```

### Priority 4: Update Error Codes

Add to `crates/nebula-error/src/kinds/codes.rs`:

```rust
// Memory error codes
pub const MEMORY_ALLOCATION_FAILED: &str = "MEMORY_ALLOCATION_FAILED";
pub const MEMORY_POOL_EXHAUSTED: &str = "MEMORY_POOL_EXHAUSTED";
pub const MEMORY_ARENA_EXHAUSTED: &str = "MEMORY_ARENA_EXHAUSTED";
pub const MEMORY_CACHE_MISS: &str = "MEMORY_CACHE_MISS";
pub const MEMORY_BUDGET_EXCEEDED: &str = "MEMORY_BUDGET_EXCEEDED";
pub const MEMORY_INVALID_LAYOUT: &str = "MEMORY_INVALID_LAYOUT";
pub const MEMORY_CORRUPTION: &str = "MEMORY_CORRUPTION";
pub const MEMORY_INVALID_CONFIG: &str = "MEMORY_INVALID_CONFIG";
pub const MEMORY_INITIALIZATION_FAILED: &str = "MEMORY_INITIALIZATION_FAILED";

// Resource error codes
pub const RESOURCE_UNAVAILABLE: &str = "RESOURCE_UNAVAILABLE";
pub const RESOURCE_HEALTH_CHECK_FAILED: &str = "RESOURCE_HEALTH_CHECK_FAILED";
pub const RESOURCE_CIRCUIT_BREAKER_OPEN: &str = "RESOURCE_CIRCUIT_BREAKER_OPEN";
pub const RESOURCE_POOL_EXHAUSTED: &str = "RESOURCE_POOL_EXHAUSTED";
pub const RESOURCE_DEPENDENCY_FAILURE: &str = "RESOURCE_DEPENDENCY_FAILURE";
pub const RESOURCE_CIRCULAR_DEPENDENCY: &str = "RESOURCE_CIRCULAR_DEPENDENCY";
pub const RESOURCE_INVALID_STATE_TRANSITION: &str = "RESOURCE_INVALID_STATE_TRANSITION";
pub const RESOURCE_INITIALIZATION_FAILED: &str = "RESOURCE_INITIALIZATION_FAILED";
pub const RESOURCE_CLEANUP_FAILED: &str = "RESOURCE_CLEANUP_FAILED";

// Category constants
pub const CATEGORY_MEMORY: &str = "MEMORY";
pub const CATEGORY_RESOURCE: &str = "RESOURCE";
```

### Priority 5: Refactor Dependent Crates

**nebula-memory/src/core/error.rs**: Simplify to use `NebulaError` directly
```rust
pub type MemoryResult<T> = Result<T, NebulaError>;

// Remove MemoryError struct, MemoryErrorCode enum (saves 600+ LOC)
// Use NebulaError::memory_* methods directly
```

**nebula-resource/src/core/error.rs**: Simplify to use `NebulaError` directly
```rust
pub type ResourceResult<T> = Result<T, NebulaError>;

// Remove ResourceError enum (saves 300+ LOC)
// Use NebulaError::resource_* methods directly
```

---

## Expected Benefits

### Code Reduction
- **nebula-memory**: -600 LOC (699 → ~100)
- **nebula-resource**: -300 LOC (378 → ~80)
- **nebula-credential**: -200 LOC (better mapping to existing methods)
- **Total**: ~1,100 LOC reduction

### Type Safety
- Memory errors have dedicated variant (not all SystemError::ResourceExhausted)
- Resource errors have dedicated variant with proper context
- Better error discrimination at API boundaries

### Consistency
- All crates use centralized error types
- Uniform error code naming
- Consistent retry semantics

### Performance
- Fewer intermediate error type conversions
- Less heap allocation (remove wrapper types)
- Faster error matching

---

## Migration Strategy

### Phase 1: Add New Error Kinds (Week 1)
1. Add `MemoryError` variant to `ErrorKind`
2. Add `ResourceError` variant to `ErrorKind`
3. Create `kinds/memory.rs` and `kinds/resource.rs`
4. Add convenience constructors to `NebulaError`
5. Add error codes to `kinds/codes.rs`
6. Ensure all tests pass (should be backward compatible)

### Phase 2: Update nebula-memory (Week 2)
1. Replace `MemoryErrorCode` with direct `NebulaError` usage
2. Update all error constructors
3. Update tests
4. Run benchmarks to verify no performance regression

### Phase 3: Update nebula-resource (Week 2)
1. Replace `ResourceError` enum with direct `NebulaError` usage
2. Update all error constructors
3. Update tests

### Phase 4: Update nebula-credential (Week 3)
1. Update `From<CredentialError>` implementation to use existing methods consistently
2. Document best practices
3. Update tests

### Phase 5: Documentation & Validation (Week 3)
1. Update error handling documentation
2. Create migration guide for other crates
3. Run full test suite
4. Run benchmarks
5. Create comprehensive error examples

---

## Conclusion

The `nebula-error` crate has solid foundations but suffers from:
1. **Missing domain-specific error kinds** (Memory, Resource)
2. **Inconsistent usage patterns** across dependent crates
3. **Unnecessary code duplication** (1000+ LOC of custom error types)

By adding `MemoryError` and `ResourceError` variants to `ErrorKind`, we can:
- Reduce code by ~1,100 LOC
- Improve type safety
- Ensure consistent error handling
- Simplify maintenance

**Recommendation**: Proceed with implementation following the 5-week migration plan outlined above.

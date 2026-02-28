# Interactions

## Ecosystem Map

### Existing Crates

| Crate | Relationship | Description |
|-------|-------------|-------------|
| `nebula-core` | Upstream (required) | Identifiers and scope system |
| `nebula-system` | Upstream (required) | Cross-platform utilities, memory pressure detection |
| `nebula-log` | Upstream (optional) | Structured logging via `logging` feature |
| `nebula-expression` | Downstream | Uses `cache` module for expression evaluation caching |

### Planned Consumers

- `nebula-engine` - Will use allocators/pools for workflow execution memory management
- `nebula-runtime` - Expected to integrate monitoring and budget modules
- `nebula-action` - May use object pools for action instance reuse

## Upstream Dependencies

### nebula-core

- **Why needed**: Provides core identity types (`ScopeId`, etc.) and foundational traits
- **Hard contract**: Stable identifier types for memory region tagging
- **Fallback**: None (required dependency)

### nebula-system

- **Why needed**: Platform-abstracted memory info (`MemoryInfo`, `MemoryPressure`)
- **Hard contract**: `memory::current()` returns valid system memory state
- **Fallback**: Degraded monitoring (no system pressure awareness)

### nebula-log (optional)

- **Why needed**: Structured logging for allocation events, warnings, errors
- **Hard contract**: Standard log macros (`info!`, `warn!`, `error!`, `debug!`)
- **Fallback**: No logging output when feature disabled

## Downstream Consumers

### nebula-expression

- **Expectations**: `ComputeCache` from `cache` module for caching parsed/evaluated expressions
- **Contract**: `CacheConfig`, `ComputeCache<K, V>` API stability
- **Usage pattern**: LRU-evicted cache for repeated expression lookups

## Interaction Matrix

| This crate <-> Other | Direction | Contract | Sync/Async | Failure Handling | Notes |
|---------------------|-----------|----------|------------|------------------|-------|
| memory -> core | upstream | type imports | sync | N/A (compile-time) | No runtime calls |
| memory -> system | upstream | `memory::current()` | sync | returns defaults on failure | Used by monitoring module |
| memory -> log | upstream | log macros | sync | silent when disabled | Feature-gated |
| expression -> memory | downstream | `ComputeCache` API | sync | cache miss returns None | Expression caching |

## Runtime Sequence

### Memory Initialization

1. Application calls `nebula_memory::init()`
2. `GlobalAllocatorManager::init()` initializes allocator registry
3. Optional: `MemoryMonitor` starts tracking system pressure

### Typical Allocation Flow

1. Consumer creates pool/arena/allocator
2. Allocation requests routed to chosen strategy
3. Stats/monitoring updated if features enabled
4. On pressure events, `PressureAction` informs consumer

### Shutdown

1. Pools/arenas drain or reset
2. `nebula_memory::shutdown()` performs cleanup
3. Statistics can be exported before shutdown

## Cross-Crate Ownership

| Responsibility | Owner |
|---------------|-------|
| Memory allocation strategies | `nebula-memory` |
| System memory detection | `nebula-system` |
| Structured logging | `nebula-log` |
| Identity types | `nebula-core` |
| Runtime orchestration (future) | `nebula-runtime` |

## Failure Propagation

### Allocation Failures

- `MemoryError::AllocationFailed` propagates to caller
- Caller decides retry/fallback strategy
- Pool exhaustion (`PoolExhausted`) is marked retryable

### Pressure Events

- `MemoryMonitor` emits `PressureAction` recommendations
- Callers should respect `DenyLargeAllocations` when returned
- Emergency actions logged via `nebula-log` if enabled

### Where Retries Apply

- Pool/cache exhaustion: consumer may retry after cleanup
- Arena exhaustion: typically requires reset or new arena

### Where Retries Are Forbidden

- `InvalidLayout`, `InvalidAlignment`: configuration bugs, not transient
- `Corruption`: indicates memory safety violation

## Versioning and Compatibility

### Compatibility Promise

- Public API (`prelude`, re-exports) follows semver
- Feature flags are additive (enabling a feature does not break existing code)
- Internal modules (`allocator::sealed`) are not public API

### Breaking-Change Protocol

1. Deprecation warning in CHANGELOG for one minor release
2. Migration guide in this document
3. Breaking change in next major release

### Deprecation Window

- Minimum one minor release with deprecation warnings
- Deprecated APIs remain functional during window

## Contract Tests Needed

- [ ] `MemoryMonitor` correctly reads `nebula-system::memory::current()`
- [ ] `ComputeCache` eviction follows LRU policy under memory pressure
- [ ] `ObjectPool` returns values to pool on drop
- [ ] `Arena::reset()` invalidates all prior allocations
- [ ] Feature combinations (`stats` + `monitoring`) compile and function together

# Interactions

## Ecosystem Map (Current + Planned)

### Existing Crates

| Crate | Relationship | Description |
|-------|-------------|-------------|
| `nebula-core` | None (no direct dep) | — |
| `nebula-memory` | Downstream | Uses `memory::current()`, `MemoryPressure` for monitoring |
| `nebula-log` | None | — |
| `nebula-resilience` | Potential consumer | Could use pressure for circuit breaker |
| `nebula-resource` | Potential consumer | Resource limits, scopes |
| `nebula-runtime` | Potential consumer | Workflow execution limits |
| `nebula-action` | Potential consumer | Action resource awareness |

### Planned Crates

- **nebula-metrics:** May consume system info for Prometheus/OpenTelemetry
  - Expected boundary: system exposes raw counters; metrics crate formats/emits

## Downstream Consumers

### nebula-memory

- **Expectations:** `memory::current()` returns valid system memory state; `MemoryPressure` for backpressure
- **Contract:** Sync; returns defaults when sysinfo unavailable (minimal feature)
- **Fallback:** Degraded monitoring when system crate unavailable

## Upstream Dependencies

| Crate | Why needed | Hard contract | Fallback |
|-------|-------------|---------------|----------|
| `sysinfo` | Cross-platform system info | `System`, `Networks`, `Disks` APIs | Feature-gated; minimal build without |
| `region` | Memory protection, lock/unlock | `alloc`, `protect`, `lock`, `unlock` | Feature-gated |
| `libc` | CPU affinity, statvfs | POSIX syscalls | Platform-specific |
| `winapi` | Windows support | — | Windows-only |
| `parking_lot` | RwLock for caching | — | — |
| `once_cell` | Lazy init | — | — |

## Interaction Matrix

| This crate <-> Other | Direction | Contract | Sync/Async | Failure handling | Notes |
|---------------------|-----------|----------|------------|------------------|-------|
| memory -> system | upstream | `memory::current()` | sync | returns defaults | Used by monitoring |
| system -> sysinfo | upstream | trait/API | sync | SystemError | Platform errors mapped |
| system -> region | upstream | alloc/protect | sync | SystemError | Unsafe blocks |

## Runtime Sequence

1. Application calls `nebula_system::init()`
2. `SystemInfo::get()` triggers lazy init of sysinfo backend
3. Consumers call `memory::pressure()`, `cpu::usage()`, etc.
4. `refresh()` updates caches when fresh data needed

## Cross-Crate Ownership

| Responsibility | Owner |
|---------------|-------|
| System info gathering | `nebula-system` |
| Memory allocation strategies | `nebula-memory` |
| Pressure-based backpressure | Consumer (engine, runtime) |
| Metrics export | Future `nebula-metrics` or telemetry |

## Failure Propagation

- **How failures bubble up:** `SystemResult<T>`; caller handles
- **Where retries apply:** Caller may retry transient failures (e.g., parse errors)
- **Where retries forbidden:** `PermissionDenied`, `FeatureNotSupported` (config/privilege)

## Versioning and Compatibility

- **Compatibility promise:** Public API semver; feature flags additive
- **Breaking-change protocol:** Deprecation 2 minor versions; migration guide in MIGRATION.md
- **Deprecation window:** Minimum 2 minor releases

## Contract Tests Needed

- [ ] `nebula-memory` correctly reads `memory::current()` under various system states
- [ ] `MemoryPressure` thresholds match documented behavior
- [ ] `init()` idempotent; safe to call multiple times
- [ ] Feature combinations compile and function

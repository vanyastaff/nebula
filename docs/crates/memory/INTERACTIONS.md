# Interactions

## Ecosystem Map (Current + Planned)

### Existing crates

- `core`: shared platform types and conventions
- `system`: host memory info and pressure signals
- `log`: structured logging (feature `logging`)
- `runtime`/`engine`/`worker`: orchestration; applies memory policy during execution
- `action`: consumer of pool/cache/arena primitives in hot-path operations
- `resilience`: retry/circuit/backpressure policy applied on top of `MemoryError`
- `config`: source of limits/capacities/policies for memory module initialization
- `metrics`/`telemetry`: export and visualization of runtime metrics

### Planned crates

- `memory-adapters`:
  - why: integration patterns for memory policies tailored to specific runtime profiles
  - boundary: adapters only; no changes to core memory contracts

## Downstream Consumers

- `runtime/engine`:
  - expectations: predictable allocation/reuse primitives and pressure signals for workload scheduling
- `action`:
  - expectations: low-latency pools/caches for repeated hot-path operations

## Upstream Dependencies

- `system`:
  - why: reads host memory info and pressure level
  - hard contract: correct `MemoryInfo`/`MemoryPressure` data
  - fallback if unavailable: monitoring path degrades; base allocator/pool functions remain
- `log` (optional, feature `logging`):
  - why: diagnostic tracing for pressure events, errors, and lifecycle
  - hard contract: standard logging macros
  - fallback if unavailable: no-op logging

## Interaction Matrix

| This crate <-> Other crate | Direction | Contract | Sync/Async | Failure handling | Notes |
|---|---|---|---|---|---|
| memory <-> system | in | memory info + pressure states | sync | degrade monitoring path | monitoring module |
| memory <-> runtime/engine | out | alloc/pool/cache/budget APIs | sync/async | runtime chooses retry/fallback | critical path |
| memory <-> action | out | reuse primitives for hot execution | sync/async | caller handles exhaustion | workload-specific configs |
| memory <-> resilience | in/out | retryability semantics via `MemoryError` | sync | retries only for retryable classes | policy ownership outside memory |
| memory <-> config | in | tuned limits and capacity values | sync | invalid config fails startup | bootstrap boundary |
| memory <-> log/telemetry | out | structured diagnostics and metrics | sync/async | observability failures non-fatal | additive |

## Runtime Sequence

1. Runtime loads memory config and initializes memory subsystem via `nebula_memory::init()`.
2. Workload path selects primitives by workload type: pool → reuse-first; arena → short-lived bulk; cache → memoization; budget → bounded multi-tenant.
3. Monitoring and stats modules observe pressure and usage metrics.
4. Caller applies policy decisions (throttle/retry/fallback) based on `MemoryError` variants and monitoring signals.
5. Shutdown calls `nebula_memory::shutdown()` to drain/clean memory-owned structures.

## Cross-Crate Ownership

- **memory owns:** allocation/reuse contracts; error taxonomy; pressure classification
- **runtime/engine own:** policy orchestration; when to throttle/retry/shed load
- **persistence:** not owned by memory
- **retry/backpressure:** `resilience` + caller; memory only classifies errors as retryable/fatal
- **security:** memory safety invariants in `memory`; request authn/authz in upper layers

## Failure Propagation

- typed `MemoryError` variants returned to all consumers; `#[non_exhaustive]`
- retryable: `PoolExhausted`, `ArenaExhausted`, `CacheOverflow`, `BudgetExceeded`, `CacheMiss`
- fatal (no retry): `Corruption`, `InvalidLayout`, `InvalidAlignment`, `InitializationFailed`, `InvalidState`
- caller/resilience layer decides retry strategy for retryable classes
- `Corruption` must never be retried; escalate to operator

## Versioning and Compatibility

- **compatibility promise:** stable `MemoryError` semantics and key API contracts within major version
- **breaking-change protocol:** proposal → decision → migration note → major release
- **deprecation window:** minimum one minor version for non-critical removals

## Contract Tests Needed

- system pressure mapping: `MemoryPressure` → monitoring path
- runtime integration: budget + pool behavior under concurrent load
- retryability mapping: resilience wrappers use correct error classification
- feature-matrix compile tests: `default`, `full`, selective feature combos

# Security

## Threat Model

### Assets

- **Allocated memory regions**: workflow data, credentials (in transit), user payloads
- **Pool/cache contents**: may contain sensitive computed values
- **Statistics data**: allocation patterns could reveal workload characteristics

### Trust Boundaries

| Boundary | Description |
|----------|-------------|
| Allocator internals | Unsafe code isolated in `allocator/` modules |
| Pool object lifecycle | Objects must be properly reset before reuse |
| Cache key space | Keys should not leak sensitive data |
| System memory queries | Relies on `nebula-system` for accurate info |

### Attacker Capabilities (Assumed)

- Can trigger high allocation volume (DoS via resource exhaustion)
- May attempt to read memory from previous allocations (use-after-free)
- Could exploit alignment/size miscalculations in unsafe code

## Security Controls

### Memory Isolation

- **Arena scopes**: Allocations within an arena are logically grouped
- **Pool reset**: `Resettable` trait requires explicit reset semantics
- **Typed allocators**: `TypedAllocator<T>` prevents type confusion

### Unsafe Code Containment

- All unsafe blocks localized in:
  - `allocator/bump/`, `allocator/pool/`, `allocator/stack/`
  - `arena/allocator.rs`, `arena/arena.rs`
- Public API surfaces use safe wrappers
- `sealed` module prevents external trait implementations

### Input Validation

- Layout validation: `Layout::from_size_align()` checks enforced
- Size overflow protection: `CheckedArithmetic` trait in `utils`
- Alignment constraints: Power-of-two alignment enforced

### No Secret Handling

- `nebula-memory` does not handle secrets directly
- Credential caching belongs to `nebula-credential` (separate crate)
- Cache keys should be hashed/opaque for sensitive lookups

## Abuse Cases

### Case: Memory Exhaustion DoS

- **Attack**: Flood allocation requests to exhaust pool/arena
- **Prevention**: Budget module (`BudgetConfig`) limits total allocation
- **Detection**: `MemoryMonitor` tracks pressure, logs warnings
- **Response**: `PressureAction::DenyLargeAllocations` blocks new requests

### Case: Use-After-Free via Pool

- **Attack**: Access pooled object after return
- **Prevention**: `PooledValue<T>` uses RAII; reference invalid after drop
- **Detection**: Debug builds can enable allocation tracking
- **Response**: Undefined behavior if bypassed; Rust ownership prevents most cases

### Case: Information Leakage via Cache

- **Attack**: Probe cache to infer previous computations
- **Prevention**: TTL policies clear stale entries; partitioned caches isolate tenants
- **Detection**: Cache hit/miss ratios can indicate probing
- **Response**: Use randomized eviction or per-request caches for sensitive data

### Case: Alignment Exploitation

- **Attack**: Provide malformed layout to corrupt memory
- **Prevention**: `Layout` validation in all allocator entry points
- **Detection**: `InvalidAlignment` error returned for bad inputs
- **Response**: Allocation refused; no memory corruption

## Security Requirements

### Must-Have

- [ ] All unsafe code reviewed and documented with safety invariants
- [ ] No raw pointer exposure in public API
- [ ] `MemoryError::Corruption` logged at error level when detected
- [ ] Budget limits enforced before allocation in constrained environments

### Should-Have

- [ ] Optional zeroing of deallocated memory (feature flag)
- [ ] Allocation audit trail for debugging (via `stats` feature)
- [ ] Integration with system-level memory protections (future)

## Security Test Plan

### Static Analysis

- `cargo clippy --all-features -- -D warnings` catches unsafe patterns
- `cargo audit` checks dependency vulnerabilities
- Manual review of `unsafe` blocks in allocator modules

### Dynamic Tests

- Allocation/deallocation stress tests for memory leaks
- Concurrent access tests for thread-safe pools/caches
- Pressure simulation to verify budget enforcement

### Fuzz/Property Tests

- `proptest` for allocation size/alignment combinations
- Fuzz pool acquire/release sequences
- Random eviction policy behavior under cache churn

### CI Quality Gates

- All tests pass with `--all-features`
- No new `unsafe` blocks without documented safety comments
- Memory sanitizers (ASAN/MSAN) in CI for nightly builds (planned)

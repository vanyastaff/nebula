# Test Strategy

## Test Pyramid

- **Unit:** Primary. All modules have `#[cfg(test)]` tests: ID creation/parse/serde, scope containment, traits, types, error, keys, constants.
- **Integration:** None — core has no Nebula dependencies. Consumer crates test integration with core.
- **Contract:** Snapshot tests for serialized forms (P-004); ID/scope/error code stability.
- **End-to-end:** N/A — core is a library.

## Critical Invariants

- **ID type safety:** `ProjectId` and `UserId` are distinct; passing one where the other is expected must not compile.
- **Scope containment:** `Action` contained in `Execution`; `Execution` contained in `Workflow`; hierarchy is transitive.
- **PluginKey normalization:** Idempotent; output only `a-z` and `_`; max 64 chars.
- **CoreError classification:** `is_retryable`, `is_client_error`, `is_server_error` are mutually consistent.

## Scenario Matrix

| Scenario | Coverage |
|----------|----------|
| Happy path | ID v4/parse/nil, scope creation, context builder, OperationResult success |
| Retry path | `CoreError::is_retryable()` for Timeout, RateLimitExceeded, etc. |
| Cancellation path | N/A — core has no async/cancellation |
| Timeout path | N/A — core has no I/O |
| Upgrade/migration path | Snapshot tests for serde forms; MIGRATION.md for breaking changes |

## Tooling

- **Property testing:** Optional; `PluginKey` normalization, ID round-trip.
- **Fuzzing:** Optional; `PluginKey::new` with random input.
- **Benchmarks:** P-005 proposes criterion benches for ID parse/serialize hot paths.
- **CI quality gates:** `cargo test -p nebula-core`; `cargo clippy -p nebula-core`; `cargo fmt --check`.

## Exit Criteria

- **Coverage goals:** All public APIs have at least one test; critical invariants covered.
- **Flaky test budget:** Zero — core tests are deterministic.
- **Performance regression:** P-005 benchmarks; no regression without explicit approval.

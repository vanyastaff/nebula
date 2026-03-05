# Migration

## Versioning Policy

- **Package semver:** `nebula-resilience` follows semver. Patch releases are bug fixes, minor releases are additive, major releases may remove deprecated behavior.
- **Policy schema semver:** `ResiliencePolicy.metadata.version` is treated as policy schema version and follows `MAJOR.MINOR.PATCH`.
- **Compatibility window:** policy schema major must match runtime-supported major. Minor/patch additions are backward-compatible.
- **Deprecation window:** minimum two minor releases before removing deprecated fields/semantics, with `CHANGELOG.md` notice.

## Supported Policy Schema

- **Current schema major:** `1`.
- **Accepted versions:** `1.x.y`.
- **Rejected versions:** any schema with major `!= 1` must be migrated before rollout.
- **Parsing rule:** unknown fields are tolerated for forward-compatible reads, but runtime behavior is defined only for known fields.

## Compatibility Guarantees

### Policy Serialization Format Guarantees

These guarantees apply to `ResiliencePolicy`, `RetryPolicyConfig`, and `PolicyMetadata` while schema major is `1`.

- **Field name stability:** serialized field names are stable across `1.x` releases.
- **Type stability:** existing field types are not narrowed or made stricter in a backward-incompatible way within `1.x`.
- **Required-field stability:** fields required for successful deserialization in `1.x` remain required with compatible semantics.
- **Additive evolution only:** new fields may be added only in backward-compatible form (optional or with safe defaults).
- **No silent semantic inversion:** existing fields keep their meaning (for example retry delay units remain milliseconds).

Breaking any guarantee above requires a schema major bump and migration (`1.x -> 2.0`).

### Metrics Schema Guarantees

These guarantees apply to metrics emitted by the crate and snapshots returned by `MetricsCollector`.

- **Snapshot shape stability:** `MetricSnapshot` keeps fields `count`, `sum`, `min`, `max`, `avg` through `1.x`.
- **Unit stability:** duration metrics recorded through `record_duration` remain in milliseconds.
- **Key stability for core hooks:** existing metric key families are stable within `1.x`:
  - `retry.started`, `retry.success`, `retry.failure`, `retry.attempts`
  - `circuit_breaker.<service>.state.<state>`
  - `rate_limit.<service>.exceeded`
  - `bulkhead.<service>.capacity_reached`
  - `timeout.<operation>`
- **Additive expansion only:** new metric keys may be introduced in minors, but existing key families are not renamed/removed in `1.x`.

Any rename/removal of existing metric keys or snapshot fields requires a schema-major migration event and rollout notice.

## Policy Reload Semantics

Runtime reload is performed through `ResilienceManager::register_service` and is deterministic:

1. Incoming policy is validated first.
2. If validation fails, the existing service policy/components remain unchanged.
3. If component construction fails (for example invalid circuit breaker config), update is rejected and prior state is preserved.
4. Components missing in the new policy are removed (`circuit_breaker`, `bulkhead`).
5. Metrics for an already-registered service are preserved across reload.

This defines an **all-or-nothing apply contract** for runtime updates and prevents partial/ambiguous state.

## Breaking Changes

### 2026-03 Contract Updates (Rust 1.93 wave)

- **Bulkhead queue is now enforced at runtime**
  - **Old behavior:** `queue_size` was declarative only; waiting was effectively unbounded.
  - **New behavior:** `acquire()` rejects when wait queue reaches `queue_size` (`BulkheadFull`).
  - **Migration steps:** increase `queue_size` for high fan-out workloads, or handle `BulkheadFull` explicitly.

- **Bulkhead acquire timeout now enforced from config**
  - **Old behavior:** `BulkheadConfig.timeout` did not affect `acquire()`.
  - **New behavior:** waiting for permit respects `timeout`; expiration returns `Timeout`.
  - **Migration steps:** tune timeout per service SLO and treat `Timeout` as expected backpressure signal.

- **Manager now applies `circuit_breaker` config during registration**
  - **Old behavior:** manager always built default circuit breaker when config existed.
  - **New behavior:** manager uses provided config; invalid config skips breaker registration for that service.
  - **Migration steps:** validate policy configs during startup; monitor logs for skipped registrations.

- **Retry jitter flag now has effect**
  - **Old behavior:** `use_jitter` field was ignored in delay calculation.
  - **New behavior:** when enabled, delay is randomized in range `0..=computed_backoff`.
  - **Migration steps:** update deterministic tests to disable jitter (`use_jitter = false`) where exact delays are asserted.

- **Policy reload contract is strict and deterministic**
  - **Old behavior:** `register_service` could partially apply updates and keep stale components when policy changed.
  - **New behavior:** invalid updates are rejected; missing components are removed; no partial update state is committed.
  - **Migration steps:** validate policy payloads before publish and expect no-op on invalid reload attempts.

- **Policy schema governance introduced**
  - **Old behavior:** version field existed in metadata but had no explicit migration contract.
  - **New behavior:** schema major compatibility and migration expectations are documented and enforced operationally.
  - **Migration steps:** pin emitted policy versions to supported major and stage major upgrades via migration pipeline.

## Migration Strategy

### Minor/Patch Schema Upgrades (`1.x -> 1.y`)

- Keep old fields readable.
- Introduce new fields with defaults.
- Maintain behavior parity when new fields are absent.
- Roll out in place (no dual-write required).

### Major Schema Upgrade (`1.x -> 2.0`)

- Generate transformed policies offline (or in config service) before deployment.
- Validate transformed policies with crate validation logic in CI/staging.
- Deploy runtime that supports the new major only after transformed payloads are available.
- Keep rollback artifact for previous schema payloads.

### Recommended Migration Pipeline

1. **Export** active policies from config store.
2. **Transform** with a deterministic converter (`vN -> vN+1`).
3. **Validate** via `ResiliencePolicy::validate` in pre-deploy checks.
4. **Canary** reload on subset of services.
5. **Promote** globally after metrics/latency checks.
6. **Retire** deprecated fields after deprecation window.

## Rollout Plan

1. **Preparation:** update policy schema/order/cancellation; add compatibility layer if needed.
2. **Dual-run / feature-flag stage:** optional; run new behavior behind flag for validation.
3. **Cutover:** release with migration doc; consumers update config/code.
4. **Cleanup:** remove deprecated APIs after deprecation window.

## Rollback Plan

- **Trigger conditions:** critical regression; policy load failures; performance degradation.
- **Rollback steps:** revert to previous crate version; restore previous config if schema changed.
- **Data/state reconciliation:** resilience is stateless per execution; no persistent state to reconcile. Policy config in external store may need manual rollback.

## Validation Checklist

- **API compatibility checks:** `cargo check --workspace`; dependent crates build.
- **Integration checks:** `cargo test --workspace`; engine/runtime integration tests pass.
- **Performance checks:** `cargo bench -p nebula-resilience`; no regression beyond threshold.

## Operator Checklist

- Validate every reload payload before publish.
- Treat failed reload as no-op and alert for investigation.
- Monitor service metrics continuity during reload (counters should remain monotonic).
- Keep versioned policy snapshots for rollback.

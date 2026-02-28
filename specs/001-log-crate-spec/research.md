# Research: Nebula Log Production Hardening

## Decision 1: Configuration Precedence Contract

- **Decision**: Use deterministic precedence: explicit runtime config overrides environment-derived config, and environment-derived config overrides preset defaults.
- **Rationale**: This preserves operator intent while keeping `auto_init` predictable across environments.
- **Alternatives considered**:
  - Environment-first precedence: rejected because it can silently override explicit service configuration.
  - Preset-only behavior: rejected because it prevents deployment-specific control.

## Decision 2: Multi-Destination Failure Policy Baseline

- **Decision**: Implement true fanout and support three explicit policies: `FailFast`, `BestEffort`, `PrimaryWithFallback`; default to `BestEffort` for resilience.
- **Rationale**: `log` docs identify incomplete multi-writer behavior; best-effort default keeps observability available during partial failures.
- **Alternatives considered**:
  - Always fail-fast: rejected due to high outage risk from single sink failures.
  - First-writer-only behavior: rejected as incompatible with intended multi-destination semantics.

## Decision 3: Hook Hardening Model

- **Decision**: Keep panic isolation as mandatory and add bounded execution policy (inline default + optional bounded async/offload mode).
- **Rationale**: `telemetry` and `log` both treat observability as non-blocking/best-effort; bounded hooks prevent unbounded latency from third-party hook implementations.
- **Alternatives considered**:
  - Inline-only forever: rejected because slow hooks become tail-latency hazard.
  - Fully async-only always: rejected due to ordering/complexity risks for existing integrations.

## Decision 4: Metrics/Telemetry Alignment Boundary

- **Decision**: `nebula-log` remains logging/trace-centric; integration points align with `nebula-telemetry` event/metric semantics and planned `nebula-metrics` export conventions without introducing direct domain coupling.
- **Rationale**: Existing docs separate concerns: telemetry owns event bus/in-memory metrics; metrics docs plan unified export later.
- **Alternatives considered**:
  - Merge telemetry responsibilities into `nebula-log`: rejected due to scope and layering violations.
  - Ignore metrics naming/export plans: rejected due to future compatibility risk.

## Decision 5: Rolling Strategy Completion

- **Decision**: Add size-based rolling with explicit behavior parity to existing rotation modes and clear operator guidance for retention-related edge cases.
- **Rationale**: `log` roadmap and current-state docs identify size rolling as a direct production gap.
- **Alternatives considered**:
  - Keep declaration-only placeholder: rejected because it violates startup and reliability expectations.
  - Replace with time-only rotation: rejected because operators need file-size controls.

## Decision 6: Performance Guardrail Validation

- **Decision**: Use repeatable benchmark scenarios around emission path, context propagation, and hook dispatch; enforce regression thresholds in CI quality checks.
- **Rationale**: Success criteria require bounded user-impact, and docs already call for hot-path benchmarking.
- **Alternatives considered**:
  - Ad hoc manual timing: rejected as non-repeatable.
  - Throughput-only checks without latency: rejected because p95/p99 user impact can regress unnoticed.

## Decision 7: Compatibility and Migration Contract

- **Decision**: Preserve additive compatibility for minor versions, enforce deprecation window before major removals, and verify via schema snapshots and migration docs.
- **Rationale**: `log` migration and API docs explicitly frame compatibility as a release contract.
- **Alternatives considered**:
  - “Best effort” compatibility without tests: rejected due to upgrade risk across workspace consumers.
  - Immediate removals for cleanup: rejected as disruptive.

## Quality Gate Results (2026-02-28)

- `cargo fmt --all` — passed
- `cargo fmt --all -- --check` — passed
- `cargo clippy -p nebula-log -- -D warnings` — passed
- `cargo check -p nebula-log --all-targets` — passed
- `cargo test -p nebula-log` — passed
- `cargo test -p nebula-log --test writer_fanout --features file` — passed
- `cargo test -p nebula-log --test config_compatibility --test hook_policy` — passed
- `cargo bench -p nebula-log --bench log_hot_path` — passed
- `cargo doc --no-deps -p nebula-log` — passed

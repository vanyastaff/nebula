# Decisions

## D001: Single crate for allocator + pool + cache + budget toolkit

Status: Adopt

Context:

Runtime consumers usually need combined memory strategies, not isolated primitives.

Decision:

Keep these capabilities in one crate with clear module boundaries.

Alternatives considered:

- split into multiple crates immediately

Trade-offs:

- pro: easier adoption and fewer cross-crate wiring costs
- con: wider API surface and maintenance burden

Consequences:

Documentation and contracts must stay explicit to avoid accidental coupling.

Migration impact:

None short-term; future split remains possible via adapter layers.

Validation plan:

Track compile/test matrix and integration friction by consumer crates.

## D002: Feature-gated architecture is mandatory

Status: Adopt

Context:

Not every binary needs stats/monitoring/async/profiling overhead.

Decision:

Preserve granular feature flags and avoid always-on heavy modules.

Alternatives considered:

- monolithic always-enabled build

Trade-offs:

- pro: smaller binaries and lower baseline overhead
- con: more CI permutations

Consequences:

Feature combinations become part of compatibility contract.

Migration impact:

Changes to default feature set require strong migration notes.

Validation plan:

Default/full/selective feature CI gates.

## D003: Reuse-first optimization strategy

Status: Adopt

Context:

Allocation churn is a major hotspot in workflow-style systems.

Decision:

Prioritize pools/arenas/caches and budget-aware reuse over raw allocation throughput alone.

Alternatives considered:

- rely on system allocator as primary path

Trade-offs:

- pro: lower tail latency in repeated execution paths
- con: requires careful lifecycle hygiene and sizing

Consequences:

Operational guidance must include sizing and reset discipline.

Migration impact:

Consumers may shift from direct alloc to reusable primitives.

Validation plan:

Benchmark + stress suite across representative workload patterns.

## D004: Error taxonomy is integration contract

Status: Adopt

Context:

Upper layers need deterministic classification for retry/fail-fast behavior.

Decision:

`MemoryError` semantics and `is_retryable()` remain stable contract signals.

Alternatives considered:

- ad-hoc string-based classification

Trade-offs:

- pro: explicit and testable behavior
- con: variant evolution requires careful versioning

Consequences:

Major changes to error meaning require major version.

Migration impact:

Any remapping must be documented in `MIGRATION.md`.

Validation plan:

Contract tests for retryable vs non-retryable mapping.

## D005: Monitoring influences policy, not hidden behavior

Status: Adopt

Context:

Automatic hidden policy mutations are hard to debug.

Decision:

Monitoring exposes signals/actions; runtime chooses how aggressively to react.

Alternatives considered:

- opaque auto-tuning inside memory crate

Trade-offs:

- pro: transparent control flow and safer operations
- con: more orchestration work in runtime layer

Consequences:

Cross-crate interaction docs become critical for consistent behavior.

Migration impact:

None immediate; enables controlled future adaptive features.

Validation plan:

Runtime contract tests for pressure-action handling.

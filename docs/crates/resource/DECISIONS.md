# Decisions

## D001: Manager is the single registry and policy gate

Status: Adopt

Context:

Multiple crates need uniform access to resources with strict isolation and operational controls.

Decision:

All acquire paths go through `Manager`, which enforces quarantine, health, scope checks, and hook/event orchestration.

Alternatives considered:

- decentralized per-crate pools
- direct pool access from action/runtime layers

Trade-offs:

- pro: central invariant enforcement and observability
- con: broader API surface and a shared hot path

Consequences:

Cross-crate contracts are clearer; manager stability becomes critical.

Migration impact:

Any direct-pool integrations should migrate to provider/manager contract.

Validation plan:

Keep integration tests for manager acquire/shutdown and scope safety as release gates.

## D002: Scope containment is deny-by-default for missing parent chain

Status: Adopt

Context:

Tenant isolation requires transitive trust across workflow/execution/action hierarchy.

Decision:

`Scope::contains` denies compatibility when parent identifiers are required but missing.

Alternatives considered:

- permissive fallback when parent fields are absent

Trade-offs:

- pro: safer multi-tenant boundaries
- con: more explicit context construction required

Consequences:

Callers must provide complete lineage for fine-grained scopes.

Migration impact:

Legacy call sites that omitted parent data may fail and need context enrichment.

Validation plan:

Preserve scope property tests and cross-tenant denial tests.

## D003: Keep heavy integrations feature-gated

Status: Adopt

Context:

Not all consumers need metrics/tracing/credential integration.

Decision:

`metrics`, `tracing`, `credentials`, `tokio` remain optional feature gates.

Alternatives considered:

- always-on dependencies

Trade-offs:

- pro: smaller baseline footprint
- con: larger compatibility matrix

Consequences:

CI and docs must cover feature combinations.

Migration impact:

Minor when adding features; major only if defaults change incompatibly.

Validation plan:

Run test matrix on default + full feature sets.

## D004: Resource IDs remain string-based public keys for now

Status: Defer

Context:

Dynamic workflows often resolve resource identity at runtime.

Decision:

Retain string IDs as primary registry key; typed key abstraction remains proposal-stage.

Alternatives considered:

- mandatory typed registration keys

Trade-offs:

- pro: dynamic compatibility with plugin/runtime resolution
- con: runtime mismatch risks

Consequences:

Need strong docs and helper APIs (`acquire_typed`) to reduce mistakes.

Migration impact:

None now; potential major migration if typed keys become primary later.

Validation plan:

Keep typed and dynamic acquire tests and add ID mismatch negative tests.

## D005: Resource crate does not own retry policy

Status: Adopt

Context:

Retries can hide back-pressure and duplicate side effects if applied in the wrong layer.

Decision:

Resource layer returns classified errors; retry/circuit policy is owned by caller and `resilience` integration.

Alternatives considered:

- built-in automatic retries inside pool acquire/create path

Trade-offs:

- pro: explicit policy ownership and easier reasoning
- con: caller must configure retries correctly

Consequences:

Interoperability contract with `resilience` is mandatory for platform behavior consistency.

Migration impact:

Callers relying on implicit retries must adopt explicit policy wrappers.

Validation plan:

Contract tests verifying retryable vs non-retryable mapping and no hidden retries.

# Interactions

## Ecosystem Map (Current + Planned)

## Existing crates

- `core`: shared ids/types consumed by API/runtime layers.
- `action`: action contracts that need predictable input validation.
- `workflow`: workflow definition validation (structure, node configs, constraints).
- `engine`/`runtime`: pre-execution and runtime guard validation.
- `sandbox`: capability and input boundary checks.
- `resource`/`credential`: config object validation before registration/use.
- `parameter`: parameter schema + runtime parameter value checks.
- `api`/`cli`/`ui`: external input validation and error mapping.
- `plugin`/`registry`/`sdk`: third-party extension contract validation.
- `log`/`metrics`/`telemetry`: observability for validation failures.

## Planned crates

- `schema` (possible): higher-level schema DSL over typed validators.
- `policy` (possible): validation policy orchestration (fail-fast vs collect-all profiles).

## Downstream Consumers

- API layer: expects stable error codes and field-path mapping.
- Workflow compiler: expects deterministic validation output.
- Plugin runtime: expects predictable compatibility checks.

Consumer mapping expectations:

- `api` maps validator `code` + `field_path` to HTTP error envelopes.
- `workflow` maps `code` + nested tree to compile-time style diagnostics.
- `plugin/sdk` maps `code` + field path to manifest/config feedback.
- `runtime` consumes deterministic pass/fail semantics for preflight checks.

## Upstream Dependencies

- `regex`, `serde`, `serde_json`, `smallvec`, `moka`, `thiserror`.
- fallback behavior:
  - if cache/combinator features unavailable, validation must still behave correctly without memoization.

## Interaction Matrix

| This crate <-> Other crate | Direction | Contract | Sync/Async | Failure handling | Notes |
|---|---|---|---|---|---|
| validator <-> api | out | stable error codes + field paths | sync | fail request with structured error | no retries |
| validator <-> workflow | out | workflow/node config validation contract | sync | reject invalid definitions | compile-time safety analog in runtime |
| validator <-> action | out | action input/config validation rules | sync | return `Validation` class errors | used before execution |
| validator <-> plugin/sdk | out | plugin manifest/config validation | sync | reject load/publish | critical for ecosystem safety |
| validator <-> runtime/engine | out | preflight + boundary checks | sync | fail-fast before expensive execution | protects reliability budget |

## Runtime Sequence

1. Consumer crate builds typed validator chain.
2. Inputs validated at boundary (API/request/config/load).
3. On failure, `ValidationError(s)` mapped to consumer-specific error envelope.
4. On success, downstream execution proceeds.

## Cross-Crate Ownership

- `validator` owns rule semantics and error code meaning.
- `api` owns HTTP representation of validation failures.
- `engine/runtime` own orchestration and retry policies (not validator).
- `sandbox` owns capability policy enforcement.

## Failure Propagation

- failures bubble up as deterministic validation failures.
- retries are generally forbidden for pure validation failures.
- only caller-level transport retries are allowed (outside validator semantics).

## Versioning and Compatibility

- error code stability is a consumer contract.
- breaking change protocol:
  - declare in `MIGRATION.md`
  - major version bump
  - provide code mapping table old -> new.

Field-path compatibility:

- dot-path and JSON pointer contracts are consumer-visible.
- format changes must follow major-version migration protocol.

## Contract Tests Needed

- cross-crate fixture tests for API error mapping.
- compatibility tests for workflow/plugin configs across versions.
- contract suite in this crate:
  - `tests/contract/compatibility_fixtures_test.rs`
  - `tests/contract/typed_dynamic_equivalence_test.rs`
  - `tests/contract/governance_policy_test.rs`
  - `tests/contract/migration_requirements_test.rs`

# Interactions

## Ecosystem Map (Current + Planned)

## Existing crates

- `core`: foundational identifiers/types used by consumers.
- `log`: structured logs used by config loading/reload lifecycle.
- `validator`: schema/rule validation integration.
- `runtime` / `engine` / `worker`: consume runtime config and reload behavior.
- `resource` / `credential` / `resilience`: receive typed sub-configs for initialization.
- `api` / `cli`: provide operational config entry points and overrides.

## Planned crates

- potential remote config providers:
  - adapter crates for `Remote`, `Database`, `KeyValue` source variants.

## Downstream Consumers

- every runtime-facing crate that needs deterministic and validated config.

## Upstream Dependencies

- parsing stack (`serde_json`, `toml`, `yaml-rust2`)
- watching/async stack (`notify`, `tokio`, `futures`)
- fallback behavior:
  - on optional source failure, continue with available valid sources.

## Interaction Matrix

| This crate <-> Other crate | Direction | Contract | Sync/Async | Failure handling | Notes |
|---|---|---|---|---|---|
| config <-> log | out | structured load/reload diagnostics | sync/async | never block config API on log failures | observability only |
| config <-> validator | out | validation hook over merged JSON value | async | reject invalid config atomically | pre-activation safety gate |
| config <-> runtime/engine | out | typed retrieval + reload lifecycle | async | keep last valid config on reload failure | operational critical path |
| config <-> resource/credential | out | section-based typed config extraction | async read | initialization fails on invalid/missing required fields | startup dependency |
| config <-> api/cli | in/out | override ingestion + diagnostics | async | invalid override rejected with field/path errors | control-plane path |

## Runtime Sequence

1. Build config from layered sources.
2. Load and merge source data in priority order.
3. Validate merged config.
4. Expose typed reads to consumers.
5. On change/reload, attempt full rebuild and swap only on success.

## Cross-Crate Ownership

- `config` owns loading/merging/reload contracts.
- consumer crates own domain-specific config schemas.
- `validator` owns validation rule mechanics.
- `runtime` owns reconfiguration rollout strategy.
- `config` owns validator invocation lifecycle (startup/reload gate + fallback).

## Failure Propagation

- source/load/parse errors bubble as `ConfigError`.
- validation errors block activation.
- consumers should treat missing required config as startup/runtime fatal.
- optional source failure contract:
  - optional source failure (`Env`, `EnvWithPrefix`, `Default`) does not activate partial invalid state.
  - diagnostics must include source identity and failure reason, without sensitive values.

## Path and Typed Retrieval Contract

- path traversal:
  - dot notation for object traversal (`a.b.c`)
  - numeric segments for arrays (`items.0.name`)
- stable categories:
  - `missing_path`: unresolved key/index path
  - `type_mismatch`: typed decode failed on existing path
  - `validation_failed`: validator rejected merged candidate
- consumer guidance:
  - treat `missing_path` as contract drift or rollout sequencing issue
  - treat `type_mismatch` as schema/config mismatch requiring rollback or migration

## Versioning and Compatibility

- precedence and path semantics are integration contracts.
- breaking-change protocol:
  - major version bump
  - migration notes with precedence/path changes
  - consumer fixture verification.

## Contract Tests Needed

- precedence matrix tests across multiple source types.
- reload atomicity tests (last-valid snapshot preserved).
- typed access compatibility tests for consumer crates.
- downstream consumer requirements:
  - each consumer crate must maintain at least one fixture asserting required paths.
  - consumer CI must fail on precedence/path category drift.
  - consumers integrating validator-driven config must pin category mapping fixtures.

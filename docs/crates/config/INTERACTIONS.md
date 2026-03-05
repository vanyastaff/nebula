# Interactions

## Ecosystem Map (Current + Planned)

### Upstream (nebula-config depends on)

- **nebula-log** — structured logging for load/reload lifecycle (debug, info, warn).
- **nebula-validator** — `ConfigValidator` blanket impl for `T: Validate<Value>`; validation errors mapped to `ConfigError::ValidationError`; category compatibility pinned by `crates/config/tests/fixtures/compat/validator_contract_v1.json`.
- **Vendor:** `tokio`, `async-trait`, `futures`, `thiserror`, `serde`, `serde_json`, `chrono`, `url`, `dashmap`, `notify`; optional `toml`, `yaml-rust2`.

### Downstream (depend on nebula-config)

- **nebula-resilience** — consumes config for resilience-related settings (current workspace consumer).
- **Planned:** runtime, engine, worker, api, resource, credential — will consume Config for typed sub-configs and reload behavior.

### Planned / optional

- Additional source adapters may be added as separate crates with explicit, versioned contracts.

## Downstream Consumers

- **nebula-resilience:** Uses config for resilience settings; expects stable precedence and typed `get<T>`.
- **Future consumers:** Same contract: deterministic precedence, validation gate, path-based access; each consumer documents its config paths and required keys.

## Upstream Dependencies

- **Parsing:** serde_json (always), toml (feature `toml`), yaml-rust2 (feature `yaml`); FileLoader supports JSON/TOML/YAML/INI/Properties.
- **Async:** tokio, futures, async-trait; ConfigLoader and ConfigWatcher are async.
- **Watching:** notify (FileWatcher); PollingWatcher uses tokio interval.
- **Fallback:** Optional sources (`is_optional()`) do not fail build/reload; non-optional source failure returns Err.
  - If `with_fail_on_missing(true)` is enabled, optional source failures are fatal at both build and reload time.

## Interaction Matrix

| This crate ↔ Other | Direction | Contract | Sync/Async | Failure handling | Notes |
|---|---|---|---|---|---|
| config ↔ log | out | nebula_log::debug/info/warn for load/reload | sync | never block on log failure | observability only |
| config ↔ validator | out | ConfigValidator impl for Validate&lt;Value&gt;; ValidationError → ConfigError | async | reject invalid config at build/reload | category fixture: validator_contract_v1.json |
| config ↔ resilience | out | Config + get&lt;T&gt; for resilience settings | async | missing/invalid config fails consumer startup | current downstream |
| config ↔ runtime/engine (planned) | out | typed retrieval + reload lifecycle | async | keep last valid config on reload failure | operational critical path |
| config ↔ resource/credential (planned) | out | section-based typed config extraction | async read | initialization fails on invalid/missing required fields | startup dependency |
| config ↔ api/cli (planned) | in/out | override ingestion + diagnostics | async | invalid override rejected with field/path errors | control-plane path |

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

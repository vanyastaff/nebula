# nebula-config

Unified configuration system for Nebula services and runtime components.

## Scope

- **In scope:**
  - **core:** `Config`, `ConfigBuilder`, `ConfigSource`, `ConfigFormat`, `SourceMetadata`, `ConfigError`, `ConfigResult`, `ConfigLoader`, `ConfigValidator`, `ConfigWatcher`, `Configurable`, `Validatable`, `AsyncConfigurable`; merge order, path traversal (dot + array index), reload semantics.
  - **loaders:** `FileLoader`, `EnvLoader`, `CompositeLoader` (JSON/TOML/YAML/INI/Properties; env with optional prefix).
  - **validators:** blanket `ConfigValidator` for `Validate<Value>` (nebula-validator bridge).
  - **watchers:** `FileWatcher`, `PollingWatcher`, `NoOpWatcher`; `ConfigWatchEvent` / `ConfigWatchEventType`; hot-reload and optional auto-reload loop.
  - **builders / utils:** `from_file`, `from_env`, `standard_app_config`, `with_hot_reload`; `check_config_file`, `merge_json_values`, `parse_config_string`.
- **Out of scope:** Business-domain config semantics of each crate; orchestration/retry policies (owned by runtime/resilience).

## Current State

- maturity: solid implementation with practical loader/validator/watcher abstractions.
- key strengths:
  - clear `ConfigBuilder` + `Config` model
  - concurrent source loading and priority-aware merge
  - typed `get<T>` API with `serde`-based conversion
  - optional hot-reload and auto-reload loop
- key risks:
  - docs previously drifted and lacked full operational governance sections
  - dynamic path model can cause runtime errors if contracts are weakly documented
  - remote/database/kv source adapters are still placeholder contracts, not production-ready defaults

## Target State

- production criteria:
  - stable source precedence and merge semantics
  - deterministic validation and reload behavior
  - explicit interaction contracts with runtime/resource/credential/log
  - robust migration and reliability guidance
- compatibility guarantees:
  - additive source/format support in minor versions
  - breaking precedence/path/error behavior only in major versions with migration guide

## Feature Flags

- `default`: `json`, `toml`, `yaml`, `env`
- `json`: enable JSON file format support
- `toml`: enable TOML file format support
- `yaml`: enable YAML file format support
- `env`: enable environment source loader (`ConfigSource::Env*`)

## Config Contract Hardening Summary

- contract test suite now lives in `crates/config/tests/contract/*`.
- versioned compatibility fixtures are tracked in `crates/config/tests/fixtures/compat/*`.
- deterministic contracts are explicitly locked for:
  - precedence: `defaults < file < env < inline`
  - reload safety: validator-gated atomic activation and last-known-good retention
  - typed access: stable path traversal and error category mapping
- governance and migration requirements are enforced by doc-backed contract tests.

## Validator Integration Summary

- `nebula-validator` integration is supported directly through `ConfigValidator` bridge impl for validator traits.
- activation contract:
  - validator pass => candidate can activate.
  - validator fail => candidate rejected, active snapshot unchanged.
- cross-crate category contract is pinned by fixtures:
  - `crates/config/tests/fixtures/compat/validator_contract_v1.json`.

## Document Map

- [CONSTITUTION.md](./CONSTITUTION.md) — platform role, principles, production vision
- [ARCHITECTURE.md](./ARCHITECTURE.md)
- [API.md](./API.md)
- [INTERACTIONS.md](./INTERACTIONS.md)
- [DECISIONS.md](./DECISIONS.md)
- [ROADMAP.md](./ROADMAP.md)
- [PROPOSALS.md](./PROPOSALS.md)
- [SECURITY.md](./SECURITY.md)
- [RELIABILITY.md](./RELIABILITY.md)
- [TEST_STRATEGY.md](./TEST_STRATEGY.md)
- [MIGRATION.md](./MIGRATION.md)

## Archive

Legacy notes:
- [`_archive/`](./_archive/)

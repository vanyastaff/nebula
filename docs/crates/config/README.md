# nebula-config

Unified configuration system for Nebula services and runtime components.

## Scope

- In scope:
  - loading config from multiple sources (file/env/composite)
  - format parsing (JSON/TOML/YAML/INI/properties)
  - configuration merging, path-based access, typed retrieval
  - validation pipeline and watcher/hot-reload hooks
- Out of scope:
  - business-domain config semantics of each crate
  - orchestration/retry policies (owned by runtime/resilience)

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

## Target State

- production criteria:
  - stable source precedence and merge semantics
  - deterministic validation and reload behavior
  - explicit interaction contracts with runtime/resource/credential/log
  - robust migration and reliability guidance
- compatibility guarantees:
  - additive source/format support in minor versions
  - breaking precedence/path/error behavior only in major versions with migration guide

## Config Contract Hardening Summary

- contract test suite now lives in `crates/config/tests/contract/*`.
- versioned compatibility fixtures are tracked in `crates/config/tests/fixtures/compat/*`.
- deterministic contracts are explicitly locked for:
  - precedence: `defaults < file < env < inline`
  - reload safety: validator-gated atomic activation and last-known-good retention
  - typed access: stable path traversal and error category mapping
- governance and migration requirements are enforced by doc-backed contract tests.

## Document Map

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

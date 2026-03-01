# nebula-validator

Composable validation framework for Nebula crates and runtime boundaries.

## Scope

- **In scope:**
  - **foundation:** `Validate<T>`, `ValidateExt`, `Validatable`, `ValidationError`, `ValidationErrors`, `ErrorSeverity`, `ValidationContext`, `ContextualValidator`, `AnyValidator`, `AsValidatable` (GAT bridge for JSON/dynamic input), sealed category traits (`StringValidator`, `NumericValidator`, etc.).
  - **validators:** length (min_length, max_length, not_empty, …), pattern (alphanumeric, contains, …), content (email, url, matches_regex), range (min, max, in_range), size (min_size, max_size), boolean (is_true, is_false), nullable (required, not_null), network (ipv4, ipv6, hostname), temporal (date, time, date_time, uuid).
  - **combinators:** And, Or, Not, When, Unless, Optional, Field, JsonField, Each, Cached, Lazy, all_of, any_of, with_message, with_code; nested and MultiField.
  - **macros:** `validator!`, `compose!`, `any_of!` (in crate; not re-exported from lib root).
  - Structured errors with code, field path, params, nested; sensitive param redaction; compatibility fixtures (error_registry_v1.json, minor_contract_v1.json).
- **Out of scope:** API transport formatting, retry policy, workflow orchestration.

## Current State

- maturity: good core design, rich validator set, strong tests/benches.
- strengths:
  - type-safe `Validate<T>` model with composable `ValidateExt`
  - structured `ValidationError` with nested errors and metadata
  - dynamic bridge via `AnyValidator` and `validate_any`
  - macro ergonomics (`validator!`, `compose!`, `any_of!`)
- risks:
  - docs drift vs actual module names in older materials
  - large generic combinator types can hurt compile times and debuggability
  - cross-crate error code governance not fully formalized yet

## Target State

- production criteria:
  - stable API contract for action/api/workflow/plugin consumers
  - explicit compatibility policy for error codes and field paths
  - tested performance budgets for hot validation paths
  - deterministic failure semantics across crates
- compatibility guarantees:
  - minor versions: additive validators/combinators only
  - major versions: explicit migration guide for behavior-significant changes

## Near-Term Plan

- governance automation for compatibility registry and migration policy checks.
- contract hardening for cross-crate category and field-path fixtures.
- benchmark policy split into fast PR profile and full release profile.

## Contract Hardening Baseline

- canonical public contract: `Validate<T>`, `ValidateExt<T>`, `ValidationError`, `ValidationErrors`
- compatibility fixtures: `crates/validator/tests/fixtures/compat/minor_contract_v1.json`
- canonical registry fixture: `crates/validator/tests/fixtures/compat/error_registry_v1.json`
- contract test suite: `crates/validator/tests/contract/*`
- migration authority: `MIGRATION.md` with explicit old->new mapping for behavior-significant changes
- minor release rule: additive-only changes to validators/combinators and helpers

## Config Integration Baseline

- config crate consumes validator semantics through direct trait bridge contract.
- category naming compatibility with config is pinned by fixtures:
  - `crates/config/tests/fixtures/compat/validator_contract_v1.json`
- validator-side category constants are defined for cross-crate consistency.

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

Legacy material:
- [`_archive/`](./_archive/)

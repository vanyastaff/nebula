# nebula-validator

Composable validation framework for Nebula crates and runtime boundaries.

## Scope

- In scope: typed validators, combinators, structured validation errors, contextual validation helpers.
- Out of scope: API transport formatting, retry policy, workflow orchestration.

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

## Contract Hardening Baseline

- canonical public contract: `Validate<T>`, `ValidateExt<T>`, `ValidationError`, `ValidationErrors`
- compatibility fixtures: `crates/validator/tests/fixtures/compat/minor_contract_v1.json`
- contract test suite: `crates/validator/tests/contract/*`
- migration authority: `MIGRATION.md` with explicit old->new mapping for behavior-significant changes
- minor release rule: additive-only changes to validators/combinators and helpers

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

Legacy material:
- [`_archive/`](./_archive/)

# Implementation Plan: Validator Contract Hardening

**Branch**: `001-validator-crate-spec` | **Date**: 2026-02-28 | **Spec**: [spec.md](./spec.md)  
**Input**: Feature specification from `/specs/001-validator-crate-spec/spec.md`

## Summary

Harden `nebula-validator` as a stable cross-crate contract by formalizing public API boundaries, error-envelope compatibility, deterministic combinator semantics, and governance for additive change. Deliver documentation-first artifacts and contract fixtures design so downstream crates (`api`, `workflow`, `plugin`, `runtime`) can rely on stable error codes and field paths across minor releases.

## Technical Context

**Language/Version**: Rust 2024, MSRV 1.93  
**Primary Dependencies**: `thiserror`, `regex`, `serde`, `serde_json`, `smallvec`, `moka`  
**Storage**: N/A (in-process library, no owned persistence)  
**Testing**: `cargo test`, integration tests in `crates/validator/tests`, property tests via `proptest`, benches via `criterion`  
**Target Platform**: Rust workspace crates on Linux/macOS/Windows build targets  
**Project Type**: Rust library crate (`crates/validator`)  
**Performance Goals**: Maintain synchronous boundary validation within existing hot-path budgets; no regression in benchmarked combinator and string-validator paths  
**Constraints**: Deterministic side-effect-free semantics; no sensitive data leakage in errors; minor releases must be additive only  
**Scale/Scope**: One crate (`nebula-validator`) with contract impact on multiple downstream crates

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- `I. Type Safety First`: PASS. Primary contract remains `Validate<T>` and typed combinators; dynamic bridge stays secondary.
- `II. Isolated Error Handling`: PASS. Error model stays local to crate (`ValidationError`/`ValidationErrors`) with boundary mapping by consumers.
- `III. Test-Driven Development`: PASS (planning stage). Implementation tasks will require contract tests first for compatibility fixtures.
- `IV. Async Discipline`: PASS (N/A). Validator API is synchronous and side-effect free.
- `V. Modular Workspace Architecture`: PASS. No new cross-layer dependency changes proposed.
- `VI. Observability by Design`: PASS with boundary note. Validator exposes structured failures; emitting telemetry remains consumer responsibility.
- `VII. Simplicity and YAGNI`: PASS. Scope is contract hardening, docs, fixtures, and governance; no speculative runtime abstractions.
- `VIII. Rust API Guidelines and Documentation`: PASS. Plan includes public contract docs, migration policy, and quality gate alignment.

Post-design re-check: PASS. Phase 1 artifacts introduce no constitution violations.

## Project Structure

### Documentation (this feature)

```text
specs/001-validator-crate-spec/
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
│   ├── validator-public-api.md
│   └── validation-error-envelope.schema.json
└── tasks.md
```

### Source Code (repository root)

```text
crates/validator/
├── src/
│   ├── foundation/
│   ├── validators/
│   ├── combinators/
│   ├── lib.rs
│   ├── macros.rs
│   └── prelude.rs
├── tests/
├── benches/
└── examples/

docs/crates/validator/
├── README.md
├── API.md
├── ARCHITECTURE.md
├── INTERACTIONS.md
├── DECISIONS.md
├── RELIABILITY.md
├── SECURITY.md
├── TEST_STRATEGY.md
├── ROADMAP.md
├── PROPOSALS.md
└── MIGRATION.md
```

**Structure Decision**: Single Rust workspace crate with documentation-driven contract hardening. Keep existing module split (`foundation`/`validators`/`combinators`) and add spec artifacts plus contract schema files under this feature directory.

## Complexity Tracking

No constitution violations or exception justifications required.

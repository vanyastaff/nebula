# Architecture

## Problem Statement

- Business problem: multiple Nebula crates need consistent, reusable, machine-readable validation.
- Technical problem: avoid ad-hoc validation logic and fragmented error semantics across API/runtime/plugin boundaries.

## Current Architecture

- module map:
  - `foundation/`: `Validate`, `ValidateExt`, context, error, type erasure
  - `validators/`: domain validators (`length`, `pattern`, `content`, `range`, `size`, `network`, `temporal`, `nullable`, `boolean`)
  - `combinators/`: composition and control (`and`, `or`, `not`, `when`, `unless`, `optional`, `each`, `field`, `json_field`, `cached`, `lazy`)
  - `macros.rs`: `validator!` + compose helpers
  - `prelude.rs`: ergonomic imports
- data/control flow:
  - input -> typed validator chain -> `Result<(), ValidationError>`
  - optionally aggregate to `ValidationErrors` for collect-all scenarios
- known bottlenecks:
  - deeply nested generic types (compile time and diagnostics)
  - regex-heavy and nested JSON validations can dominate runtime CPU

## Target Architecture

- target module map:
  - preserve current split; do not over-fragment into many micro-modules
  - add stricter docs and compatibility contracts rather than structural churn
- public contract boundaries:
  - `Validate<T>`/`ValidateExt` as primary stable API
  - `ValidationError` code/field-path schema as cross-crate contract
- internal invariants:
  - validators are side-effect free
  - combinators preserve deterministic evaluation semantics
  - error codes remain stable unless major migration is declared

## Design Reasoning

- trade-off 1: static typing vs dynamic flexibility
  - chosen: static first (`Validate<T>`), dynamic bridge via `AnyValidator` and `validate_any`
- trade-off 2: rich errors vs allocation cost
  - chosen: rich `ValidationError` with boxed extras and smallvec optimization
- rejected alternatives:
  - stringly-typed validator registry as primary model (reject: unsafe and hard to refactor)

## Comparative Analysis

Sources considered: n8n, Node-RED, Activepieces/Activeflow, Temporal/Airflow style systems.

- Adopt:
  - Node-based platforms’ need for human-readable validation errors and field-path mapping.
  - Workflow platforms’ need for deterministic, replay-safe validation behavior.
- Reject:
  - JS-style runtime schema-only validation as sole source of truth (too weak for Rust compile-time guarantees).
  - heavily implicit coercion behavior (causes hidden bugs in automation flows).
- Defer:
  - declarative schema DSL on top of current typed API, if demand from plugin SDK grows.

## Breaking Changes (if any)

- currently none required.
- potential future break candidates:
  - formalized `FieldPath` type in place of plain string paths
  - stricter error code registry enforcement

## Open Questions

- should `collect-all` vs `fail-fast` be first-class in combinator API or policy wrapper?
- should validator catalogs be exported as machine-readable metadata for UI generation?

# Architecture

## Positioning

`nebula-validator` is a shared domain-infra crate. It should be consumed by many crates, but remain independent from business/runtime orchestration.

Dependency direction:
- `api`, `workflow`, `plugin`, `engine`, etc. -> `nebula-validator`
- `nebula-validator` should not depend on workflow runtime internals

## Internal Structure

- `foundation/`
  - core traits and types (`Validate`, `ValidateExt`, `Validatable`, `ValidationError`, context APIs)
- `validators/`
  - ready-to-use validator primitives by domain
- `combinators/`
  - composition layer (`and`, `or`, `not`, `when`, `optional`, `cached`, `field`, `json_field`, ...)
- `macros.rs`
  - codegen macro for creating validators with low boilerplate
- `prelude.rs`
  - ergonomic import surface for consumers

## Core Design Properties

- compile-time type safety through trait bounds (`AsRef<str>`, `Ord`, etc.)
- composability through combinator types and extension traits
- structured error reporting with nested failures
- context-based validation support for cross-field rules
- performance-conscious error model (`ValidationError` size optimization)

## Runtime Characteristics

- supports both simple single-rule validations and complex composed validators
- optional caching combinator for expensive/repeated checks
- broad test/bench coverage in crate (`tests/`, `benches/`, `examples/`)

## Known Constraints

- complex combinator nesting can produce very large generic types
- heterogeneous validator collections require type erasure patterns (e.g., `AnyValidator`)

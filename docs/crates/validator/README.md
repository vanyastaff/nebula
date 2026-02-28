# nebula-validator

`nebula-validator` is the composable validation framework for Nebula.

It is designed for:
- type-safe validation through trait bounds
- reusable primitive validators (`length`, `range`, `pattern`, `network`, `temporal`, etc.)
- combinator-based composition (`and`, `or`, `not`, `when`, `optional`, `cached`, `field`, `json_field`)
- structured errors with nesting and metadata

## Role in Platform

For a Rust-first workflow automation platform (n8n-like), this crate provides a shared validation language for API inputs, workflow definitions, plugin configs, and runtime data contracts.

## Main Surface

- Foundation:
  - `Validate<T>`
  - `ValidateExt<T>`
  - `Validatable` (`.validate_with(...)`)
  - `ValidationError`, `ValidationErrors`
  - `ValidationContext`, `ContextualValidator`
- Built-in validators:
  - string/content/pattern/length
  - numeric range validators
  - collection size validators
  - boolean/nullable validators
  - network and temporal validators
- Combinators:
  - logical/conditional/optional/cached/field/json-field/nested
- Macros:
  - `validator!`
  - `compose!`
  - `any_of!`

## Document Set

- [ARCHITECTURE.md](ARCHITECTURE.md)
- [API.md](API.md)
- [DECISIONS.md](DECISIONS.md)
- [ROADMAP.md](ROADMAP.md)
- [PROPOSALS.md](PROPOSALS.md)

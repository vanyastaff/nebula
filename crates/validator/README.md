# nebula-validator

Composable, type-safe validation for the Nebula workflow engine.

Provides a generic `Validate<T>` trait with zero-cost combinator composition, a structured
`ValidationError` type with RFC 6901 field paths and nested error trees, `Validated<T>` proof
tokens, and a JSON-serializable `Rule` enum for runtime-configured validation.

## Quick Start

```rust
use nebula_validator::prelude::*;

// Compose validators with .and() / .or() / .not()
let username = min_length(3).and(max_length(20)).and(alphanumeric());
assert!(username.validate("alice").is_ok());
assert!(username.validate("ab").is_err()); // min_length

// Proof token: validate once, carry the guarantee in the type system
let name: Validated<String> = min_length(3).validate_into("alice".to_string())?;
// fn process(name: Validated<String>) — the compiler enforces the check happened
```

## Notes

- `use nebula_validator::prelude::*` imports all built-in validators and core combinators.
- `Validated<T>` intentionally does not implement `Deserialize` — deserialized data must
  be re-validated before a proof token can be issued.
- Error codes are stable across minor releases; the registry lives in
  `tests/fixtures/compat/error_registry_v1.json`.

## Documentation

| Document | Contents |
|----------|----------|
| [`docs/README.md`](docs/README.md) | Core concepts, feature matrix, crate layout |
| [`docs/architecture.md`](docs/architecture.md) | Design decisions, module map, data flow, invariants |
| [`docs/api-reference.md`](docs/api-reference.md) | Every public type, trait, and method |
| [`docs/combinators.md`](docs/combinators.md) | Full combinator catalog and composition patterns |
| [`docs/extending.md`](docs/extending.md) | Writing custom validators, the `validator!` macro |
| [`docs/migration.md`](docs/migration.md) | Versioning policy, breaking changes, migration paths |

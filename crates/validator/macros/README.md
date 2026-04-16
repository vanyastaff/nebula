# nebula-validator-macros

Proc-macro crate for [`nebula-validator`]. Private implementation detail — do
not depend on this crate directly.

Use `nebula-validator` with the `derive` feature (enabled by default) and
import [`Validator`] through the top-level re-export:

```rust
use nebula_validator::Validator;

#[derive(Validator)]
#[validator(message = "user validation failed")]
struct User {
    #[validate(min_length = 3, max_length = 32, alphanumeric)]
    username: String,
    #[validate(email)]
    email: String,
    #[validate(required, range(min = 18, max = 120))]
    age: Option<u8>,
    #[validate(min_size = 1, each(regex = "^[a-z]+$"))]
    tags: Vec<String>,
}
```

See the crate-level rustdoc on [`nebula-validator-macros`] for the full
attribute catalogue and architecture notes.

## Guarantees

- **Regex patterns are validated at macro time** — bad patterns surface as
  compile errors with spans pointing at the field, not runtime panics on
  first call.
- **Regex is compiled once per process** via `LazyLock<Regex>` inside the
  generated `validate_fields` method; no recompilation per `.validate()`.
- **Attribute combinations are checked at parse time** — `exact_length`
  with `min_length`, `is_true` with `is_false`, `required` on a non-`Option`
  field, etc. all fail with clear messages. See
  `nebula-validator/tests/ui/*.rs` for the asserted diagnostics.

## Architecture

Three phases, each in its own module:

1. `parse` — `syn::DeriveInput` → `model::ValidatorInput` IR. Validates
   attribute combinations and regex patterns.
2. `model` — pure IR with zero `syn` types; clean bridge between parse and
   emit.
3. `emit` — `ValidatorInput` → `proc_macro2::TokenStream`. Uses shared
   `wrap_option` / `wrap_message` helpers so every rule emitter stays small
   and consistent.

Shared type-introspection helpers (`option_inner_type`, `vec_inner_type`,
`is_string_type`, `is_bool_type`, `is_option_type`) live in `types` and are
re-used by both parse and emit.

[`nebula-validator`]: https://crates.io/crates/nebula-validator
[`Validator`]: https://docs.rs/nebula-validator/latest/nebula_validator/derive.Validator.html
[`nebula-validator-macros`]: https://docs.rs/nebula-validator-macros

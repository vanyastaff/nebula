# nebula-macros
Proc-macro derives for the Nebula ecosystem — Action, Resource, Plugin, Credential, Parameters, Validator, Config.

## Invariants
- `#[derive(Action)]` requires **unit structs** (no fields). Any field on the action struct is a compile error. Configuration is a separate injected type.
- `#[derive(Validator)]` generates a `nebula_validator::foundation::Validate<Self>` impl — the generated code depends on the current nebula-validator API.

## Key Decisions
- `#[derive(Validator)]` + `#[validate(...)]` field attributes map directly to nebula-validator combinators. The macro translates declarative attrs into composed `Validate` impls.
- `#[derive(Config)]` generates `from_env()` and `from_env_with_prefix(prefix)` + validation. Sources configurable: `dotenv`, `env`, `file`.
- `#[derive(Parameters)]` generates `ParameterCollection` from struct fields — used by the engine to discover action inputs.

## Traps
- Macro expansion errors can be cryptic. Run `cargo expand` to see generated code when debugging.
- `#[derive(Action)]` and `credential = Type` syntax expects a type path, not a string. `credential = "key"` is silently ignored.
- `#[derive(Config)]` field-level `#[validate(...)]` uses the same syntax as `#[derive(Validator)]` — they share the parser in `support` module.

## Relations
- proc-macro crate (no runtime nebula deps). Generates code that uses nebula-validator, nebula-action, nebula-resource, nebula-credential, nebula-parameter.

<!-- reviewed: 2026-03-19 -->

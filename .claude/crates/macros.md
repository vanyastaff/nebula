# nebula-macros
Proc-macro derives for the Nebula ecosystem — Action, Resource, Plugin, Credential, Parameters, Validator, Config.

## Invariants
- `#[derive(Action)]` requires **unit structs** (no fields). Any field on the action struct is a compile error. Configuration is a separate injected type.
- `#[derive(Credential)]` requires **unit structs**. Generates v2 `Credential` impl for static credentials via `StaticProtocol`. State = Scheme (identity path), Pending = NoPendingState, all capability flags false.
- `#[derive(Credential)]` requires `identity_state!(SchemeType, "kind", version)` to be called somewhere for the scheme type — the macro does not generate this.
- `#[derive(Validator)]` generates a `nebula_validator::foundation::Validate<Self>` impl — the generated code depends on the current nebula-validator API.

## Key Decisions
- `#[derive(Credential)]` v2 uses `scheme` + `protocol` attributes (not v1's `input`/`state`/`extends`). The old `CredentialAttrs` in `types/credential_attrs.rs` is dead code from v1.
- `#[derive(Validator)]` + `#[validate(...)]` field attributes map directly to nebula-validator combinators. The macro translates declarative attrs into composed `Validate` impls.
- `#[derive(Config)]` generates `from_env()` and `from_env_with_prefix(prefix)` + validation. Sources configurable: `dotenv`, `env`, `file`.
- `#[derive(Parameters)]` generates `ParameterCollection` from struct fields — used by the engine to discover action inputs.

## Traps
- Macro expansion errors can be cryptic. Run `cargo expand` to see generated code when debugging.
- `#[derive(Action)]` and `credential = Type` syntax expects a type path, not a string. `credential = "key"` is silently ignored.
- `#[derive(Config)]` field-level `#[validate(...)]` uses the same syntax as `#[derive(Validator)]` — they share the parser in `support` module.
- `support/attrs.rs` has `get_ident`, `get_ident_str`, `get_bool` marked `#[allow(dead_code)]` — reserved for future OAuth2/LDAP credential derive macros.

## Relations
- proc-macro crate (no runtime nebula deps). Generates code that uses nebula-validator, nebula-action, nebula-resource, nebula-credential, nebula-parameter.

<!-- reviewed: 2026-03-30 -->

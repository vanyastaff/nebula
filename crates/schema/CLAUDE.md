# nebula-schema — Claude Code orientation
> Agent quick-map for `crates/schema/`. Full design: `README.md`. Repo-wide rules: root `CLAUDE.md`.

**Purpose:** Typed configuration schema shared by every integration concept (Actions, Credentials, Resources); enforces a lint → validate → resolve proof-token pipeline. Replaces the deleted `nebula-parameter` crate.
**Layer:** Core — depends only downward (root CLAUDE.md -> Layered Dependency Map). Siblings: `nebula-validator` (rules), `nebula-expression` (resolution context).

## Commands
- `cargo check -p nebula-schema`
- `cargo nextest run -p nebula-schema`  ·  doctests: `cargo test -p nebula-schema --doc`
- `cargo test -p nebula-schema --features schemars` — JSON Schema export path (smoke test + `json_schema_export` example gated on this feature)
- `task bench:crate CRATE=nebula-schema` — criterion benches (build/validate/serde/resolve/lookup/memory)

## Key files
- `src/lib.rs` — crate root: re-exports, quick-start docs, `extern crate self as nebula_schema` (so `field_key!` absolute paths resolve internally)
- `src/schema.rs` — `Schema` / `SchemaBuilder` (draft model + `build()` proof-token entry)
- `src/validated.rs` — proof-tokens: `ValidSchema`, `ValidValues`, `ResolvedValues` (the typestate sequence)
- `src/field.rs` — unified `Field` enum + all field kinds (string/number/secret/select/object/list/mode/computed…)
- `src/lint.rs` — structural lint passes (duplicate keys, cross-field invariants the builder type can't express)
- `src/has_schema.rs` — `HasSchema` / `schema_of` (the sole Action/Credential schema path; ADR-0052 P3)
- `src/json_schema.rs` — `schemars`-feature Draft 2020-12 export with `x-nebula-*` extensions

## Conventions & never-do
- Proof-tokens are compile-time-evident (L1-4.5): never add runtime flags to skip validate/resolve — the type transition IS the gate.
- This crate is NOT a validation-rules engine (that's `nebula-validator`) nor an expression evaluator (resolution delegates to a caller-supplied `ExpressionContext`).
- The single schema→validator crossing is `validate_rules_with_ctx` + `resolve_field_policies`; rule-failure codes surface validator-native verbatim (`min_length`, `min`, `invalid_format`) — no namespace remap (ADR-0052 P2).
- No KDF/hashing here (removed as a weaker dup of nebula-credential's Argon2id); do not re-add.
- Public surface is strict: `Field::*::new` needs a pre-validated `FieldKey`; use `field_key!(...)` or `Field::try_*` — no panic-on-bad-key helpers (`set_raw` removed; use `try_set_raw`).
- `#[deny(clippy::disallowed_macros)]` bans `#[async_trait]`; use the crate's `EvalFuture` (BoxFuture) alias for object-safe async.
- Cross-crate calls go through `nebula-eventbus`, not direct sibling imports.
- Library code uses typed `thiserror`/`NebulaError`; no panicking unwrap/expect/panic in lib code.

## See also
- `README.md` — full design (Purpose / Role / Public API / Contract / Non-goals); `CHANGELOG.md` for `set_raw` migration
- ADR-0052 (P2 validator-native codes, P3 `schema_of` sole schema path); canon invariants L1-3.5, L1-4.5

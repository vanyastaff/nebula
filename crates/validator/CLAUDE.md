# nebula-validator — Claude Code orientation
> Agent quick-map for `crates/validator/`. Full design: `README.md`. Repo-wide rules: root `CLAUDE.md`.

**Purpose:** Shared validation rules engine — composable programmatic validators (`Validate<T>`) plus a JSON-serializable `Rule` enum that schema fields carry for engine-evaluated validation at lint/activation/runtime.
**Layer:** Core — depends only downward (root CLAUDE.md -> Layered Dependency Map): `nebula-error` only, no sibling Core imports.

## Commands
- `cargo check -p nebula-validator`
- `cargo nextest run -p nebula-validator`  ·  doctests: `cargo test -p nebula-validator --doc`
- `cargo test -p nebula-validator --all-features` — features: `derive` (proc-macro), `network`, `temporal` (all default-on)
- `cargo nextest run -p nebula-validator --test integration` · benches: `string_validators`, `combinators`, `rule_engine`, `error_construction`, `derive_*`

## Key files
- `src/lib.rs` — public surface + re-exports (`Rule`, `RuleKind`, `Validated`, `ExecutionMode`, `validate_rules`); `__private::regex` for derive output
- `src/foundation/` — `Validate`/`ValidateExt` traits, `AnyValidator`, `ValidationError` (80-byte, RFC 6901 paths), `FieldPath`
- `src/rule/mod.rs` — typed sum-of-sums `Rule` seam (`Value`/`Predicate`/`Logic`/`Deferred`/`Described`); cross-kind misuse is a compile error
- `src/engine.rs` — `validate_rules` / `validate_rules_with_ctx`, `ExecutionMode` (`StaticOnly`/`Deferred`/`Full`)
- `src/combinators/` — `.and()`/`.or()`/`.not()`/`when`/`unless`/`each`/`field` composition types
- `src/validators/` — built-ins by category (length, range, content, pattern, network, temporal, nullable)
- `src/proof.rs` — `Validated<T>` proof-token (no `Deserialize` by design)

## Conventions & never-do
- `Validated<T>` is a proof-token (canon §4.5): never construct it without calling `validate`; never add `Deserialize` — deserialized data must be re-validated.
- Each `Rule` inner kind exposes only the method valid for it; do NOT reintroduce a flat enum or cross-kind silent-pass — typed narrowing replaced it (ADR-0052/0080).
- This is NOT a schema system (`nebula-schema`), expression evaluator (`nebula-expression`), resilience pipeline, or API error formatter — keep those concerns out.
- `Rule` wire format is externally-tagged tuple-compact; changing serialization breaks stored rules — keep error codes stable (`tests/fixtures/compat/error_registry_v1.json`).
- Cross-crate calls go through `nebula-eventbus`, not direct sibling imports.
- Library code uses typed `thiserror`/`NebulaError`; no panicking unwrap/expect/panic in lib code.

## See also
- `README.md` — full design · `docs/` (architecture, api-reference, combinators, extending, migration)
- `docs/adr/0080-schema-validation-platform.md` (ADR-0052 consolidated) — schema↔validator condition-eval seam

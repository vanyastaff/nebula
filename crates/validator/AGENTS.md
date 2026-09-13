# nebula-validator — Agent orientation
> Local guide for `crates/validator/`. Read [root AGENTS.md](../../AGENTS.md) first;
> this guide adds crate-specific rules. Design and status: [README.md](README.md).

**Purpose:** Shared validation rules engine — composable programmatic validators (`Validate<T>`) plus a JSON-serializable `Rule` enum that schema fields carry for engine-evaluated validation at lint/activation/runtime.
**Layer:** Core — depends only downward (root AGENTS.md -> Layered Dependency Map): `nebula-error` only, no sibling Core imports.

## Commands

- `cargo test -p nebula-validator --all-features` — features: `derive` (proc-macro), `network`, `temporal` (all default-on)
- `cargo nextest run -p nebula-validator --test integration` · benches: `string_validators`, `combinators`, `rule_engine`, `error_construction`, `derive_*`

## Key files

- `src/lib.rs` — public surface + re-exports (`Rule`, `RuleKind`, `Validated`, `ExecutionMode`, `validate_rules`); `__private::regex` for derive output
- `src/foundation/` — `Validate`/`ValidateExt` traits, `AnyValidator`, `ValidationError` (80-byte, RFC 6901 paths), `FieldPath`
- `src/rule/mod.rs` — typed sum-of-sums `Rule` seam (`Value`/`Predicate`/`Logic`/`Deferred`/`Described`); cross-kind misuse is a compile error
- `src/engine.rs` — `validate_rules` / `validate_rules_with_ctx`, `ExecutionMode` (`StaticOnly`/`Deferred`/`Full`)
- `EvaluationOutcome` distinguishes satisfied rules from partial evaluation;
  `ValidationErrorKind` keeps configuration/unavailable diagnostics out of boolean negation.
- `src/rule/pattern.rs` — `RulePattern`, checked equally by construction and serde
- `src/rule/context.rs` — pending roots affect ancestors and descendants; predicate evaluation is fallible, and pending policies emit `FieldDirective::Deferred` without early `required` errors
- `src/foundation/field_path.rs` — shared data/diagnostic pointer; root `""` differs from empty-key `"/"`, serde is strict, and `from_segments` preserves empty keys
- `src/combinators/` — `.and()`/`.or()`/`.not()`/`when`/`unless`/`each`/`field` composition types
- `src/validators/` — built-ins by category (length, range, content, pattern, network, temporal, nullable)
- `src/proof.rs` — `Validated<T>` proof-token (no `Deserialize` by design)

## Conventions & never-do

- `Validated<T>` is a proof-token (canon §4.5): never construct it without calling `validate`; never add `Deserialize` — deserialized data must be re-validated.
- Each `Rule` inner kind exposes only the method valid for it; do NOT reintroduce a flat enum or cross-kind silent-pass — typed narrowing replaced it (ADR-0052/0080).
- Never interpret a deferred outcome as proof of satisfaction. `Validate<Value>`
  requires `Full`; unavailable evaluation cannot be inverted by `Not` or discarded
  as an ordinary failed alternative. Conditions reject value/deferred rule kinds.
- This is NOT a schema system (`nebula-schema`), expression evaluator (`nebula-expression`), resilience pipeline, or API error formatter — keep those concerns out.
- `Rule` wire format is externally-tagged tuple-compact; changing serialization breaks stored rules — keep error codes stable (`tests/fixtures/compat/error_registry_v1.json`).

## Change checks

| Change | Relevant evidence |
|--------|-------------------|
| Rule evaluation and compatibility | The `integration` target is explicitly mapped to [tests/integration/main.rs](tests/integration/main.rs) in `Cargo.toml`; [json_integration](tests/json_integration.rs) covers JSON rules. |
| Derive syntax and diagnostics | [derive_tests](tests/derive_tests.rs), [ui](tests/ui.rs) with `--features derive`; also SDK [derive_external_contract](../sdk/tests/derive_external_contract.rs) for generated paths. |
| Feature gates | Run affected tests with default features and `--no-default-features`; default enables `derive`, `network`, and `temporal`, so an all-features pass alone misses disabled paths. |

## See also

- `README.md` — full design · `docs/` (architecture, api-reference, combinators, extending, migration)
- ADR-0080 (ADR-0052 consolidated) — schema↔validator condition-eval seam

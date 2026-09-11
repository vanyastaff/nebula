# nebula-schema — Agent orientation
> Local guide for `crates/schema/`. Read [root AGENTS.md](../../AGENTS.md) first;
> this guide adds crate-specific rules. Contract: [README.md](README.md) and
> [design](docs/DESIGN.md).

**Purpose:** Typed configuration schema and canonical phase-indexed data shared by
Actions, Credentials, and Resources; enforces the lint -> validate -> resolve
proof-token pipeline. Replaces the deleted `nebula-parameter` crate.
**Layer:** Core; follow the root dependency map. Siblings own rules
(`nebula-validator`) and expression compilation/evaluation (`nebula-expression`).
`nebula-sdk` is the sole curated, supported Rust product surface; this crate is
an internal technical boundary, not a separately supported downstream API.

## Commands

- `cargo nextest run -p nebula-schema --features schemars --test json_schema_smoke` — JSON Schema export contract; `cargo test -p nebula-schema --features schemars --doc` checks documentation examples separately.
- `cargo nextest run -p nebula-schema --test authored_wire --test json_canonical_v1` - authored serde and independent durable identity contracts.
- `task bench:crate CRATE=nebula-schema` — criterion benches (build/validate/serde/resolve/lookup/memory)

## Key files

- `src/lib.rs` — crate root: re-exports, quick-start docs, `extern crate self as nebula_schema` (so `field_key!` absolute paths resolve internally)
- `src/schema.rs` — `Schema` / `SchemaBuilder` (draft model + `build()` proof-token entry)
- `src/validated/mod.rs` - schema snapshots and checked `ValidSchema`; `validated/preparation.rs`, `validation.rs`, and `values.rs` own consuming preparation, validator integration, and `ValidValues`/`ResolvedValues` custody
- `src/value/mod.rs` - `ValueTree<E>`, phase aliases, `ScalarValue`, and RFC6901 `ValuePath`; helpers `value/tree.rs`, `wire.rs`, `tree_canonical.rs`, and `canonical.rs` separate representation, authored serde, tree identity, and durable raw-JSON v1 bytes
- `src/field.rs` — unified `Field` enum + all field kinds (string/number/secret/select/object/list/mode/computed…)
- `src/lint.rs` — structural lint passes (duplicate keys, cross-field invariants the builder type can't express)
- `src/has_schema.rs` - checked `HasSchema` / `schema_of` returning `Result<ValidSchema, ValidationReport>`; the sole type-driven Action/Credential/Resource schema path (ADR-0052 P3)
- `src/expression.rs` - safe authored sources and `ExpressionContext` over retained `CompiledProgram`s
- `src/context.rs` / `src/loader.rs` - prepared predicate context and bounded, schema-aware redacted loader snapshots
- `src/transformer.rs` - checked regex/capture configuration; infallible string-only application
- `src/json_schema.rs` — `schemars`-feature Draft 2020-12 export with `x-nebula-*` extensions

## Conventions & never-do

- Proof-tokens are compile-time-evident (L1-4.5): never add runtime flags to skip validate/resolve — the type transition IS the gate.
- `RootShape` owns the authoritative root contract. Unit types describe `null`,
  empty braced structs describe objects, and primitives retain known domains.
  Never restore `Any` or empty-object fallbacks for a known scalar. Preserve
  historical record/union/unknown wire bytes; scalar descriptors need explicit
  version support at durable consumer boundaries.
- A tree phase is not a proof. `ValidSchema::validate` consumes `AuthoredValue`
  into schema-bound `ValidValues` containing `CompiledValue` and explicit
  pending obligations. Only consuming resolution produces `ResolvedValues`;
  do not add public constructors or serde paths that forge either value proof.
- Preserve one representation per shape: `Literal` contains only `ScalarValue`,
  objects use arbitrary `String` keys, lists are tree containers, and mode
  envelopes are ordinary objects. `FieldKey`/`FieldPath` identify declarations;
  `ValuePath` identifies RFC6901 data locations, including errors and pending work.
- `from_data` is literal-only; `from_template_json` is explicit authoring
  shorthand. Never infer code from external data or evaluator results. Expression
  permission applies only at the exact declared field, not opaque descendants.
  A parent's expression prohibition applies to its entire subtree.
- Prepare values once: consume all read aliases, apply transforms, and promote
  declared string secrets before returning `ValidValues`. Canonical input wins,
  otherwise the first declared alias wins. Prepare newly evaluated subtrees
  without transforming already prepared siblings again.
- `resolve_data` is the no-engine transition, not a shortcut around validation.
  It rejects compiled expressions and performs full rules and conditional policies.
- This crate is NOT a validation-rules engine (that's `nebula-validator`) nor an expression evaluator (resolution delegates to a caller-supplied `ExpressionContext`).
- The single schema→validator crossing is `validate_rules_with_ctx` + `resolve_field_policies`; rule-failure codes surface validator-native verbatim (`min_length`, `min`, `invalid_format`) — no namespace remap (ADR-0052 P2).
- Field/root checks share `prepared_predicate_context`; secret subtrees and
  expression sources are unavailable, pending paths are supplied separately,
  and whole-container predicates remain supported. Predicate arrays stay opaque
  leaves. Raw wrappers must guard depth before copying and scrub aliases, wrong
  secret-bearing shapes, and unknown mode payloads. Schema loader dispatch must
  bind redaction to declarations; a raw `LoaderContext` is not a safe snapshot.
- Secret and expression diagnostics, including public error source chains,
  must not expose payloads. Use private typed causes for parser/evaluator/decoder
  errors, not interpolated upstream messages. `into_typed` refuses secrets;
  `into_typed_exposing_secrets`, `expose`, and `SecretWire` are explicit trusted
  disclosure boundaries. Consumers must protect any plaintext they obtain.
- Authored serde is wire v2 `{version, data, expressions}` with strict RFC6901
  expression slots `{path, syntax, source}` and bounded depth. Syntax is required
  (`auto`, `expression`, or `template`) and preserved from compiled programs.
  Only the authored phase deserializes;
  secret-bearing trees cannot serialize through that format. JSON views are
  not authored persistence and may retain expression sources.
- Tree canonical version 2, historical schema-definition wire version 1, and raw-JSON
  `canonical_json_v1` are separate contracts. Preserve the exact durable JSON-v1
  byte encoding; it neither interprets expressions nor redacts secrets.
- Expression equality and tree-v2 canonical/commitment encodings include immutable
  `ProgramSyntax` plus exact source bytes. `ExpressionMode` is permission, not grammar.
  `Expression::template` is always string; `new` and authoring shorthand remain AUTO.
- `HasSchema`/derives return checked results and cache failures as reports.
  Regex transformers compile and validate capture indices at construction/serde;
  never restore invalid-pattern no-op fallbacks or logs containing patterns.
- No KDF/hashing here — cryptographic primitives belong to `nebula-crypto`.
- Declaration construction is strict: `Field::*::new` needs a pre-validated
  `FieldKey`; use `field_key!(...)` or `Field::try_*`, never panic-on-bad-key
  helpers. Do not impose declaration-key syntax on arbitrary data properties.
- `#[deny(clippy::disallowed_macros)]` bans `#[async_trait]`; use the crate's `EvalFuture` (BoxFuture) alias for object-safe async.

## Change checks

| Change | Relevant evidence |
|--------|-------------------|
| Validation/resolve custody | [seam_proof_token_custody](tests/seam_proof_token_custody.rs), [seam_single_crossing](tests/seam_single_crossing.rs), [pipeline_e2e](tests/pipeline_e2e.rs). |
| Preparation, pending work, or data-only resolution | [foundation_contract](tests/foundation_contract.rs), [proof_contract](tests/proof_contract.rs), [resolve](tests/resolve.rs). |
| Data representation, arbitrary keys, or paths | [foundation_contract](tests/foundation_contract.rs), [authored_wire](tests/authored_wire.rs), [alias](tests/alias.rs). |
| Authored serde or durable canonical bytes | [authored_wire](tests/authored_wire.rs), [json_canonical_v1](tests/json_canonical_v1.rs); do not substitute tree-v2 snapshots for the v1 contract. |
| Secrets, predicates, or loader snapshots | [context_loader_foundation](tests/context_loader_foundation.rs), [seam_root_rule_scrub](tests/seam_root_rule_scrub.rs), [lint_and_loader](tests/lint_and_loader.rs), [expression_diagnostics](tests/expression_diagnostics.rs). |
| Checked transformer configuration | [transformer_contract](tests/transformer_contract.rs), plus the transformer unit tests. |
| Persisted shape or JSON Schema export | [wire_format](tests/wire_format.rs), [evolution_wire_snapshot](tests/evolution_wire_snapshot.rs); [json_schema_smoke](tests/json_schema_smoke.rs) requires `schemars`. |
| Derives and checked schema discovery | [derive_schema](tests/derive_schema.rs), [derive_schema_failures](tests/derive_schema_failures.rs), [compile_fail](tests/compile_fail.rs), and SDK [derive_external_contract](../sdk/tests/derive_external_contract.rs). |

## See also

- `README.md` - current API contract; `docs/DESIGN.md` - representation and proof boundaries; `CHANGELOG.md` - breaking migrations and historical entries
- ADR-0052 (P2 validator-native codes, P3 `schema_of` sole schema path); canon invariants L1-3.5, L1-4.5

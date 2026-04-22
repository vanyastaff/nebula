---

name: nebula-schema

role: Typed Configuration Schema with Proof-Token Pipeline (bespoke; informed by Domain Modeling Made Functional "make illegal states unrepresentable")
status: frontier
last-reviewed: 2026-04-22
canon-invariants: [L1-3.5, L1-4.5]
related: [nebula-validator, nebula-expression, nebula-action, nebula-resource, nebula-credential]
---

# nebula-schema

## Purpose

Typed configuration schema used by every integration concept (Actions, Credentials, Resources). Replaces the deleted `nebula-parameter` crate. Provides schema-time validation and runtime resolution as compile-time-evident steps through a proof-token pipeline.

## Role

**Typed Configuration Schema with Proof-Token Pipeline.** The shared schema system across all integration concepts. A caller cannot skip validation or resolution because the types enforce the sequence: you hold a `Schema`, you call `validate` to get `ValidValues`, you call `resolve` to get `ResolvedValues`. Each step is a type transition; the next step is only callable when the previous has completed.

Pattern inspiration: DMMF proof-tokens (ch "Modeling with Types") and Rust typestate (Rust for Rustaceans, ch Designing Interfaces).

## Public API

- `Field` — unified enum over all field kinds (string, number, bool, enum, nested, …).
- `Schema` — value type for a field list; use `Schema::builder()` or `Schema::add` for construction.
- `Schema::builder() -> SchemaBuilder` — primary entry point.
- `Schema::add` replaces an existing top-level field with the same key; `SchemaBuilder::add` accumulates and lets lint catch duplicates.
- `Schema::lint() -> ValidationReport` — structural diagnostics (errors block `build`; warnings are advisory).
- `SchemaBuilder::build() -> Result<ValidSchema, ValidationReport>` — runs lint passes and returns the `ValidSchema` proof-token.
- `ValidSchema::validate(&FieldValues) -> Result<ValidValues, ValidationReport>` — schema-time validation; returns the first proof-token.
- `ValidValues::resolve(self, ctx: &dyn ExpressionContext) -> Result<ResolvedValues, ValidationReport>` — async runtime resolution; consumes the first proof-token and returns the second (use `.await`).
- `FieldValues`, `ResolvedValues` — value containers.
- `FieldValues::try_set_raw` — fallible raw setter for runtime code paths; `set_raw` is the panic-on-invalid-key helper for tests/migrations.
- `ValidSchema::json_schema() -> Result<schemars::Schema, JsonSchemaExportError>` (`schemars` feature) — exports JSON Schema Draft 2020-12 plus `x-nebula-*` contract extensions for schema semantics that JSON Schema alone cannot encode.

See `src/lib.rs` rustdoc for the quick-start example.

## Contract

- **[L1-3.5]** Schema is the typed-configuration surface for all integration concepts. See `docs/INTEGRATION_MODEL.md`.
- **[L1-4.5]** `ValidValues` and `ResolvedValues` are compile-time-evident proof-tokens: a caller cannot invoke `resolve` without first holding `ValidValues`, cannot access resolved fields without `ResolvedValues`. No runtime flags.
- **Structural lint** — `Schema::lint` enforces constraints that cannot be expressed in the builder type alone (duplicate keys, invariant violations across fields). Seam: `crates/schema/src/lint.rs`. Tests: `crates/schema/tests/`.
- **Expression-required fields** — fields with `ExpressionMode::Required` (for example `ComputedField`) reject literal inputs with `expression.required` during validate-time.
- **Strict key ingestion** — `FieldValues::from_json` rejects invalid object keys with `invalid_key` instead of silently dropping them.
- **JSON Schema contract export** (`schemars` feature) — `ValidSchema::json_schema` emits Draft 2020-12 shape/rules (`minLength`, `pattern`, `minimum`, `exclusiveMinimum`, `enum`, `minItems`, etc.) and augments it with `x-nebula-*` extensions for expression/required/visibility modes and root rules.

## Non-goals

- Not a validation rules engine — see `nebula-validator` for programmatic validators and declarative `Rule`.
- Not an expression evaluator — resolution delegates to a caller-supplied `ExpressionContext` (implemented by `nebula-expression`).
- Not a UI form renderer — schema carries UI hints as data, rendering lives elsewhere.

## Maturity

See `docs/MATURITY.md` row for `nebula-schema`.

- API stability: `frontier` — Phase 1 Foundation just landed (commit `ed3a0ce0`); Phases 2–4 (DX, security, advanced) in progress.
- Core pipeline (lint → validate → resolve) is stable; peripheral APIs (UI hints, expression context adapters) may move.

## Related

- Canon: `docs/PRODUCT_CANON.md §1`, §3.5 (via `docs/INTEGRATION_MODEL.md`).
- ADRs: `docs/adr/0001-schema-consolidation.md`, `docs/adr/0002-proof-token-pipeline.md`, `docs/adr/0003-consolidated-field-enum.md`.
- Siblings: `nebula-validator` (rules), `nebula-expression` (resolution context).
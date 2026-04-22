# nebula-schema Phase 4 Advanced — Implementation Plan (skeleton)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans. **This is a skeleton plan** — refine after Phase 3 lands.

**Goal:** Deliver stability + advanced capabilities: JSON Schema export, full nebula-expression integration, schema diffing, i18n runtime helper, async validate, perf pass.

**Spec:** `docs/superpowers/specs/2026-04-16-nebula-schema-phase4-advanced-design.md`

**Estimated tasks:** 12.

---

## Skeleton tasks

### Task 1 — JSON Schema export: primitives

**Files:** `crates/schema/src/json_schema.rs` (new, feature-gated)

- [ ] TDD: `StringField` → `schemars::Schema` with `type: "string"`, format mapped from `InputHint`.
- [ ] Implement for `String/Number/Boolean`.
- [ ] Commit.

### Task 2 — JSON Schema export: composite types

- [ ] TDD: `ObjectField` → `{ type: "object", properties: {...}, required: [...] }`.
- [ ] TDD: `ListField` → `{ type: "array", items: {...} }`.
- [ ] TDD: `SelectField` → `{ enum: [...] }` (or `{ oneOf }` with labels).
- [ ] TDD: `ModeField` → `{ oneOf: [...] }` discriminated by `mode`.
- [ ] Commit.

### Task 3 — JSON Schema export: rules mapping

- [ ] `Pattern` → `pattern`, `MinLength`/`MaxLength` → `minLength`/`maxLength`, `Min`/`Max` → `minimum`/`maximum`, `Url`/`Email` → `format`.
- [ ] TDD per rule kind.
- [ ] Commit.

### Task 4 — nebula-expression real AST integration

**Files:** `crates/schema/src/expression.rs` (modify)

- [ ] Replace the Phase 1 stub with `nebula_expression::parse(&source) -> Result<Ast, _>`.
- [ ] `Expression::parse()` returns `&nebula_expression::Ast`.
- [ ] Commit.

### Task 5 — Expression type inference at build time

**Files:** `crates/schema/src/expression.rs`, `crates/expression/src/infer.rs` (new)

- [ ] Add `nebula_expression::infer_type(&Ast) -> ExpressionType`.
- [ ] At `SchemaBuilder::build`, for each field with a default expression, infer + compare to field target type; mismatch → `ValidationError::new("expression.type_mismatch")`.
- [ ] TDD: a `NumberField` with `default: {"$expr": "{{ $input.name }}"}` where inference says the expression is string → build-time error.
- [ ] Commit.

### Task 6 — `SchemaDiff::between`

**Files:** `crates/schema/src/diff.rs` (new)

- [ ] Define `SchemaChange` variants per spec.
- [ ] Algorithm: walk both schemas keyed on `FieldPath`; emit `Added`/`Removed`/`TypeChanged`/`RequiredChanged`/`RulesChanged`.
- [ ] TDD: two schemas differing in one top-level field → one `Added` + one `Removed`.
- [ ] TDD: deeply nested field required-changed → exactly one `RequiredChanged` event.
- [ ] Commit.

### Task 7 — `nebula-schema-i18n` crate skeleton

**Files:** `crates/schema-i18n/Cargo.toml`, `src/lib.rs`, `src/translator.rs`

- [ ] Add to workspace members.
- [ ] Define `Translator` trait, `SimpleTranslator { map: HashMap<locale, HashMap<code, template>> }`.
- [ ] Template interpolation: `"{field} too long, max {max}"` + params → formatted string.
- [ ] TDD: simple load + translate test.
- [ ] Commit.

### Task 8 — `localize(report, locale, translator)` helper

**Files:** `crates/schema-i18n/src/lib.rs`

- [ ] Iterate `ValidationReport`, call `translator.translate`, fall back to `error.message`.
- [ ] TDD: english + russian fixtures, report with mixed codes.
- [ ] Commit.

### Task 9 — `ValidSchema::validate_async`

**Files:** `crates/schema/src/validate_async.rs` (new)

- [ ] Signature: `async fn validate_async(&self, &FieldValues, &LoaderRegistry) -> Result<ValidValues<'_>, ValidationReport>`.
- [ ] For each `SelectField` with `dynamic=true`, fetch options, verify value membership.
- [ ] Sequential (not parallel) for 4.0.
- [ ] TDD: dynamic select with mocked loader returning `[option_a, option_b]`; value `option_c` → `option.invalid`.
- [ ] Commit.

### Task 10 — Arena-backed `FieldValues` (opt-in feature)

**Files:** `crates/schema/src/value.rs` (additions)

- [ ] Feature flag `arena`. When enabled, alternative `FieldValuesArena<'bump>` backed by `bumpalo::Bump`.
- [ ] Parse path into arena, single deallocation on arena drop.
- [ ] TDD: build 10_000-key arena values, assert total allocations drop to 1 (bump).
- [ ] Commit.

### Task 11 — Performance benchmarks

**Files:** `crates/schema/benches/bench_pipeline.rs` (new)

- [ ] End-to-end bench: parse → validate → resolve for small/medium/large schemas.
- [ ] Compare arena vs standard `IndexMap` variant.
- [ ] Compare sync validate vs validate_async (with noop loader).
- [ ] Record in README or `docs/bench-results-phase4.md`.
- [ ] Commit.

### Task 12 — Acceptance + CHANGELOG

- [ ] `cargo test --workspace && cargo clippy --workspace -- -D warnings && cargo nextest run --workspace`.
- [ ] Update `CHANGELOG.md` with Phase 4 entries per spec §1.
- [ ] Commit.

---

## Acceptance (Phase 4)

- [ ] `schemars` feature compiles and exports canonical JSON Schema for every `Field` variant
- [ ] Expression default with wrong type is caught at build time
- [ ] `SchemaDiff` accurately reports added/removed/changed
- [ ] `nebula-schema-i18n` lookups fall back to English `message`
- [ ] `validate_async` rejects unknown values for dynamic selects
- [ ] Bench: 100-field schema parse+validate+resolve under 10 µs on a typical dev machine (target — adjust based on actuals)
- [ ] Workspace green

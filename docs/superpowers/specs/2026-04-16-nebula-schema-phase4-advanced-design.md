# nebula-schema — Phase 4 Advanced — Design Spec

**Status:** Draft — to be refined after Phase 3 lands
**Date:** 2026-04-16
**Phase:** 4 of 4
**Depends on:** Phases 1–3

---

## 1. Goal

Stability + advanced capabilities. This is the phase where the core API stops changing and the surrounding ecosystem is filled in:

1. Real JSON Schema export (`schemars` feature → usable)
2. Full `nebula-expression` integration — real AST parse/evaluate + compile-time type inference against schema
3. Schema diffing for plugin migration
4. i18n runtime helper crate (sibling to `nebula-schema`)
5. Async loader integration into validate
6. Performance pass with bench-driven optimization

---

## 2. Non-goals

- Breaking API changes — by Phase 4, API is stable
- Multi-language codegen (Python / TS / …) — separate project
- Browser/WASM compatibility — if it falls out naturally, great; not a goal

---

## 3. Key decisions

| Question | Decision |
|----------|----------|
| JSON Schema output format | `schemars::Schema` (JSON Schema Draft 2020-12). Every `Field` variant maps to a canonical JSON Schema shape. |
| Expression type inference | At `Schema::build()` time, for each field carrying an expression default, `nebula-expression::infer_type(ast)` checks against the field's target type; mismatch → `ValidationError::new("expression.type_mismatch")` at build time. |
| Schema diffing surface | `SchemaDiff::between(&old, &new) -> Vec<SchemaChange>` with typed variants: `Added`, `Removed`, `TypeChanged`, `RequiredChanged`, `RulesChanged`. Returns machine-readable; visualisation is consumer-side. |
| i18n runtime | Separate crate `nebula-schema-i18n` under `crates/schema-i18n/`. Depends on `nebula-schema`. Lookup format: `schema.{code}` → template; interpolates `{max}`, `{actual}` from `ValidationError::params`. |
| Async validate | New method `ValidSchema::validate_async(&values, &loaders) -> Result<ValidValues, Report>`. Calls option loaders for `SelectField` with `dynamic=true` and verifies value membership. Sync `validate` stays default. |
| Performance | Arena-allocated `FieldValues` variant via `bumpalo` for request-scope lifetimes; optional feature `arena`. Main benchmark goal: parse + validate of a 100-field schema under 10 µs on desktop. |

---

## 4. Architecture

```
crates/schema/
├── src/
│   ├── json_schema.rs  (new, behind `schemars` feature)
│   ├── diff.rs         (new) — SchemaDiff, SchemaChange
│   └── validate_async.rs (new) — ValidSchema::validate_async
├── benches/
│   └── bench_pipeline.rs (new) — end-to-end build+parse+validate+resolve

crates/schema-i18n/   (new sibling crate)
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── translator.rs   — Translator trait, lookup + interpolation
    └── fluent.rs       (optional) — fluent-rs backend

crates/expression/  (modified — Phase 4 touches `nebula-expression`)
├── src/
│   ├── ast.rs          (expose)
│   ├── infer.rs        (new) — type inference
│   └── evaluate.rs     (keep async surface)
```

### JSON Schema mapping

| Field | JSON Schema |
|-------|-------------|
| `StringField` | `{ type: "string", format?: "email"/"url"/"date"..., minLength?, maxLength?, pattern? }` |
| `NumberField { integer=false }` | `{ type: "number", minimum?, maximum? }` |
| `NumberField { integer=true }` | `{ type: "integer", ... }` |
| `BooleanField` | `{ type: "boolean" }` |
| `SelectField` | `{ enum: [ ... ] }` or `{ oneOf: [...] }` for labelled options |
| `ObjectField` | `{ type: "object", properties: {...}, required: [...] }` |
| `ListField` | `{ type: "array", items: {...}, minItems?, maxItems?, uniqueItems? }` |
| `ModeField` | `{ oneOf: [ { properties: { mode: const, value: ... } } ] }` |

### Schema diffing

```rust
#[non_exhaustive]
pub enum SchemaChange {
    Added { path: FieldPath, new_type: &'static str },
    Removed { path: FieldPath, old_type: &'static str },
    TypeChanged { path: FieldPath, from: &'static str, to: &'static str },
    RequiredChanged { path: FieldPath, from: RequiredMode, to: RequiredMode },
    RulesChanged { path: FieldPath, added: Vec<Rule>, removed: Vec<Rule> },
}

pub struct SchemaDiff;
impl SchemaDiff {
    pub fn between(old: &ValidSchema, new: &ValidSchema) -> Vec<SchemaChange>;
}
```

### Async loader integration

```rust
impl ValidSchema {
    pub async fn validate_async(
        &self,
        values: &FieldValues,
        loaders: &LoaderRegistry,
    ) -> Result<ValidValues<'_>, ValidationReport>;
}
```

Walks the schema. For each `SelectField` with `dynamic=true`:
- Reads loader_key
- Builds `LoaderContext` with redacted values + loader_key
- Calls `loaders.load_options(loader_key, ctx).await`
- Verifies incoming value is in the returned option set; reports `option.invalid` if not

Performance: loader calls are awaited in order per field (not parallel) — simpler, and typical schema has few dynamic selects. Phase 4.5 can parallelise if profiling shows it matters.

### i18n runtime

```rust
// crate nebula-schema-i18n
pub trait Translator {
    fn translate(&self, locale: &str, code: &str, params: &[(Cow<'static, str>, Value)]) -> Option<String>;
}

pub struct SimpleTranslator { /* JSON files */ }
impl SimpleTranslator {
    pub fn load_dir(path: &Path) -> Result<Self, std::io::Error>;
}

pub fn localize(report: &ValidationReport, locale: &str, tr: &dyn Translator) -> Vec<String> {
    report.iter().map(|e| {
        tr.translate(locale, &e.code, &e.params)
          .unwrap_or_else(|| e.message.clone().into_owned())
    }).collect()
}
```

---

## 5. Open questions

1. **JSON Schema for expressions.** A field with `ExpressionMode::Allowed` can be either a string or `{"$expr": "..."}`. JSON Schema represents this as `oneOf: [{ type: "string" }, { properties: { "$expr": "string" } }]`. Decide whether to include the expression-wrapper branch or leave it as "UI-layer concern". Candidate: include, gated by a feature flag.
2. **Schema diff granularity.** Do we detect `label` / `description` changes? Candidate: no — those are i18n/UI concerns and cause too much noise. Diff covers type/shape/validation only.
3. **Async validation fan-out.** Parallel loader calls vs sequential. Candidate: sequential in 4.0, parallel opt-in via method `validate_async_parallel` in 4.1.
4. **`bumpalo` arena ergonomics.** Introducing `&'arena FieldValues` ripples through callers. Candidate: keep as opt-in feature, don't make it the default.

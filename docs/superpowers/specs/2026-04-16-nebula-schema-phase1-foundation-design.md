# nebula-schema — Phase 1 Foundation — Design Spec

**Status:** Draft — restored after lost due to working-tree reset 2026-04-16
**Date:** 2026-04-16
**Phase:** 1 of 4
**Builds on:** `2026-04-06-parameter-v4-design.md` (ambition extended beyond v4)

---

## 1. Goal

Rebuild `nebula-schema` on a proof-token pipeline that makes it structurally impossible to use unvalidated values or misconstruct a schema. `nebula-parameter` is deleted in this phase; current callers (`action`, `credential`, `sdk`) migrate synchronously.

## 2. Non-goals

- Derive macros — **Phase 2**
- Typed-closure builder DSL — **Phase 2**
- Secret `zeroize` / redaction — **Phase 3**
- JSON-Schema export full — **Phase 4**
- i18n runtime, expression type-inference, schema diffing — **Phase 4**

## 3. Core decisions

| Decision | Choice |
|----------|--------|
| Ambition | Option C — beyond v4 |
| Safety invariants | A + B in one pass: `Validated<FieldValues>` + `Validated<Schema>` |
| Schema-id binding | `'s` borrow; no phantom types |
| Error model | Fully structured unified `ValidationError { code, path, severity, params, message }` across build / lint / validation |
| Expression model | Per-field `ExpressionMode::{Forbidden, Allowed, Required}`; tree-first `FieldValue::Expression`; two-phase validation |
| Proof tokens | Purpose-specific: `ValidSchema`, `ValidValues<'s>`, `ResolvedValues<'s>` |
| Field variants | Consolidate per v4 §5.1: remove `Date`/`DateTime`/`Time`/`Color`/`Hidden`; fold into `String + InputHint` and `VisibilityMode::Never` |
| Extensibility | `Field`, `Rule`, `Transformer`, `ValidationError`, `Severity`, `InputHint` all `#[non_exhaustive]` |
| Async | `ValidValues::resolve` is `async fn` for forward-compat; validation itself sync |

## 4. Five-layer pipeline

```
L1: Schema definitions (static types)
        │ Schema::builder().add(…).build()
L2: Schema construction + lint  →  ValidSchema (ok) | Vec<ValidationError> (errors)
        │ owned handle, Arc-cloned
L3: Value parsing  Value → FieldValues (tree)
        │ ValidSchema.validate(&values)
L4: Schema-time validation  →  ValidValues<'s>
        │ .resolve(&expr_ctx).await
L5: Runtime resolution  →  ResolvedValues<'s>  (what StatelessAction receives)
```

## 5. Crate boundaries

**Dependencies:** `nebula-validator` (Rule, Validated base, RuleContext trait new in Phase 1), `nebula-expression` (Ast, ExpressionContext trait), `serde`, `serde_json`, `indexmap`, `smallvec`, `once_cell`, `thiserror`.

**Features:** `schemars` (stub — full impl Phase 4), `zeroize` (reserved Phase 3).

**Current consumers (migrate in Phase 1):** `nebula-action`, `nebula-credential`, `nebula-sdk`.
**Planned consumers (API-ready):** `nebula-resource`, `nebula-plugin`, `nebula-plugin-sdk`.
**Deleted:** `nebula-parameter` and its `macros` crate, end of Phase 1.

## 6. Key types

```rust
// Proof tokens
pub struct ValidSchema(Arc<ValidSchemaInner>);
pub struct ValidValues<'s> { schema: &'s ValidSchema, values: FieldValues, warnings: Arc<[ValidationError]> }
pub struct ResolvedValues<'s> { /* guaranteed no Expression in tree */ }

// Builder
pub struct Schema;  // marker; Schema::builder() → SchemaBuilder
pub struct SchemaBuilder { fields: Vec<Field> }
impl SchemaBuilder { pub fn build(self) -> Result<ValidSchema, ValidationReport>; }

// FieldKey — no panicking From
pub struct FieldKey(Arc<str>);
impl FieldKey { pub fn new(s: impl AsRef<str>) -> Result<Self, ValidationError>; }
// Compile-time literal: field_key!("name") in nebula-schema-macros

// FieldPath
pub struct FieldPath(SmallVec<[PathSegment; 4]>);
pub enum PathSegment { Key(FieldKey), Index(usize) }

// FieldValue tree
#[non_exhaustive]
pub enum FieldValue {
    Literal(Value),
    Expression(Expression),
    Object(IndexMap<FieldKey, FieldValue>),
    List(Vec<FieldValue>),
    Mode { mode: FieldKey, value: Option<Box<FieldValue>> },
}

pub struct FieldValues(IndexMap<FieldKey, FieldValue>);

// ValidationError
#[non_exhaustive]
pub struct ValidationError {
    pub code: Cow<'static, str>,
    pub path: FieldPath,
    pub severity: Severity,
    pub params: Arc<[(Cow<'static, str>, Value)]>,
    pub message: Cow<'static, str>,
    pub source: Option<Arc<dyn Error + Send + Sync>>,
}

pub enum Severity { Error, Warning }
pub struct ValidationReport { issues: Vec<ValidationError> }

// Field — consolidated, 13 variants
#[non_exhaustive]
pub enum Field {
    String(StringField), Secret(SecretField), Number(NumberField), Boolean(BooleanField),
    Select(SelectField), Object(ObjectField), List(ListField), Mode(ModeField),
    Code(CodeField), File(FileField), Computed(ComputedField),
    Dynamic(DynamicField), Notice(NoticeField),
}
// Removed: Date, DateTime, Time, Color, Hidden
```

## 7. Standard error codes

Vocabulary defined as `pub const STANDARD_CODES: &[&str]` in `error.rs`. Categories:

- Value: `required`, `type_mismatch`, `length.min/max`, `range.min/max`, `pattern`, `url`, `email`, `items.min/max/unique`, `option.invalid`
- Mode: `mode.required`, `mode.invalid`
- Expression: `expression.forbidden`, `expression.parse`, `expression.type_mismatch`, `expression.runtime`
- Loader: `loader.not_registered`, `loader.failed`
- Build: `invalid_key`, `duplicate_key`, `dangling_reference`, `self_dependency`, `visibility_cycle`, `rule.contradictory`, `missing_item_schema`, `invalid_default_variant`, `duplicate_variant`
- Warnings: `rule.incompatible`, `notice.misuse`, `missing_loader`, `loader_without_dynamic`, `missing_variant_label`, `notice_missing_description`

## 8. Expression handling

- `FieldValue::Expression` carries `Expression { source: Arc<str>, parsed: Arc<OnceLock<Ast>> }` — lazy parse, cached.
- Schema-time validation on expression value: parse only; skip value-rules; run predicate-rules.
- Runtime resolution: walk tree, replace each `Expression` with resolved `Literal`; run previously-skipped value-rules post-resolve; fast-path bypass when `SchemaFlags::uses_expressions == false`.
- Per-field `ExpressionMode` default: `Forbidden` for `Boolean`/`Select(single)`/`Notice`; `Allowed` for String/Number/Code/List/Object/Secret/File/Select(multi)/Dynamic; `Required` for `Computed`.

## 9. Data flow & complexity

| Step | Complexity | Allocations |
|------|-----------|-------------|
| Schema build | O(N) | 1× `IndexMap<FieldPath, FieldHandle>` + error Vec |
| Value parse | O(V) | tree proportional to input |
| Schema-time validate | O(N + E·R) | **1× `Vec<ValidationError>`** (was HashMap-per-nesting before) |
| Runtime resolve | O(E) where E = expressions | in-place tree mutation |
| `ValidSchema::find(key)` top-level | O(1) | 0 |
| `ValidSchema::find_by_path(path)` | O(1) hash + O(depth) walk | 0 |

**Key perf win:** `Rule::evaluate` signature changes to `(&dyn RuleContext)` — `FieldValues` and a borrowed `ObjectContext` both implement `RuleContext`, eliminating the HashMap-per-nesting allocation from the current schema.

## 10. Testing

- **Compile-fail** (trybuild) — proof-token invariants (can't validate without build, can't resolve without validate, can't cross-schema values, `field_key!("1bad")` rejects, `FieldKey::from(&str)` removed)
- **Unit** per module, target ≥80% core coverage
- **Property** (proptest) — path parse/display roundtrip, validation never panics, serde roundtrip preserves values
- **Integration** — full flow + every error code from `STANDARD_CODES` covered
- **Benchmarks** (criterion + codspeed) — regression baseline captured pre-refactor; target ≥2× faster `bench_validate` on nested schemas

## 11. Breaking changes + migration

- `Schema::new().add(…)` → `Schema::builder().add(…).build()?`
- `FieldKey::from(&'static str)` removed → use `field_key!("name")` macro or `FieldKey::new(s)?`
- `FieldValues` tree-based (was flat HashMap) — typed accessors retained
- `Field::{Date, DateTime, Time, Color, Hidden}` removed — consolidated
- Error types unified → `ValidationError` + `ValidationReport`
- `Rule::evaluate(&HashMap)` → `Rule::evaluate(&dyn RuleContext)` in `nebula-validator`
- `nebula-parameter` deleted end of Phase 1
- Callers: mechanical rename + per-crate build fixup (tasks 28–31 of plan)

## 12. Acceptance

- [ ] All types in §6 exist with documented surfaces
- [ ] All five layers produce/consume specified proof tokens
- [ ] All codes in STANDARD_CODES emittable from integration tests
- [ ] `cargo nextest run --workspace` green
- [ ] `cargo clippy --workspace -- -D warnings` clean, zero deprecation warnings
- [ ] `cargo test --workspace --doc` green
- [ ] `cargo deny check` green
- [ ] `crates/parameter/` deleted from workspace
- [ ] Callers pass own tests on new API
- [ ] Bench: `bench_validate` improved ≥2× on nested-object schemas vs phase0 baseline
- [ ] All compile-fail fixtures fail to compile and match their `.stderr`

**Plan:** `docs/superpowers/plans/2026-04-16-nebula-schema-phase1-foundation.md`

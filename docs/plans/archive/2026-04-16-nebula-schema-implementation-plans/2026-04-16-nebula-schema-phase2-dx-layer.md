# nebula-schema Phase 2 DX Layer — Implementation Plan (skeleton)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans. **This is a skeleton plan** — refine task-by-task after Phase 1 is done. Don't execute without re-reading the spec and checking what Phase 1 actually landed.

**Goal:** Add `#[derive(Schema)]`, `#[derive(EnumSelect)]`, and typed-closure `Schema::builder()` DSL to the crate.

**Spec:** `docs/superpowers/specs/2026-04-16-nebula-schema-phase2-dx-layer-design.md`

**Estimated tasks:** 12 (vs 32 in Phase 1 — purely additive work).

---

## Skeleton tasks

### Task 1 — `HasSchema` + `HasSelectOptions` traits

**Files:** `crates/schema/src/has_schema.rs` (new), `crates/schema/src/lib.rs`

- [ ] Define:
  ```rust
  pub trait HasSchema {
      fn schema() -> ValidSchema;
  }
  pub trait HasSelectOptions {
      fn select_options() -> Vec<SelectOption>;
  }
  ```
- [ ] Unit test: hand-implement `HasSchema` for a dummy struct, call `T::schema()`, assert fields.
- [ ] Commit.

### Task 2 — Typed-closure builder types

**Files:** `crates/schema/src/builder/{mod,string,number,boolean,select,object,list,code,group}.rs`

Pattern per builder:
```rust
pub struct StringBuilder {
    inner: StringField,
}
impl StringBuilder {
    pub(crate) fn new(key: FieldKey) -> Self { Self { inner: StringField::new(key) } }
    // mirror StringField builder methods
    pub fn build(self) -> StringField { self.inner }
}
```

- [ ] Implement one builder per field type from `field.rs`.
- [ ] TDD: per builder, write test of `.build()` producing the same shape as direct `StringField::new(...)` with matching methods.
- [ ] Commit.

### Task 3 — `Schema::builder()` closure entry points

**Files:** `crates/schema/src/schema.rs`

Extend `SchemaBuilder`:
```rust
impl SchemaBuilder {
    pub fn string(mut self, key: &str, f: impl FnOnce(StringBuilder) -> StringBuilder) -> Self {
        let key = FieldKey::new(key).expect("invalid key literal");
        let built = f(StringBuilder::new(key)).build();
        self.fields.push(built.into());
        self
    }
    // same for number/boolean/select/object/list/code/group...
}
```

- [ ] TDD: write the example from spec §4 as an integration test.
- [ ] Commit.

### Task 4 — `GroupBuilder` with shared `visible_when`

**Files:** `crates/schema/src/builder/group.rs`

- [ ] Define `GroupBuilder` that accumulates child fields and propagates `visible_when`/`required_when` to each child at `.build()` time.
- [ ] Test: group with 3 children, all inherit the same Rule.
- [ ] Commit.

### Task 5 — Compile-fail tests for builder type safety

**Files:** `crates/schema/tests/compile_fail/builder_*.rs` + `.stderr`

- [ ] `builder_min_on_string.rs` — `|s| s.min(3)` where `s: StringBuilder` → error: no such method.
- [ ] `builder_option_on_boolean.rs` — `|b| b.option(json!(1), "X")` on `BooleanBuilder` → error.
- [ ] Record `.stderr` with `TRYBUILD=overwrite`.
- [ ] Commit.

### Task 6 — Attribute-parser module

**Files:** `crates/schema/macros/src/attrs.rs`

- [ ] Define `ParamAttrs`, `ValidateAttrs`, `SchemaAttrs` with `syn`-based parsers.
- [ ] Handle quoted strings, identifiers, pipe-separated values (`"a" | "b" | "c"`), comma-separated conditions.
- [ ] Unit-test each attribute form.
- [ ] Commit.

### Task 7 — Type inference module

**Files:** `crates/schema/macros/src/type_infer.rs`

- [ ] Given a `syn::Type`, recognise `String` / `bool` / primitives / `Option<T>` / `Vec<T>` / "user type" (falls through to `T::schema()` or `T::select_options()`).
- [ ] Handle full paths (`std::string::String`) by last-segment match.
- [ ] Unit test each mapping.
- [ ] Commit.

### Task 8 — `#[derive(Schema)]` macro

**Files:** `crates/schema/macros/src/derive_schema.rs`, wire into `macros/src/lib.rs`

- [ ] Emit `impl HasSchema for $ty { fn schema() -> ValidSchema { Schema::builder()...build().unwrap() } }`.
- [ ] Handle every `#[param(...)]` / `#[validate(...)]` attribute.
- [ ] `#[schema(custom(fn))]` emits a top-level `Rule::Custom`.
- [ ] TDD: derive on `HttpInput` struct from spec §4, check that `HttpInput::schema().fields().len() == 6`.
- [ ] Commit.

### Task 9 — `#[derive(EnumSelect)]` macro

**Files:** `crates/schema/macros/src/derive_enum.rs`

- [ ] Emit `impl HasSelectOptions` with one `SelectOption` per variant.
- [ ] Support `#[param(label = "...")]` per variant.
- [ ] TDD: derive on `HttpMethod`, assert option count and labels.
- [ ] Commit.

### Task 10 — `T: HasSchema` blanket ObjectField inference

**Files:** `crates/schema/macros/src/type_infer.rs`, `crates/schema/src/has_schema.rs`

- [ ] When a struct field type implements `HasSchema`, derive generates `ObjectField` with `T::schema().fields()` inlined.
- [ ] Verify: nested struct `Header { key, value }` inside `HttpInput.headers: Option<Vec<Header>>` produces `ListField` containing `ObjectField` with 2 children.
- [ ] Commit.

### Task 11 — Serde `#[serde(default)]` alignment (v4 §12)

**Files:** `crates/schema/macros/src/derive_schema.rs`

- [ ] When `#[param(default = "...")]` is present, also emit `#[serde(default = "...")]` on the same field so derive-based action Input deserialization matches schema defaults.
- [ ] Integration test: `serde_json::from_value(json!({}))` on `HttpInput` uses defaults from schema.
- [ ] Commit.

### Task 12 — Integration + doctest sweep

**Files:** `crates/schema/tests/flow/derive_roundtrip.rs`, doctests in `lib.rs`

- [ ] Full roundtrip: derive → `schema()` → `validate` → `resolve` → `into_typed::<HttpInput>()`.
- [ ] Doctest in `lib.rs` showing both derive path and builder path.
- [ ] `cargo test --workspace --doc` green.
- [ ] Commit + CHANGELOG entry.

---

## Acceptance (Phase 2)

- [ ] Derive generates `HasSchema` impl that produces `ValidSchema` equivalent to hand-written builder
- [ ] Compile-fail tests pass (all type-safety invariants)
- [ ] Documented examples compile and run
- [ ] Workspace green: `cargo nextest run --workspace && cargo clippy --workspace -- -D warnings`
- [ ] Struct-backed actions can use their struct as both `HasSchema` source and `StatelessAction::Input`

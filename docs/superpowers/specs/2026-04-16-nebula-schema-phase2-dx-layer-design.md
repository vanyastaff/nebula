# nebula-schema — Phase 2 DX Layer — Design Spec

**Status:** Draft — to be refined after Phase 1 lands
**Date:** 2026-04-16
**Phase:** 2 of 4
**Depends on:** Phase 1 Foundation complete

---

## 1. Goal

Make schema authoring pleasant. Two paths both produce the same `ValidSchema`:

1. **Derive path** — Rust struct generates schema via `#[derive(Schema)]` (static schemas, action Input).
2. **Builder path** — typed-closure DSL for dynamic schemas (runtime-constructed from config).

Plus `#[derive(EnumSelect)]` for enum-backed select options.

Neither path is second-class. A struct can simultaneously be `HasSchema` source AND `StatelessAction::Input` type.

---

## 2. Non-goals

- Secrets — **Phase 3**
- JSON Schema export — **Phase 4**
- Expression type-inference at derive-time — **Phase 4**

---

## 3. Key decisions

| Question | Decision |
|----------|----------|
| Builder return type | Each `FooBuilder::build() -> FooField`, then `.into()` to `Field`. Entry point `Schema::builder().string("k", \|s\| s.label(...))` uses closure for type safety — `s: StringBuilder` so `.min()` is a compile error. |
| `HasSchema` trait | `fn schema() -> ValidSchema` (owned — cheap Arc clone internally). |
| Attribute namespaces | `#[param(...)]` for UI/metadata, `#[validate(...)]` for value rules, `#[schema(...)]` on struct for cross-cutting (e.g. `#[schema(custom(fn))]`). |
| `Option<T>` handling | `Option<T>` auto-sets `required=false`; unwrap inner type for field construction. |
| `Vec<T>` handling | `Vec<T>` generates `ListField { item: infer(T) }`. |
| Nested structs | `T: HasSchema` → generates `ObjectField` with `T::schema().fields()` as children. |
| Nested enums | `T: HasSelectOptions` → generates `SelectField` with options. |
| Cross-field validation | `#[schema(custom(fn))]` on struct — generates a `Rule::Custom` tied to the whole value bag. |
| Expression opt-out | `#[param(no_expression)]` on field; derives set `ExpressionMode::Forbidden`. |

---

## 4. Architecture

```
crates/schema/macros/src/
├── lib.rs           (existing) — field_key! macro (Phase 1)
├── derive_schema.rs (new)       — #[derive(Schema)]
├── derive_enum.rs   (new)       — #[derive(EnumSelect)]
├── attrs.rs         (new)       — #[param]/#[validate]/#[schema] parsing
└── type_infer.rs    (new)       — Rust type → Field type mapping

crates/schema/src/
├── has_schema.rs    (new)       — HasSchema, HasSelectOptions traits
├── builder/
│   ├── mod.rs       (new)
│   ├── string.rs    (new)       — StringBuilder
│   ├── number.rs    (new)       — NumberBuilder
│   ├── boolean.rs   (new)
│   ├── select.rs    (new)
│   ├── object.rs    (new)       — nested builder via closure
│   ├── list.rs      (new)
│   ├── code.rs      (new)
│   ├── group.rs     (new)       — grouping builder with shared visible_when
│   └── ...
└── schema.rs        (modify)    — SchemaBuilder gains typed-closure entry methods
```

### Typed-closure builder surface

```rust
let schema: ValidSchema = Schema::builder()
    .string("url", |s| s
        .label("URL")
        .hint(InputHint::Url)
        .required()
        .max_length(8192))
    .number("timeout", |n| n
        .label("Timeout (s)")
        .integer()
        .min(1)
        .max(300)
        .default_i64(30))
    .group("body_section", |g| g
        .visible_when(Rule::one_of("method", ["POST", "PUT", "PATCH"]))
        .string("body", |s| s.widget(StringWidget::Multiline)))
    .boolean("verbose", |b| b.no_expression())
    .build()?;
```

### Derive surface

```rust
#[derive(Schema, Deserialize)]
#[schema(custom(validate_body_matches_method))]
struct HttpInput {
    #[param(label = "URL", hint = "url")]
    #[validate(required, url, length(max = 8192))]
    url: String,

    #[param(default = "GET")]
    method: HttpMethod,

    #[param(label = "Headers")]
    headers: Option<Vec<Header>>,

    #[param(label = "Body", multiline)]
    #[param(visible_when(method = "POST" | "PUT" | "PATCH"))]
    #[validate(length(max = 10_485_760))]
    body: Option<String>,

    #[param(label = "Timeout (s)")]
    #[validate(range(1..=300))]
    timeout: Option<u32>,

    #[param(no_expression)]
    verbose: bool,
}

#[derive(EnumSelect, Deserialize)]
enum HttpMethod {
    #[param(label = "GET")]
    Get,
    #[param(label = "POST")]
    Post,
    // ...
}

// Uses the same struct for both the schema AND the action input:
impl StatelessAction for HttpNode {
    type Input = HttpInput;           // typed!
    async fn execute(&self, input: Self::Input, ctx: &ActionContext) -> ... {
        let url = &input.url;
    }
}
```

---

## 5. Attribute language

### `#[param(...)]` — UI/metadata

| Attribute | Applies to | Maps to |
|-----------|------------|---------|
| `label = "..."` | field | `.label(...)` |
| `description = "..."` | field | `.description(...)` |
| `placeholder = "..."` | field | `.placeholder(...)` |
| `hint = "url"` | String field | `.hint(InputHint::Url)` |
| `default = expr` | field | `.default(json!(expr))` |
| `secret` | String field | switches to `SecretField` |
| `multiline` | String field | `.widget(StringWidget::Multiline)` |
| `no_expression` | field | `.expression_mode(ExpressionMode::Forbidden)` |
| `expression_required` | field | `ExpressionMode::Required` |
| `group = "..."` | field | `.group(...)` |
| `skip` | field | exclude from schema |
| `visible_when(cond)` | field | `.visible_when(rule)` |
| `required_when(cond)` | field | `.required_when(rule)` |
| `disabled_when(cond)` | field | (Phase 2 adds `DisabledMode` to fields) |

Condition syntax (in attribute values):
- `method = "POST"` → `Rule::Eq { field: "method", value: json!("POST") }`
- `method != "GET"` → `Rule::Ne { ... }`
- `method = "POST" | "PUT" | "PATCH"` → `Rule::In { ... }`
- Multiple conditions: `#[param(required_when(a = "x", b = "y"))]` → `Rule::All { ... }`

### `#[validate(...)]` — value rules

| Attribute | Emits |
|-----------|-------|
| `required` | forces `RequiredMode::Always` |
| `length(min = 1, max = 100)` | `MinLength` + `MaxLength` |
| `range(1..=300)` | `Min` + `Max` |
| `pattern = "..."` | `Pattern { pattern }` |
| `url` | `Url` |
| `email` | `Email` |
| `custom(fn, code = "my.code")` | `Rule::Custom` |

### `#[schema(...)]` — struct-level

| Attribute | Effect |
|-----------|--------|
| `custom(fn)` | Adds a struct-level `Rule::Custom` tied to the full value bag |
| `rename_all = "snake_case"` | Field-name transformation for `serde` alignment |

---

## 6. Type inference (syntactic, not semantic)

| Rust type (by path segment) | Inferred Field |
|-----------------------------|----------------|
| `String` | `StringField` |
| `bool` | `BooleanField` |
| `u8..u64`, `i8..i64` | `NumberField { integer: true }` |
| `f32`, `f64` | `NumberField { integer: false }` |
| `Option<T>` | `infer(T)`, `required=false` |
| `Vec<T>` | `ListField { item: infer(T) }` |
| `T: HasSchema` | `ObjectField` (inlines `T::schema()` fields) |
| `T: HasSelectOptions` | `SelectField` with options |

Full paths (`std::string::String`) are recognised by the last segment. Unknown types fall through to a compile-time error listing the trait bounds the user must satisfy (`T must implement HasSchema`).

---

## 7. Migration

Phase 2 is **additive**. Existing callers (already migrated in Phase 1 to direct `Schema::builder().add(Field::...)`) can adopt derive incrementally. No breaking changes.

---

## 8. Open questions (resolve at planning time)

1. `#[param(default = "expr")]` parsing — accept Rust expressions or string-only? Decision candidate: string-only in Phase 2, full expr in Phase 4.
2. Condition syntax nesting — does `#[param(visible_when(not(a = "x")))]` work? Decision candidate: yes, via `not(...)` prefix.
3. `#[schema(custom(fn))]` — sync or async? Decision candidate: sync only in Phase 2; async defers to runtime layer (Phase 4).

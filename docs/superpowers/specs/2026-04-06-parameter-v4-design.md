# nebula-parameter v4 — Design Spec

## Goal

Redesign nebula-parameter for production-quality DX: a working derive macro that infers schemas from Rust types, typed builders, unified validation through nebula-validator, and extensibility for localization. Breaking changes allowed.

## Philosophy

**Two paths, one result.** Inspired by Temporal (Rust types = schema) + n8n (runtime DSL):

1. **Derive path** — Rust struct generates `ParameterCollection` automatically (static schemas)
2. **Builder path** — fluent API with typed closures (dynamic schemas, runtime-constructed)

Both produce the same `ParameterCollection`. Neither is second-class. One struct can be both the schema source AND the typed input for `StatelessAction::Input`.

---

## 1. Derive Macro — Full Rewrite

The current `#[derive(Parameters)]` is non-functional — it generates `Parameter::string()` for every field regardless of type, ignores `default`/`validation`/`options` attributes, and has dead code. Complete rewrite.

### 1.1 Two Derives

**`#[derive(Parameters)]`** — on structs. Generates `HasParameters` trait impl.

**`#[derive(EnumSelect)]`** — on enums. Generates `HasSelectOptions` trait impl. Separate because proc macros cannot resolve types across crate boundaries — the `Parameters` derive calls `HasSelectOptions` via trait bound in generated code.

### 1.2 Type Mapping

| Rust type | ParameterType | Required | Notes |
|-----------|--------------|----------|-------|
| `String` | String | yes | |
| `bool` | Boolean | yes | |
| `u8..u64`, `i8..i64` | Number { integer: true } | yes | |
| `f32`, `f64` | Number { integer: false } | yes | |
| `Option<T>` | infer(T) | no | Unwraps Option, sets required=false |
| `Vec<T>` | List { item: infer(T) } | yes | |
| `T: HasParameters` | Object { nested } | yes | Nested struct = Object parameter |
| `T: HasSelectOptions` | Select { options from T } | yes | Enum = Select parameter |

Detection is syntactic (like clap/schemars): the macro matches on path segments `Option`, `Vec`, `String`, `bool`, numeric primitives. Full paths (`std::string::String`) are also handled.

### 1.3 Attributes

Two attribute namespaces on fields:

**`#[param(...)]`** — parameter metadata (label, hint, default, visibility, expression, secret, display):

```rust
#[param(label = "URL")]                          // display label (fallback for l10n)
#[param(hint = "url")]                           // input hint for UI (url, email, color, date, etc.)
#[param(default = "GET")]                        // default value (becomes serde_json::json!(...))
#[param(secret)]                                 // mask in UI, redact in logs
#[param(multiline)]                              // textarea instead of input (String only)
#[param(expression)]                             // explicitly enable (default is already true)
#[param(no_expression)]                          // explicitly disable expression toggle
#[param(placeholder = "https://...")]            // placeholder text
#[param(description = "...")]                    // long description (fallback for l10n)
#[param(group = "auth")]                         // visual grouping key
#[param(skip)]                                   // exclude from schema entirely
#[param(visible_when(field = "value"))]          // visibility condition
#[param(required_when(field = "value"))]         // conditional required
#[param(disabled_when(field = "value"))]         // conditional disabled
```

**`#[validate(...)]`** — validation rules (required, length, range, pattern, custom):

```rust
#[validate(required)]                            // field must have a value
#[validate(length(min = 1, max = 2048))]         // string/array length bounds
#[validate(range(1..=300))]                      // numeric range
#[validate(pattern = r"^[A-Z]+$")]               // regex pattern
#[validate(url)]                                 // URL format
#[validate(email)]                               // email format
#[validate(custom(my_fn, code = "my.error"))]    // custom validator function
```

On the struct (cross-field validation):

```rust
#[derive(Parameters)]
#[validate(custom(validate_date_range))]
struct ScheduleParams { ... }
```

### 1.4 Condition Syntax in Attributes

```rust
// Equality
#[param(visible_when(method = "POST"))]

// Not equal
#[param(visible_when(method != "GET"))]

// OR — multiple values for one field
#[param(visible_when(method = "POST" | "PUT" | "PATCH"))]

// AND — multiple field conditions (comma-separated)
#[param(required_when(auth_type = "basic", mode = "production"))]
```

Semantics: comma = AND, pipe = OR (matches n8n: multiple keys AND, multiple values OR).

### 1.5 EnumSelect

```rust
#[derive(EnumSelect)]
pub enum HttpMethod {
    #[param(label = "GET")]
    Get,
    #[param(label = "POST")]
    Post,
    #[param(label = "PUT", description = "Update existing resource")]
    Put,
    #[param(label = "DELETE")]
    Delete,
}
```

If `#[param(label)]` is omitted, the variant name is used as-is. Generates:

```rust
pub trait HasSelectOptions {
    fn select_options() -> Vec<SelectOption>;
}
```

### 1.6 Generated Trait

```rust
pub trait HasParameters {
    fn parameters() -> ParameterCollection;
}
```

The derive generates an impl of this trait. The struct can also derive `Deserialize` to be used as `StatelessAction::Input`:

```rust
#[derive(Parameters, Deserialize)]
struct HttpRequestInput {
    #[param(label = "URL")]
    #[validate(required, url)]
    url: String,

    #[param(default = "GET")]
    method: HttpMethod,
}

impl StatelessAction for HttpRequestNode {
    type Input = HttpRequestInput;   // same struct!
    type Output = serde_json::Value;

    async fn execute(&self, input: Self::Input, ctx: &ActionContext) -> ... {
        let url = &input.url;        // typed access!
    }
}
```

### 1.7 Full Example

```rust
#[derive(Parameters, Deserialize)]
#[validate(custom(validate_body_with_method))]
struct HttpRequestInput {
    /// Target URL
    #[param(label = "URL", hint = "url")]
    #[validate(required, url, length(max = 8192))]
    url: String,

    /// HTTP method
    #[param(default = "GET")]
    method: HttpMethod,

    /// Request headers
    #[param(label = "Headers")]
    headers: Option<Vec<Header>>,

    /// Request body — visible only for methods that support it
    #[param(label = "Body", multiline)]
    #[param(visible_when(method = "POST" | "PUT" | "PATCH"))]
    #[validate(max_length = 10_485_760)]
    body: Option<String>,

    /// Timeout in seconds
    #[param(label = "Timeout (s)")]
    #[validate(range(1..=300))]
    timeout: Option<u32>,

    /// Verbose logging (no expression — just a toggle)
    #[param(no_expression)]
    verbose: bool,
}

#[derive(Parameters, Deserialize)]
struct Header {
    #[validate(required)]
    key: String,
    value: String,
}

#[derive(EnumSelect, Deserialize)]
enum HttpMethod {
    #[param(label = "GET")]
    Get,
    #[param(label = "POST")]
    Post,
    #[param(label = "PUT")]
    Put,
    #[param(label = "PATCH")]
    Patch,
    #[param(label = "DELETE")]
    Delete,
}

fn validate_body_with_method(values: &ParameterValues) -> Result<(), ValidationError> {
    let method = values.get_str("method").unwrap_or("GET");
    let has_body = values.get("body").is_some_and(|v| !v.is_null());
    if method == "GET" && has_body {
        return Err(ValidationError::new("http.body_not_allowed")
            .field("body")
            .param("method", method));
    }
    Ok(())
}
```

---

## 2. Builder API — Typed Closures

For dynamic schemas (runtime-constructed, config-driven). Breaking change from current API.

### 2.1 New API

```rust
let params = ParameterCollection::builder()
    .string("url", |s| s
        .label("URL")
        .hint("url")
        .placeholder("https://...")
    )
    .select("method", |s| s
        .option("GET", "GET")
        .option("POST", "POST")
        .option("PUT", "PUT")
        .option("DELETE", "DELETE")
        .default("GET")
    )
    .group("body_settings", |g| g
        .visible_when(Condition::one_of("method", ["POST", "PUT", "PATCH"]))
        .string("body", |s| s.multiline())
        .select("content_type", |s| s
            .option("json", "JSON")
            .option("xml", "XML")
            .option("text", "Plain Text")
            .default("json")
        )
    )
    .number("timeout", |n| n
        .label("Timeout (s)")
        .integer()
        .default(30)
        .min(1)
        .max(300)
    )
    .boolean("verbose", |b| b
        .label("Verbose Logging")
        .no_expression()
    )
    .build()?;
```

### 2.2 Key Differences From Current API

| Current | New | Why |
|---------|-----|-----|
| `Parameter::string("id").label(...)` | `.string("id", \|s\| s.label(...))` | Closure receives typed `StringBuilder` — can't call `.min()` on it |
| `.searchable()` on String = silent no-op | Compile error — `StringBuilder` has no `.searchable()` | Type safety |
| No grouping | `.group("name", \|g\| ...)` | Shared `visible_when` for related fields |
| `Parameter::number("id", false)` (bool for integer) | `.number("id", \|n\| n.integer())` or `.integer("id", \|n\| ...)` | Clearer API |
| Expression opt-in | Expression default true | Matches n8n; `no_expression()` to opt out |

### 2.3 Builder Types

Each parameter type has its own builder:

- `StringBuilder` — label, placeholder, multiline, hint, secret, default(str)
- `NumberBuilder` — label, integer, min, max, step, default(f64/i64)
- `BooleanBuilder` — label, default(bool)
- `SelectBuilder` — label, option(), options(), default, searchable, multiple
- `ObjectBuilder` — label, nested parameters (recursive)
- `ListBuilder` — label, item template parameter, min_items, max_items
- `CodeBuilder` — label, language(str), default
- `GroupBuilder` — visible_when, required_when, contains nested builders

Common methods (on all builders): `no_expression()`, `description()`, `group()`.

Validation via `.rule()` on any builder:
```rust
.string("url", |s| s.label("URL").rule(Rule::required()).rule(Rule::url()))
```

---

## 3. Validation Integration — Unify with nebula-validator

### 3.1 Current State (Two Systems)

- `Rule` enum in parameter — declarative, serializable to JSON for UI
- `Validate<T>` trait in nebula-validator — programmatic, combinators, proof tokens

These are disconnected. `Validate<T>` is not used in the parameter validation pipeline.

### 3.2 New Design: Rule Implements Validate

`Rule` becomes a serializable wrapper that implements `Validate<Value>` from nebula-validator. One system, one validation path:

```
#[validate(required, url)] on field
        ↓ derive macro generates
Rule::Required, Rule::Url in ParameterCollection schema
        ↓ validation time
Rule.validate(&value) calls nebula-validator infrastructure
        ↓ produces
Validated<ParameterValues> proof token
```

### 3.3 Structured Validation Errors

`ValidationError` gains structured fields for localization:

```rust
pub struct ValidationError {
    /// Standard error code (e.g., "length.max", "required", "url")
    pub code: Cow<'static, str>,
    /// Which field failed
    pub field: Option<String>,
    /// Template parameters for interpolation
    pub params: BTreeMap<Cow<'static, str>, Value>,
    /// Default English message (fallback when no l10n available)
    pub message: String,
    // ... existing fields (path, severity, etc.)
}
```

Standard codes are consistent across all plugins:
- `required` — field is required
- `length.min`, `length.max` — string/array length
- `range.min`, `range.max` — numeric range
- `pattern` — regex mismatch
- `url`, `email` — format validation
- Custom codes via `#[validate(custom(fn, code = "my.code"))]`

### 3.4 Three Validation Levels

| Level | Attribute | Serializable? | Example |
|-------|-----------|---------------|---------|
| **Field** | `#[validate(...)]` on field | Yes — `Rule` in JSON schema | `required`, `length(min=1)`, `url` |
| **Conditional** | `#[param(required_when(...))]` on field | Yes — `Condition` in JSON | "body required when POST" |
| **Cross-field** | `#[validate(custom(fn))]` on struct | No — Rust-only | "end_date > start_date" |

UI renders Level 1 + 2 errors in real-time (from schema JSON). Level 3 errors appear after submit (server-side).

---

## 4. Expression Support — Default On

All parameters support expression mode by default (like n8n). The UI shows a toggle button to switch between fixed value and expression editor.

```rust
// Default: expression enabled (no annotation needed)
url: String,

// Explicitly disable for fields where expressions don't make sense
#[param(no_expression)]
verbose: bool,
```

In the schema JSON: `"expression": true` (default) or `"expression": false`.

At runtime, `ParamValue` enum in nebula-workflow already supports this:
```rust
enum ParamValue {
    Literal { value: Value },       // fixed mode
    Expression { expr: String },    // expression mode
    Template { template: String },  // mixed text + {{ expressions }}
    Reference { ... },              // node output reference
}
```

The engine resolves expressions before passing values to actions. Actions never see raw expressions — they receive resolved `Value` or typed `Input` struct.

---

## 5. ParameterType Consolidation

### 5.1 Merge (19 → ~16)

| Remove | Replace With | Rationale |
|--------|-------------|-----------|
| `Date` | `String` + `hint = "date"` | Zero-field variant, just a UI hint |
| `DateTime` | `String` + `hint = "datetime"` | Same |
| `Time` | `String` + `hint = "time"` | Same |
| `Color` | `String` + `hint = "color"` | Same |
| `Hidden` | `Parameter.visible = false` | Not a type, it's a visibility state |

### 5.2 InputHint Enum (New)

Replaces the collapsed types with a UI hint on String parameters:

```rust
#[non_exhaustive]
pub enum InputHint {
    Text,        // default
    Url,
    Email,
    Date,
    DateTime,
    Time,
    Color,
    Password,    // like secret but specific to password fields
    Phone,
    Ip,
    // extensible via #[non_exhaustive]
}
```

In derive: `#[param(hint = "url")]` maps to `InputHint::Url`.

### 5.3 Remaining Types (~16)

String, Number, Boolean, Select, MultiSelect, Object, List, Code, Filter, Mode, Markdown, Notice, File, Dynamic, CurrencyInput, SelectorInput.

`Number` changes: remove `integer: bool` field. Add separate `Integer` variant OR use `InputHint::Integer` on Number. Decision: separate variant is clearer for type inference (`u32` → Integer, `f64` → Number).

---

## 6. Localization Strategy

### 6.1 Convention-Based Keys

No changes to Parameter struct. Localization is a UI-layer concern.

Key format: `{action_key}.{param_id}.{field}`

```json
// plugins/my-plugin/locales/ru.json
{
  "http_request.url.label": "Ссылка",
  "http_request.url.description": "Целевой URL адрес",
  "http_request.url.placeholder": "https://...",
  "http_request.method.label": "Метод"
}
```

Resolution algorithm (in UI layer):
1. Has translation for `{action_key}.{param_id}.label`? → use it
2. No? → use `#[param(label = "URL")]` from schema as fallback

### 6.2 Validation Error Localization

Same convention for validation errors:

```json
{
  "validation.required": "{field} обязательно для заполнения",
  "validation.length.max": "{field} должно быть не более {max} символов",
  "validation.url": "{field} должно быть валидным URL"
}
```

Custom plugin codes:
```json
{
  "validation.webhook.unreachable": "{field} недоступен"
}
```

### 6.3 What This Requires Now

- `param_id` must be stable and human-readable (already is — struct field name)
- `ValidationError` must have `code` + `params` (new, see Section 3.3)
- No new fields on `Parameter` — `label`/`description`/`placeholder` serve as English defaults

---

## 7. Code Quality Fixes

### 7.1 Must Fix (RED)

| File | Issue | Fix |
|------|-------|-----|
| `error.rs` | `category()`/`code()` methods conflict with `#[classify]` derive — different values for same variant | Delete manual methods, use only Classify derive |
| `macros/*` | Entire macro crate generates wrong code | Rewrite (Section 1) |

### 7.2 Should Fix (YELLOW)

| File | Issue | Fix |
|------|-------|-----|
| `parameter.rs` | All fields `pub` — bypasses builder invariants | Make fields `pub(crate)`, expose getters |
| `parameter.rs` | Type-specific methods silently no-op in release | Replace with typed builders (Section 2) |
| `parameter.rs` | `min(f64)` with NaN → silent None | Validate at construction, return Result or clamp |
| `transformer.rs:137` | `Regex::new()` on every `apply()` call | Cache compiled regex at construction (`OnceLock`) |
| `validate.rs:339,418` | Allocates HashMap per nested object and per list item | Pass `&Value` refs through recursion, avoid intermediate ParameterValues |
| `conditions.rs:229` | `evaluate()` takes `&HashMap` not `&ParameterValues` | Change signature to accept `&ParameterValues` |
| `loader.rs` | Three identical loader types (copy-paste x3) | Generic `Loader<T>` type |
| `spec.rs:218` | Debug-based variant name extraction | Use explicit `as_str()` method |
| `runtime.rs` | Stale "v2" docs, duplicate re-exports | Clean up |
| `values.rs:43` | `from_json(&Value)` clones — take `Value` by value | Add `from_json_owned(Value)` or change signature |

### 7.3 Consider (GREEN)

| File | Issue | Fix |
|------|-------|-----|
| `values.rs:322` | `diff()` allocates 3 Vecs unconditionally | Lazy allocation if non-empty |
| `spec.rs:239` | `From<FieldSpec>` creates throwaway Parameter | Direct construction |
| `report.rs` | `errors`/`warnings` are `pub` — undermines `into_validated()` | Make `pub(crate)` with accessors |
| `path.rs:43` | `segments()` returns `Vec<&str>` — could be iterator | Return `impl Iterator` |

---

## 8. Cross-Crate Impact

### 8.1 Dependency Map

| Crate | Dependency | Types Used | Files to Update |
|-------|-----------|------------|-----------------|
| **action** | Direct | `Parameter`, `ParameterCollection` (re-export + ActionMetadata field) | 3 files |
| **credential** | Direct + Heavy | `ParameterCollection`, `ParameterValues`, `Parameter` in trait + 5 built-in impls | ~10 files |
| **sdk** | Facade | Re-exports everything | 2 files |
| engine | None | — | 0 |
| api | None | — | 0 |
| workflow | None | — | 0 |

### 8.2 Migration Strategy

**Phase 1:** Internal refactors (validate.rs, transformer.rs, conditions.rs, loader.rs, error.rs) — zero cross-crate impact.

**Phase 2:** Builder API change (new typed closures) — update credential built-in impls (~5 files) and sdk.

**Phase 3:** Derive macro rewrite — new capability, no breakage (old manual construction still works).

**Phase 4:** ParameterType consolidation (Date/DateTime/Time/Color/Hidden removal) — update any code using these variants.

### 8.3 Stable Public API (Do Not Break)

These types are used in trait signatures and must maintain compatibility:

- `ParameterCollection` — used in `Credential::parameters()`, `ActionMetadata.parameters`
- `ParameterValues` — used in `Credential::resolve()`, `StaticProtocol::build()`
- `HasParameters` trait — new, additive

---

## 9. Credential Auto-Injection

When an Action declares credential dependencies via `ActionDependencies`, credential parameters are automatically added to the schema. Developers do not manually add credential fields.

```rust
impl ActionDependencies for HttpRequestNode {
    fn credentials(&self) -> &[CredentialKey] {
        &[credential_key!("http_auth")]
    }
}
// → Engine auto-injects credential picker into the node's parameter UI
```

This is a runtime concern (engine/UI layer), not a parameter schema concern. The parameter crate does not need changes for this — the engine reads `ActionDependencies::credentials()` and augments the UI.

---

## 10. Not In Scope (Deferred)

- n8n-style `routing` system (declarative HTTP mapping) — belongs in action crate
- `resourceLocator` multi-mode (list/id/url enrichment) — Phase 2, Mode type exists
- Filter builder improvements — existing is adequate
- Expression evaluation in parameters — belongs in engine
- DisplayMode changes — UI concern for desktop app
- Localization implementation — UI layer, parameter crate only provides convention
- Actual localization files — per-plugin concern

---

## 11. Breaking Changes Summary

| Change | What Breaks | Migration |
|--------|------------|-----------|
| ParameterType variants removed (Date/DateTime/Time/Color/Hidden) | Code matching on these variants | Replace with String + InputHint |
| Parameter fields become `pub(crate)` | Direct struct construction | Use builder or derive |
| Builder API changes to typed closures | All manual `Parameter::string("id").label(...)` calls | Update to closure syntax |
| `Condition::evaluate` takes `&ParameterValues` | Internal callers passing `&HashMap` | Pass `&ParameterValues` |
| `ValidationError` gains `code`/`params` fields | Any code constructing `ValidationError` directly | Add new fields |
| Expression default flips to `true` | Parameters that assumed no-expression | Add `no_expression` where needed |
| Loader types unified to `Loader<T>` | Code importing `OptionLoader`/`RecordLoader`/`FilterFieldLoader` | Import `Loader<T>` |

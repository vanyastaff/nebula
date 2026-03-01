# API

## Public Surface

- **Stable APIs:**
  - `ParameterDef` — tagged enum of all 19 parameter types
  - `ParameterKind`, `ParameterCapability` — kind metadata and capability flags
  - `ParameterMetadata` — key, name, description, required, placeholder, hint, sensitive
  - `ParameterCollection` — ordered schema; `validate(&values)` entry point
  - `ParameterValues`, `ParameterSnapshot`, `ParameterDiff` — runtime value map with snapshot/diff
  - `ValidationRule` — declarative constraint schema (9 variants)
  - `ParameterDisplay`, `DisplayRuleSet`, `DisplayCondition`, `DisplayContext` — conditional visibility
  - `ParameterError` (#[non_exhaustive-like, `#[derive(Clone, PartialEq)]`]) — 9 variants with `code()`, `category()`, `is_retryable()`
  - `SelectOption`, `OptionsSource` — options for select/multi-select parameters
  - **Concrete type structs** (all re-exported from `types::*` via prelude): `TextParameter`, `TextareaParameter`, `CodeParameter`, `SecretParameter`, `NumberParameter`, `CheckboxParameter`, `SelectParameter`, `MultiSelectParameter`, `ColorParameter`, `DateTimeParameter`, `DateParameter`, `TimeParameter`, `HiddenParameter`, `NoticeParameter`, `ObjectParameter`, `ListParameter`, `ModeParameter`, `ModeVariant`, `GroupParameter`, `ExpirableParameter`
  - **Options structs** (re-exported from `types::*`): `TextOptions`, `NumberOptions`, `CodeOptions`, `ColorOptions`, etc.
  - **Enums from types**: `NoticeType`, `CodeLanguage`, `ColorFormat`, `ModeSelectorStyle`
  - `prelude::*` — single-import of all stable types
- **Internal/hidden:** None. All modules are `pub`.

---

## `ParameterError`

`#[derive(Debug, Clone, PartialEq, Eq)]` — all variants are deterministic; none retryable.

| Variant | Fields | `code()` | `category()` | `is_retryable()` |
|---|---|---|---|---|
| `InvalidKeyFormat` | `key`, `reason` | `PARAM_INVALID_KEY` | `format` | false |
| `NotFound` | `key` | `PARAM_NOT_FOUND` | `lookup` | false |
| `AlreadyExists` | `key` | `PARAM_ALREADY_EXISTS` | `lookup` | false |
| `InvalidType` | `key`, `expected_type`, `actual_details` | `PARAM_INVALID_TYPE` | `type` | false |
| `InvalidValue` | `key`, `reason` | `PARAM_INVALID_VALUE` | `value` | false |
| `MissingValue` | `key` | `PARAM_MISSING_VALUE` | `value` | false |
| `ValidationError` | `key`, `reason` | `PARAM_VALIDATION` | `validation` | false |
| `DeserializationError` | `key`, `error` | `PARAM_DESER` | `serialization` | false |
| `SerializationError` | `error` | `PARAM_SER` | `serialization` | false |

**Methods:**
- `code() -> &str` — machine-readable string, always starts with `"PARAM_"`
- `category() -> &str` — broad group: `"format"`, `"lookup"`, `"type"`, `"value"`, `"validation"`, `"serialization"`
- `is_retryable() -> bool` — always `false`; all parameter errors are deterministic

**Validation returns `Vec<ParameterError>`, collecting all failures** (not fail-fast). Callers pattern-match on `code()` or individual variants for UI/API error mapping.

---

## `ParameterKind`

19 variants, `Copy + Clone + PartialEq + Serialize + Deserialize`. Serde: `rename_all = "snake_case"`.

| Variant | `as_str()` | `value_type()` | `is_container()` | Notes |
|---|---|---|---|---|
| `Text` | `"text"` | `"string"` | false | Single-line |
| `Textarea` | `"textarea"` | `"string"` | false | Multi-line |
| `Secret` | `"secret"` | `"string"` | false | Masked; not `Displayable` |
| `Number` | `"number"` | `"number"` | false | f64 |
| `Checkbox` | `"checkbox"` | `"boolean"` | false | |
| `Select` | `"select"` | `"any"` | false | Single selection |
| `MultiSelect` | `"multi_select"` | `"array"` | false | |
| `DateTime` | `"date_time"` | `"string"` | false | ISO 8601 string |
| `Date` | `"date"` | `"string"` | false | |
| `Time` | `"time"` | `"string"` | false | |
| `Code` | `"code"` | `"string"` | false | Editor with language |
| `Color` | `"color"` | `"string"` | false | Hex/HSL/RGB |
| `Hidden` | `"hidden"` | `"any"` | false | Not displayed, not editable |
| `Notice` | `"notice"` | `"none"` | false | Display-only; no value |
| `Object` | `"object"` | `"object"` | **true** | Named fields |
| `List` | `"list"` | `"array"` | **true** | Repeated template |
| `Mode` | `"mode"` | `"object"` | **true** | Mutually exclusive variants |
| `Group` | `"group"` | `"none"` | **true** | Visual grouping; no value |
| `Expirable` | `"expirable"` | `"any"` | **true** | Wraps inner with expiry |

**Convenience predicates:**
- `has_value()` — `Notice` and `Group` return `false`
- `is_editable()` — user can change in UI
- `is_validatable()` — validation rules apply
- `is_displayable()` — shown in UI (`Secret` and `Hidden` return `false`)
- `is_requirable()` — can be marked required
- `supports_expressions()` — expression resolution supported
- `is_container()` — holds child `ParameterDef`s
- `is_text_based()` — `Text | Textarea | Secret | Code | Color`
- `is_selection_based()` — `Select | MultiSelect`
- `is_temporal()` — `DateTime | Date | Time`

**Capability access:** `capabilities() -> &'static [ParameterCapability]`, `has_capability(cap) -> bool`

---

## `ParameterCapability`

`Copy + Clone + PartialEq + Serialize + Deserialize`. 9 flags:

| Flag | Meaning |
|---|---|
| `HasValue` | Parameter carries a runtime value |
| `Editable` | User can edit in UI |
| `Validatable` | Validation rules applicable |
| `Displayable` | Shown in UI |
| `Requirable` | Can be marked required |
| `SupportsExpressions` | Expression resolution supported |
| `Container` | Holds child parameters |
| `Interactive` | Requires dynamic interaction (e.g., option loaders) |
| `Serializable` | Value can be serialized |

---

## `ParameterDef`

Tagged enum — 19 variants. Serde: `#[serde(tag = "type", rename_all = "snake_case")]`.

**JSON shape:** `{ "type": "text", "key": "host", "name": "Host", ... }`

**Variants:** `Text(TextParameter)`, `Textarea(TextareaParameter)`, `Code(CodeParameter)`, `Secret(SecretParameter)`, `Number(NumberParameter)`, `Checkbox(CheckboxParameter)`, `Select(SelectParameter)`, `MultiSelect(MultiSelectParameter)`, `Color(ColorParameter)`, `DateTime(DateTimeParameter)`, `Date(DateParameter)`, `Time(TimeParameter)`, `Hidden(HiddenParameter)`, `Notice(NoticeParameter)`, `Object(ObjectParameter)`, `List(ListParameter)`, `Mode(ModeParameter)`, `Group(GroupParameter)`, `Expirable(ExpirableParameter)`

**Methods:**

| Method | Return | Notes |
|---|---|---|
| `key()` | `&str` | Delegates to metadata |
| `name()` | `&str` | Delegates to metadata |
| `kind()` | `ParameterKind` | Matches variant |
| `metadata()` | `&ParameterMetadata` | |
| `metadata_mut()` | `&mut ParameterMetadata` | Post-construction mutation |
| `is_required()` | `bool` | `metadata.required` |
| `is_sensitive()` | `bool` | `metadata.sensitive` |
| `display()` | `Option<&ParameterDisplay>` | |
| `validation_rules()` | `&[ValidationRule]` | `Notice`/`Group` return `&[]` |
| `children()` | `Option<Vec<&ParameterDef>>` | `None` for scalars; `Some(_)` for containers |

**`children()` behavior by variant:**
- `Object` → field list
- `List` → `[item_template]` (single element)
- `Mode` → all parameters across all variants (flattened)
- `Group` → parameter list
- `Expirable` → `[inner]`
- All scalar variants → `None`

---

## `ParameterCollection`

Ordered list of `ParameterDef`s. Backed by `Vec<ParameterDef>` (insertion order preserved).

**Construction:**

```rust
// Builder pattern (consuming)
let col = ParameterCollection::new()
    .with(ParameterDef::Text(TextParameter::new("host", "Host")))
    .with(ParameterDef::Number(NumberParameter::new("port", "Port")));

// Mutation pattern
let mut col = ParameterCollection::new();
col.add(ParameterDef::Text(TextParameter::new("host", "Host")));

// From iterator
let col: ParameterCollection = defs.into_iter().collect();
```

**Full API:**

| Method | Signature | Notes |
|---|---|---|
| `new()` | `-> Self` | Empty collection |
| `with(param)` | `Self -> Self` | Builder-style, consuming |
| `add(&mut self, param)` | `-> &mut Self` | Mutation-style, chainable |
| `get(index)` | `-> Option<&ParameterDef>` | By position |
| `get_by_key(key)` | `-> Option<&ParameterDef>` | Linear scan |
| `remove(key)` | `-> Option<ParameterDef>` | Removes first match |
| `contains(key)` | `-> bool` | |
| `keys()` | `-> impl Iterator<Item = &str>` | Insertion order |
| `len()` | `-> usize` | |
| `is_empty()` | `-> bool` | |
| `iter()` | `-> impl Iterator<Item = &ParameterDef>` | |
| `iter_mut()` | `-> impl Iterator<Item = &mut ParameterDef>` | |
| `validate(&values)` | `-> Result<(), Vec<ParameterError>>` | All errors collected |

**`validate()` semantics:**

1. **Display-only parameters skipped**: `Notice` and `Group` (`value_type == "none"`) are never validated.
2. **Missing required**: `None` or `Value::Null` value for a `required` parameter → `MissingValue`.
3. **Type check**: Value JSON type must match `kind.value_type()`. On mismatch → `InvalidType`; rule evaluation is skipped.
4. **Rule evaluation**: All `ValidationRule`s evaluated in order; errors accumulated. **`Custom` rules are always skipped** — they require `ExpressionEngine` from the caller.
5. **Recursive container validation**: `Object` fields validated as `"outer.field"` paths; `List` items validated as `"list[0]"`, `"list[1]"` paths. Nesting is fully recursive.
6. **Extra keys ignored**: Values with no matching definition are silently skipped.

**Traits:** `IntoIterator` (by value and by reference), `FromIterator<ParameterDef>`, `Default`, `Clone`, `PartialEq`, `Serialize`, `Deserialize`

---

## `ValidationRule`

Declarative constraint schema. Actual evaluation is done by `nebula-validator` (via `ParameterCollection::validate`) or by the expression engine (for `Custom`).

Serde: `#[serde(tag = "rule", rename_all = "snake_case")]` — JSON: `{ "rule": "min_length", "length": 3 }`.

All variants carry `message: Option<String>` (`skip_serializing_if = "Option::is_none"`).

| Variant | Fields | Constructor | Applicable to |
|---|---|---|---|
| `MinLength { length }` | `length: usize` | `min_length(n)` | string kinds |
| `MaxLength { length }` | `length: usize` | `max_length(n)` | string kinds |
| `Pattern { pattern }` | `pattern: String` | `pattern(r"regex")` | string kinds |
| `Min { value }` | `value: f64` | `min(v)` | `Number` |
| `Max { value }` | `value: f64` | `max(v)` | `Number` |
| `OneOf { values }` | `values: Vec<Value>` | — (construct directly) | any |
| `Custom { expression }` | `expression: String` | — | any (engine only) |
| `MinItems { count }` | `count: usize` | `min_items(n)` | `List`, `MultiSelect` |
| `MaxItems { count }` | `count: usize` | `max_items(n)` | `List`, `MultiSelect` |

**`range(min, max) -> Vec<ValidationRule>`** — convenience that returns `[Min, Max]`.

**`.with_message(msg)`** — builder method, sets optional custom error text used in `ValidationError::reason`.

> **Important:** `Custom` rules are **never evaluated** by `ParameterCollection::validate()`. They are stored in the schema for the expression engine to evaluate at execution time.

---

## `ParameterMetadata`

Attached to every parameter type via `#[serde(flatten)]`.

| Field | Type | Serde | Default |
|---|---|---|---|
| `key` | `String` | always present | `""` |
| `name` | `String` | always present | `""` |
| `description` | `Option<String>` | `skip_if_none` | `None` |
| `required` | `bool` | `default` | `false` |
| `placeholder` | `Option<String>` | `skip_if_none` | `None` |
| `hint` | `Option<String>` | `skip_if_none` | `None` |
| `sensitive` | `bool` | `default` | `false` |

**Constructor:** `ParameterMetadata::new(key, name)` — all optional fields default to `None`/`false`.

`sensitive = true` means the value is masked in logs, UI, and exports. `SecretParameter::new(...)` sets `sensitive = true` automatically.

---

## `ParameterValues`

Runtime key→value map. Backed by `HashMap<String, serde_json::Value>`.
Serde: `#[serde(flatten)]` — serializes as flat JSON object (no `"values"` wrapper key).

**Full API:**

| Method | Signature | Notes |
|---|---|---|
| `new()` | `-> Self` | Empty |
| `set(key, value)` | `(&mut self, impl Into<String>, Value)` | Overwrites |
| `get(key)` | `-> Option<&Value>` | |
| `remove(key)` | `-> Option<Value>` | |
| `contains(key)` | `-> bool` | |
| `keys()` | `-> impl Iterator<Item = &str>` | Iteration order not guaranteed |
| `len()` | `-> usize` | |
| `is_empty()` | `-> bool` | |
| `get_string(key)` | `-> Option<&str>` | Returns `None` if value is not a string |
| `get_f64(key)` | `-> Option<f64>` | Returns `None` if value is not a number |
| `get_bool(key)` | `-> Option<bool>` | Returns `None` if value is not a boolean |
| `snapshot()` | `-> ParameterSnapshot` | Frozen copy |
| `restore(&snapshot)` | | Replaces entire value set |
| `diff(&other)` | `-> ParameterDiff` | Added/removed/changed keys |
| `Index<&str>` | panics on missing key | Use `get()` for fallible access |

**`FromIterator<(String, Value)>`** — construct from key-value pairs.

### `ParameterSnapshot`

Opaque frozen copy of `ParameterValues`. Fields are private. Use `values.restore(&snapshot)` to roll back.

### `ParameterDiff`

Returned by `values.diff(&other)`. All fields `pub Vec<String>`, sorted alphabetically.

```rust
pub struct ParameterDiff {
    pub added: Vec<String>,    // in `other` but not `self`
    pub removed: Vec<String>,  // in `self` but not `other`
    pub changed: Vec<String>,  // in both, with different values
}
```

---

## `SelectOption` and `OptionsSource`

### `SelectOption`

```rust
pub struct SelectOption {
    pub key: String,                    // machine-readable id
    pub name: String,                   // display label
    pub value: serde_json::Value,       // the submitted value
    pub description: Option<String>,    // tooltip (skip_if_none)
    pub disabled: bool,                 // shown but not selectable (default: false)
}
```

Constructor: `SelectOption::new(key, name, value)`.

### `OptionsSource`

Serde: `#[serde(tag = "source", rename_all = "snake_case")]`.

| Variant | Fields | JSON |
|---|---|---|
| `Static { options }` | `options: Vec<SelectOption>` | `"source": "static"` |
| `Dynamic { loader_key }` | `loader_key: String` | `"source": "dynamic"` |

`Dynamic` is declared in the schema; resolution is handled by the runtime layer, not this crate.

---

## Display System

Controls conditional visibility of parameters in the UI. All types are serializable.

### `DisplayCondition`

16 variants. Serde: `#[serde(tag = "condition", rename_all = "snake_case")]`.

| Variant | Fields | Evaluates to `true` when... |
|---|---|---|
| `Equals { value }` | `value: Value` | field equals `value` |
| `NotEquals { value }` | `value: Value` | field does not equal `value` |
| `IsSet` | — | field is not null |
| `IsNull` | — | field is null |
| `IsEmpty` | — | string is `""`, array is `[]`, or value is null |
| `IsNotEmpty` | — | string non-empty, array non-empty, or non-null non-string non-array |
| `IsTrue` | — | `value == true` (strict boolean) |
| `IsFalse` | — | `value == false` (strict boolean) |
| `GreaterThan { value }` | `value: f64` | field > threshold |
| `LessThan { value }` | `value: f64` | field < threshold |
| `InRange { min, max }` | `min, max: f64` | `min <= field <= max` (inclusive) |
| `Contains { value }` | `value: Value` | string contains substring, or array contains element |
| `StartsWith { prefix }` | `prefix: String` | string starts with prefix |
| `EndsWith { suffix }` | `suffix: String` | string ends with suffix |
| `OneOf { values }` | `values: Vec<Value>` | field is one of the listed values |
| `IsValid` | — | field's validation state is `true` (requires `DisplayContext`) |

**`evaluate(&value) -> bool`** — evaluates against a concrete `serde_json::Value`. For `IsValid`, always returns `false`; use `DisplayRule::evaluate(&context)` instead.

**Missing field** in context: treated as `Value::Null`.

### `DisplayRule`

```rust
pub struct DisplayRule {
    pub field: String,              // key of the sibling parameter to check
    pub condition: DisplayCondition,
}
```

`evaluate(&context) -> bool` — special-cases `IsValid` to check `context.get_validation(field)`.

### `DisplayRuleSet`

Composable logic. Serde: `#[serde(tag = "logic", rename_all = "snake_case")]`.

| Variant | Semantics |
|---|---|
| `Single(DisplayRule)` | The single rule's result |
| `All { rules }` | All rules must match (AND) |
| `Any { rules }` | At least one must match (OR) |
| `Not { rule: Box<_> }` | Negates the nested rule |

**`evaluate(&context) -> bool`**, **`dependencies() -> Vec<String>`** (sorted, deduplicated field names).

### `ParameterDisplay`

```rust
pub struct ParameterDisplay {
    pub show_when: Vec<DisplayRuleSet>,  // skip_if_empty
    pub hide_when: Vec<DisplayRuleSet>,  // skip_if_empty
}
```

**`should_display(&context) -> bool`** logic:
1. If any `hide_when` rule matches → `false` (hide takes priority)
2. If `show_when` is empty → `true` (visible by default)
3. If at least one `show_when` rule matches → `true`
4. Otherwise → `false`

**`dependencies() -> Vec<String>`** — all field names referenced, sorted and deduplicated.

**`is_empty() -> bool`** — no show/hide rules at all.

### `DisplayContext`

Builder for display rule evaluation. Not serializable.

```rust
let ctx = DisplayContext::new()
    .with_value("mode", json!("advanced"))
    .with_validation("email", true);

ctx.get("mode")           // -> Option<&Value>
ctx.get_validation("email") // -> Option<bool>
```

---

## Concrete Parameter Type Structs

All concrete structs share a common layout:

```rust
pub struct XxxParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,        // key, name, ...
    pub default: Option<_>,                  // type-specific default
    pub options: Option<XxxOptions>,         // type-specific UI/behavior options
    pub display: Option<ParameterDisplay>,   // conditional visibility
    pub validation: Vec<ValidationRule>,     // declarative constraints
}
```

Constructor: `XxxParameter::new(key, name)` — all optional fields default to `None`/empty.

| Struct | Default type | Notable options/fields |
|---|---|---|
| `TextParameter` | `Option<String>` | `TextOptions { pattern, max_length, min_length }` |
| `TextareaParameter` | `Option<String>` | `TextareaOptions { rows }` |
| `CodeParameter` | `Option<String>` | `CodeOptions { language: CodeLanguage, ... }` |
| `SecretParameter` | `Option<String>` | `sensitive = true` by default |
| `NumberParameter` | `Option<f64>` | `NumberOptions { min, max, step, precision }` |
| `CheckboxParameter` | `Option<bool>` | `CheckboxOptions` |
| `SelectParameter` | `Option<Value>` | `options: Vec<SelectOption>` in options |
| `MultiSelectParameter` | `Option<Vec<Value>>` | `options: Vec<SelectOption>` |
| `ColorParameter` | `Option<String>` | `ColorOptions { format: ColorFormat }` |
| `DateTimeParameter` | `Option<String>` | ISO 8601 string |
| `DateParameter` | `Option<String>` | ISO date string |
| `TimeParameter` | `Option<String>` | Time string |
| `HiddenParameter` | `Option<Value>` | Not displayed, not editable |
| `NoticeParameter` | — | `notice_type: NoticeType`, `content: String`; no default/validation |
| `ObjectParameter` | — | `fields: Vec<ParameterDef>`; `.with_field(param)` builder |
| `ListParameter` | — | `item_template: Box<ParameterDef>`; `ListParameter::new(key, name, template)` |
| `ModeParameter` | — | `variants: Vec<ModeVariant>`, `default_variant: Option<String>`; `.get_variant(key)` |
| `GroupParameter` | — | `parameters: Vec<ParameterDef>`; `.with_parameter(param)` builder |
| `ExpirableParameter` | — | `inner: Box<ParameterDef>`; `ExpirableParameter::new(key, name, inner)` |

**Container constructors that differ from pattern:**

```rust
// ListParameter requires a template
let list = ListParameter::new("emails", "Emails",
    ParameterDef::Text(TextParameter::new("email", "Email")));

// ObjectParameter uses .with_field()
let obj = ObjectParameter::new("db", "Database")
    .with_field(ParameterDef::Text(TextParameter::new("host", "Host")))
    .with_field(ParameterDef::Number(NumberParameter::new("port", "Port")));

// ExpirableParameter wraps an inner parameter
let expirable = ExpirableParameter::new("token", "Token",
    ParameterDef::Secret(SecretParameter::new("val", "Value")));

// ModeVariant groups parameters for a single mode
let variant = ModeVariant::new("api_key", "API Key")
    .with_parameter(ParameterDef::Secret(SecretParameter::new("key", "API Key")));
```

**`NoticeParameter::new(key, name, notice_type, content)` — 4-arg constructor.**

**`NoticeType`:** `Info`, `Warning`, `Error`, `Success`

**`ModeSelectorStyle`:** `Dropdown`, `Radio`, `Tabs` (serde: `rename_all = "snake_case"`)

---

## Usage Patterns

- Build schema with `ParameterCollection::new().with(...)` (builder) or `.add(...)` (mutate).
- Populate values with `ParameterValues::set(key, value)`.
- Validate before execution: `collection.validate(&values)` — aggregates all errors.
- Use `prelude::*` for common imports.
- Use `display.should_display(&ctx)` in UI layer to conditionally render fields.
- Snapshot-restore for undo: `let snap = values.snapshot(); ... values.restore(&snap)`.
- Diff for change detection: `let diff = old.diff(&new); if !diff.changed.is_empty() { ... }`.

---

## Minimal Example

```rust
use nebula_parameter::prelude::*;
use serde_json::json;

let col = ParameterCollection::new()
    .with(ParameterDef::Text(TextParameter::new("host", "Host")))
    .with(ParameterDef::Number({
        let mut p = NumberParameter::new("port", "Port");
        p.metadata.required = true;
        p.validation = ValidationRule::range(1.0, 65535.0);
        p
    }));

let mut values = ParameterValues::new();
values.set("host", json!("localhost"));
values.set("port", json!(5432));

match col.validate(&values) {
    Ok(()) => println!("valid"),
    Err(errors) => {
        for e in &errors {
            eprintln!("[{}] {}", e.code(), e);
        }
    }
}
```

---

## Advanced Example

```rust
use nebula_parameter::prelude::*;
use serde_json::json;

// Mode parameter: API Key vs OAuth
let mut mode = ModeParameter::new("auth", "Authentication");
mode.variants.push(
    ModeVariant::new("api_key", "API Key")
        .with_parameter(ParameterDef::Secret(SecretParameter::new("key", "API Key"))),
);
mode.variants.push(
    ModeVariant::new("oauth", "OAuth")
        .with_parameter(ParameterDef::Text(TextParameter::new("client_id", "Client ID")))
        .with_parameter(ParameterDef::Secret(SecretParameter::new("secret", "Client Secret"))),
);

// Display rule: show cert_path only when tls is enabled
let cert_path = {
    let mut p = TextParameter::new("cert_path", "Certificate Path");
    p.display = Some(ParameterDisplay {
        show_when: vec![DisplayRuleSet::Single(DisplayRule {
            field: "tls".into(),
            condition: DisplayCondition::IsTrue,
        })],
        hide_when: vec![],
    });
    p
};

let col = ParameterCollection::new()
    .with(ParameterDef::Mode(mode))
    .with(ParameterDef::Checkbox(CheckboxParameter::new("tls", "Use TLS")))
    .with(ParameterDef::Text(cert_path));

// Validate
let mut values = ParameterValues::new();
values.set("auth", json!({"api_key": "secret-key"}));
values.set("tls", json!(false));

assert!(col.validate(&values).is_ok());

// Diff
let mut updated = values.clone();
updated.set("tls", json!(true));
let diff = values.diff(&updated);
assert_eq!(diff.changed, vec!["tls"]);
```

---

## Error Semantics

- **`validate()` collects all errors** — never stops at first failure. Callers receive `Vec<ParameterError>`.
- **`Custom` validation rules** are skipped by `validate()` and must be evaluated by the expression engine before calling `validate()`.
- **All `ParameterError` variants are deterministic** — same input always produces same errors. `is_retryable()` is always `false`.
- **Nested error paths**: Object fields → `"parent.field"`, List items → `"list[0]"`, combined → `"request.headers[1].value"`.
- **Type error skips rules**: If type check fails, validation rules are not evaluated for that parameter.
- **Extra keys in `ParameterValues` are ignored** — no error for unknown keys.
- API layer: map `code()` → HTTP 400/422; `category()` for grouping in error responses.

---

## Compatibility Rules

- **Major version bump required:**
  - Adding/removing `ParameterDef` variants
  - Changing `ValidationRule` variant fields
  - Changing `ParameterError` variant fields or `code()` strings
  - Changing `validate()` semantics (e.g., switching to fail-fast)
  - Changing serde representations (`tag` field names, `rename_all`)
- **Minor bump (backward-compatible):**
  - Adding new `DisplayCondition` variants (unknown variants will fail to deserialize on old clients)
  - Adding optional fields with `skip_if_none` / `default`
- **Deprecation policy:** Deprecate in minor release; remove in next major; minimum 6 months (see MIGRATION.md)

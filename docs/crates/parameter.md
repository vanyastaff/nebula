# nebula-parameter

Parameter definition system for workflow nodes. Describes _what_ inputs a node accepts: type,
UI widget, metadata, validation rules, and conditional display logic. Values at runtime are
plain `serde_json::Value`.

**Depends on:** `nebula-validator`

---

## Design Philosophy

Parameters define **data shape and constraints**, not visual appearance.

**Parameters own:**
- Type semantics (which JSON type, which capabilities)
- Validation constraints (`min`, `max`, `pattern`, `required`)
- Conditional visibility rules (`show_when`, `hide_when`)
- Sensitive/secret flag
- Structural nesting (Object fields, List item template, Mode variants)

**Platform/engine owns (do not encode in parameters):**
- Widget dimensions (height, width, column count)
- Visual styling (colors, themes, spacing)
- Standard behaviors (character counter, auto-format, loading states)
- Expression toggle UI and variable auto-completion
- Accessibility and error recovery UI

**Expression pipeline** — values stored in `ParameterValues` can be literal or
expression strings (resolved by `nebula-expression`). The engine applies a
three-phase contract before action execution:
1. **Transform** — resolve `$nodes.…` expressions to concrete `serde_json::Value`
2. **Validate** — run `ParameterCollection::validate` against the resolved values
3. **Execute** — pass clean values to the action

---

## Module Map

| Module | What it provides |
|---|---|
| `kind` | `ParameterKind` enum + `ParameterCapability` flags |
| `def` | `ParameterDef` — tagged enum wrapping concrete types |
| `metadata` | `ParameterMetadata` — key, name, hints, required, sensitive |
| `collection` | `ParameterCollection` — ordered list of defs, with validation |
| `values` | `ParameterValues` — runtime key→value map, snapshot/diff |
| `validation` | `ValidationRule` — declarative constraint descriptors |
| `display` | `ParameterDisplay`, `DisplayCondition`, `DisplayRuleSet` |
| `option` | `SelectOption`, `OptionsSource` |
| `types/*` | One struct per `ParameterKind` variant |
| `error` | `ParameterError` |

---

## ParameterKind

19 variants, each mapped to a UI widget and a JSON value type.

### Scalar types

| Kind | JSON type | Notes |
|---|---|---|
| `Text` | `string` | Single-line input |
| `Textarea` | `string` | Multi-line input |
| `Secret` | `string` | Masked, not `Displayable` |
| `Number` | `number` | `f64` range |
| `Checkbox` | `boolean` | |
| `Select` | `any` | Single selection from options |
| `MultiSelect` | `array` | Multiple selections |
| `DateTime` | `string` | ISO 8601 datetime |
| `Date` | `string` | ISO 8601 date |
| `Time` | `string` | ISO 8601 time |
| `Code` | `string` | Code editor widget |
| `Color` | `string` | Color picker |
| `Hidden` | `any` | Has value, invisible in UI |
| `Notice` | `none` | Display-only message, no value |

### Container types

| Kind | JSON type | Children |
|---|---|---|
| `Object` | `object` | Named fields (`Vec<ParameterDef>`) |
| `List` | `array` | Homogeneous items (`item_template`) |
| `Mode` | `object` | Mutually-exclusive variants |
| `Group` | `none` | Visual grouping, no own value |
| `Expirable` | `any` | Wraps a single inner parameter |

### Capability flags

Each kind exposes a `&'static [ParameterCapability]`:

```
HasValue          — carries a runtime value
Editable          — user can type/select in UI
Validatable       — ValidationRules can be attached
Displayable       — visible in UI
Requirable        — can be marked required
SupportsExpressions — engine can resolve `$nodes.…` expressions
Container         — holds child parameters
Interactive       — needs special UI (editor, picker, dropdown)
Serializable      — round-trips through serde_json
```

Convenience methods on `ParameterKind`:

```rust
kind.has_value()           // HasValue
kind.is_editable()         // Editable
kind.is_validatable()      // Validatable
kind.is_displayable()      // Displayable
kind.is_requirable()       // Requirable
kind.supports_expressions()// SupportsExpressions
kind.is_container()        // Container
kind.is_text_based()       // Text | Textarea | Secret | Code | Color
kind.is_selection_based()  // Select | MultiSelect
kind.is_temporal()         // DateTime | Date | Time
kind.value_type()          // "string" | "number" | "boolean" | "array" | "object" | "any" | "none"
```

Notable edge cases:
- `Secret` — has value, editable, but **not** displayable (masked)
- `Hidden` — has value, **not** editable, **not** displayable
- `Notice` — displayable only, no value, no validation
- `Group` — container + displayable, no own value

---

## ParameterDef

Tagged enum (`#[serde(tag = "type", rename_all = "snake_case")]`) — one variant per kind:

```rust
pub enum ParameterDef {
    Text(TextParameter),
    Textarea(TextareaParameter),
    Code(CodeParameter),
    Secret(SecretParameter),
    Number(NumberParameter),
    Checkbox(CheckboxParameter),
    Select(SelectParameter),
    MultiSelect(MultiSelectParameter),
    Color(ColorParameter),
    DateTime(DateTimeParameter),
    Date(DateParameter),
    Time(TimeParameter),
    Hidden(HiddenParameter),
    Notice(NoticeParameter),
    Object(ObjectParameter),
    List(ListParameter),
    Mode(ModeParameter),
    Group(GroupParameter),
    Expirable(ExpirableParameter),
}
```

Shared accessor methods (delegate to the inner struct's `metadata` field):

```rust
def.key()              // &str — unique within parent scope
def.name()             // &str — display label
def.kind()             // ParameterKind
def.metadata()         // &ParameterMetadata
def.metadata_mut()     // &mut ParameterMetadata
def.is_required()      // bool
def.is_sensitive()     // bool
def.display()          // Option<&ParameterDisplay>
def.validation_rules() // &[ValidationRule]  — &[] for Notice/Group
def.children()         // Option<Vec<&ParameterDef>> — Some for containers
```

`children()` shapes by container:
- `Object` → all fields
- `List` → `[item_template]`
- `Mode` → all parameters from all variants (flattened)
- `Group` → all child parameters
- `Expirable` → `[inner]`

JSON round-trip example:

```json
{
  "type": "object",
  "key": "connection",
  "name": "Connection",
  "fields": [
    { "type": "text",   "key": "host", "name": "Host" },
    { "type": "number", "key": "port", "name": "Port", "default": 5432.0 }
  ]
}
```

---

## ParameterMetadata

Human-facing descriptor embedded in every parameter struct:

```rust
pub struct ParameterMetadata {
    pub key: String,                      // required: unique key
    pub name: String,                     // required: display label
    pub description: Option<String>,      // tooltip / help text
    pub required: bool,                   // default false
    pub placeholder: Option<String>,      // empty-field hint
    pub hint: Option<String>,             // short inline hint
    pub sensitive: bool,                  // mask in UI/logs; default false
}
```

Optional fields are omitted from JSON when `None`. `SecretParameter` sets `sensitive = true`
automatically.

---

## ParameterCollection

Ordered `Vec<ParameterDef>` with lookup and validation:

```rust
// Construction
let col = ParameterCollection::new()
    .with(ParameterDef::Text(TextParameter::new("host", "Host")))
    .with(ParameterDef::Number(NumberParameter::new("port", "Port")));

// Lookup
col.get(0)               // Option<&ParameterDef>  — by index
col.get_by_key("host")   // Option<&ParameterDef>  — by key
col.contains("port")     // bool
col.keys()               // impl Iterator<Item = &str>
col.len() / col.is_empty()
col.iter() / col.iter_mut()

// Mutation
col.add(def)             // &mut Self
col.remove("host")       // Option<ParameterDef>

// Validation
col.validate(&values)    // Result<(), Vec<ParameterError>>
```

### Validation algorithm

`ParameterCollection::validate` iterates all defs and for each:

1. **Display-only skip** — `Notice` and `Group` (`value_type == "none"`) are skipped.
2. **Required check** — missing or `null` value → `MissingValue`.
3. **Type check** — JSON value kind must match `kind.value_type()`.
4. **Rule evaluation** — `ValidationRule`s are applied via `nebula-validator`.
5. **Recursive descent** — `Object` fields and `List` items are validated with dotted paths
   (`"connection.host"`, `"items[1].name"`).

Errors are **collected**, not short-circuited. `Custom` rules are skipped here (require
the expression engine).

```rust
let mut values = ParameterValues::new();
values.set("host", json!("localhost"));
values.set("port", json!(5432));

match col.validate(&values) {
    Ok(()) => { /* all good */ }
    Err(errors) => {
        for e in &errors {
            eprintln!("{} [{}]", e, e.code());
        }
    }
}
```

---

## ParameterValues

Runtime map from parameter key to `serde_json::Value`:

```rust
let mut vals = ParameterValues::new();
vals.set("host", json!("localhost"));
vals.set("port", json!(8080));

vals.get("host")          // Option<&Value>
vals.get_string("host")   // Option<&str>
vals.get_f64("port")      // Option<f64>
vals.get_bool("tls")      // Option<bool>
vals.contains("host")     // bool
vals.remove("host")       // Option<Value>
vals.keys()               // impl Iterator<Item = &str>
vals["host"]              // &Value  (panics if missing)

// Snapshot / restore
let snap = vals.snapshot();
vals.set("host", json!("changed"));
vals.restore(&snap);  // back to original

// Diff
let diff = a.diff(&b);
// diff.added:   keys in b not in a
// diff.removed: keys in a not in b
// diff.changed: keys in both with different values
```

Serializes as a flat JSON object (no nesting under `"values"`).

---

## ValidationRule

Pure data descriptions of constraints — serializable, no logic:

```rust
pub enum ValidationRule {
    MinLength { length: usize, message: Option<String> },
    MaxLength { length: usize, message: Option<String> },
    Pattern   { pattern: String, message: Option<String> },
    Min       { value: f64, message: Option<String> },
    Max       { value: f64, message: Option<String> },
    OneOf     { values: Vec<Value>, message: Option<String> },
    MinItems  { count: usize, message: Option<String> },
    MaxItems  { count: usize, message: Option<String> },
    Custom    { expression: String, message: Option<String> },  // engine-evaluated
}
```

Builder constructors and chaining:

```rust
ValidationRule::min_length(3)
ValidationRule::max_length(255)
ValidationRule::pattern(r"^[^@]+@[^@]+$")
ValidationRule::min(1.0)
ValidationRule::max(65535.0)
ValidationRule::range(1.0, 65535.0)   // returns Vec<ValidationRule>
ValidationRule::min_items(1)
ValidationRule::max_items(10)

// attach custom message
ValidationRule::min(18.0).with_message("must be 18 or older")
```

Serialized form uses `"rule"` as the tag:

```json
{ "rule": "min_length", "length": 3, "message": "too short" }
{ "rule": "pattern",    "pattern": "^\\w+$" }
```

---

## Display System

Controls whether a parameter is shown in the UI based on sibling field values.

### DisplayCondition

15 condition variants:

```
Equals { value }     NotEquals { value }
IsSet                IsNull
IsEmpty              IsNotEmpty
IsTrue               IsFalse
GreaterThan { value: f64 }  LessThan { value: f64 }
InRange { min, max }
Contains { value }
StartsWith { prefix }  EndsWith { suffix }
OneOf { values }
IsValid              — checks validation state, not raw value
```

### DisplayRuleSet

Composable via `Single`, `All`, `Any`, `Not`:

```rust
DisplayRuleSet::All {
    rules: vec![
        DisplayRuleSet::Single(DisplayRule {
            field: "mode".into(),
            condition: DisplayCondition::Equals { value: json!("advanced") },
        }),
        DisplayRuleSet::Not {
            rule: Box::new(DisplayRuleSet::Single(DisplayRule {
                field: "locked".into(),
                condition: DisplayCondition::IsTrue,
            })),
        },
    ],
}
```

### ParameterDisplay

```rust
pub struct ParameterDisplay {
    pub show_when: Vec<DisplayRuleSet>,  // any must match; empty = always visible
    pub hide_when: Vec<DisplayRuleSet>,  // any match = hidden; takes priority
}
```

Evaluation: `hide_when` is checked first; if any matches, hidden regardless of `show_when`.

```rust
let visible = display.should_display(&context);
let deps = display.dependencies(); // Vec<String> — all referenced field keys
```

`DisplayContext` carries current field values and validation states:

```rust
let ctx = DisplayContext::new()
    .with_value("mode", json!("advanced"))
    .with_validation("email", true);
```

---

## Concrete Parameter Types

Every type struct follows the same shape:

```rust
pub struct XxxParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,
    pub display: Option<ParameterDisplay>,
    pub validation: Vec<ValidationRule>,
    // type-specific fields…
}
```

Type-specific additions:

| Type | Extra fields |
|---|---|
| `NumberParameter` | `default: Option<f64>`, `options: NumberOptions` (step, integer_only) |
| `SelectParameter` | `options: Vec<SelectOption>`, `options_source: Option<OptionsSource>` |
| `MultiSelectParameter` | same as Select |
| `CodeParameter` | `language: CodeLanguage`, `options: CodeOptions` |
| `ColorParameter` | `format: ColorFormat` (hex, rgb, hsl) |
| `NoticeParameter` | `notice_type: NoticeType` (info, warning, error, success) |
| `ObjectParameter` | `fields: Vec<ParameterDef>` |
| `ListParameter` | `item_template: Box<ParameterDef>` |
| `ModeParameter` | `variants: Vec<ModeVariant>`, `default_variant: Option<String>` |
| `GroupParameter` | `parameters: Vec<ParameterDef>` |
| `ExpirableParameter` | `inner: Box<ParameterDef>` |

`ModeVariant`:

```rust
pub struct ModeVariant {
    pub key: String,
    pub name: String,
    pub parameters: Vec<ParameterDef>,
}
```

### SelectOption / OptionsSource

```rust
pub struct SelectOption {
    pub key: String,
    pub name: String,
    pub value: serde_json::Value,
    pub description: Option<String>,
    pub disabled: bool,
}

pub enum OptionsSource {
    Static  { options: Vec<SelectOption> },
    Dynamic { loader_key: String },         // resolved at runtime
}
```

---

## ParameterError

```rust
pub enum ParameterError {
    InvalidKeyFormat { key, reason },   // "format"       PARAM_INVALID_KEY
    NotFound         { key },           // "lookup"       PARAM_NOT_FOUND
    AlreadyExists    { key },           // "lookup"       PARAM_ALREADY_EXISTS
    InvalidType      { key, expected_type, actual_details }, // "type"  PARAM_INVALID_TYPE
    InvalidValue     { key, reason },   // "value"        PARAM_INVALID_VALUE
    MissingValue     { key },           // "value"        PARAM_MISSING_VALUE
    ValidationError  { key, reason },   // "validation"   PARAM_VALIDATION
    DeserializationError { key, error },// "serialization" PARAM_DESER
    SerializationError   { error },     // "serialization" PARAM_SER
}
```

All variants are deterministic (`is_retryable() == false`). Use `err.category()` for log
bucketing and `err.code()` for programmatic handling.

---

## Prelude

```rust
use nebula_parameter::prelude::*;
// ParameterCollection, ParameterDef, ParameterDisplay, DisplayCondition,
// DisplayContext, DisplayRuleSet, ParameterError, ParameterKind, ParameterCapability,
// ParameterMetadata, OptionsSource, SelectOption, ValidationRule, ParameterValues,
// and all concrete type structs (TextParameter, NumberParameter, …)
```

---

## Usage in the Crate Ecosystem

- **nebula-action** — `Action` trait implementations attach a `ParameterCollection`
  describing their inputs. The engine reads this schema to build the UI and validate
  user-supplied values before execution.
- **nebula-expression** — expression parameters (`SupportsExpressions` kinds) are resolved
  before values reach the action. The `Custom` `ValidationRule` is also evaluated by the
  expression engine.
- **nebula-credential** — credential parameters (typically `Secret` kind) follow the same
  schema path but their values are resolved from a credential store, not from user input directly.

### Archive references

- `docs/archive/node-execution.md` — `nebula-parameter` in the Node Layer dependency graph
- `docs/archive/layers-interaction.md` — interaction with `nebula-validator`
- `docs/archive/crates-dependencies.md` — used by `nebula-node`, `nebula-action`, `nebula-credential`

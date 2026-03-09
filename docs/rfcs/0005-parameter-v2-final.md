# RFC 0005: Parameter v2 — Final Design

**Type:** Standards Track RFC  
**Status:** Accepted  
**Created:** 2026-03-08  
**Updated:** 2026-03-08  
**Authors:** Nebula core team  
**Supersedes:** RFC 0001 (`0001-parameter-schema-v2.md`), Working Paper (`0001-parameter-api-v2.md`),
Playground (`0001-v2-universality-playground.md`), RFC 0002, RFC 0004  
**Informs:** RFC 0003 (gap analysis, no design changes)  
**Target:** `nebula-parameter` v1.0  

---

## Summary

This RFC resolves all open questions from RFC 0001–0004 and defines the **single authoritative
design** for the v2 parameter schema. All earlier drafts and working papers are superseded.

Decisions locked in this RFC:

1. `FieldMeta` is flattened into every `Field` variant; `id` and `label` live there.
2. `Field` enum is complete: 16 variants including `DynamicRecord` and `Predicate`.
3. `Condition` is complete: 13 operators covering comparisons, presence, and combinators.
4. `Rule` is complete: 9 variants covering length, range, pattern, membership, uniqueness.
5. `OptionProvider` uses RPITIT (`-> impl Future`); `DynamicRecordProvider` uses `async_trait`.
6. `secret: bool` in `FieldMeta` is the only wire-level secret control.
7. `Condition` evaluation is a small deterministic evaluator shared across Rust and JS.
8. `Rule::Custom.expression` delegated to `nebula-expression` under policy; not evaluated inline.
9. `DynamicRecord` and `Predicate` are first-class `Field` variants (RFC 0004).
10. Core presets module provides reusable field builders (RFC 0002).

---

## Design Principles

1. **Wire contract is the Rust struct.** `Schema` serializes directly to the JSON API contract.  
   No separate DTO or view-model layer at the boundary.
2. **Invalid states are unrepresentable.** Enums encode all legal variants. UI-only nodes never
   appear in value payloads.
3. **No stringly-configured behavior.** Sentinel field names, control hint strings, and ad-hoc
   `hint` bags are replaced by typed enums (`EditorLanguage`, `Severity`, `DynamicRecordMode`).
4. **Deterministic ordering everywhere.** Field traversal, error emission, and variant iteration
   follow declaration order. `IndexMap` for all ordered maps; no `HashMap` in schema types.
5. **Container semantics explicit.** `List`, `Object`, `Mode`, `DynamicRecord`, `Predicate` are
   distinct types — not emulated with generic blobs.
6. **Security by design.** `secret: bool` triggers redaction in logs, errors, and API responses.
   Secret values never appear in `FieldError.message`.
7. **Expression boundary is clear.** `Condition` (form state) ≠ `Rule::Custom` (expression
   engine) ≠ `expression: bool` (field capability). Each has its own evaluation path.

---

## Module Layout

```
nebula_parameter::schema       — Field, Schema, FieldMeta, Condition, Rule, UiElement, Group
nebula_parameter::runtime      — ParameterValues, ValidatedValues, FieldError
nebula_parameter::providers    — DynamicProviderEnvelope, OptionProvider, DynamicRecordProvider
nebula_parameter::presets      — core_fields builders (branch_target, retry_policy, …)
nebula_parameter::migration    — import_v1_json (tooling-only surface)
```

Legacy modules (`def`, `collection`, `display`, `kind`, `subtype`) remain accessible for
migration but are not part of the v2 public runtime surface.

---

## JSON Wire Contract

The canonical JSON shape sent over HTTP. Frontends render forms directly from this.

```json
{
  "fields": [
    {
      "type": "mode",
      "id": "auth",
      "label": "Authentication",
      "default_variant": "none",
      "variants": [
        { "key": "none",   "label": "None",
          "content": { "type": "hidden",  "id": "_none", "label": "_" } },
        { "key": "bearer", "label": "Bearer Token",
          "content": { "type": "text", "id": "token", "label": "Token",
                       "secret": true, "required": true } },
        { "key": "basic",  "label": "Basic Auth",
          "content": { "type": "object", "id": "credentials", "label": "Credentials",
            "fields": [
              { "type": "text", "id": "username", "label": "Username", "required": true },
              { "type": "text", "id": "password", "label": "Password",
                "secret": true, "required": true }
            ]
          }
        }
      ]
    },
    {
      "type": "select", "id": "method", "label": "HTTP Method",
      "required": true, "default": "GET",
      "source": "static",
      "options": [
        { "value": "GET",    "label": "GET"    },
        { "value": "POST",   "label": "POST"   },
        { "value": "PUT",    "label": "PUT"    },
        { "value": "DELETE", "label": "DELETE" },
        { "value": "PATCH",  "label": "PATCH"  }
      ]
    },
    {
      "type": "text", "id": "url", "label": "URL",
      "placeholder": "https://api.example.com/endpoint",
      "required": true,
      "rules": [
        { "rule": "pattern", "pattern": "^https?://", "message": "Must be a valid URL" }
      ]
    },
    {
      "type": "number", "id": "timeout", "label": "Timeout (ms)",
      "integer": true, "default": 30000, "min": 100, "max": 120000
    },
    {
      "type": "boolean", "id": "retry_enabled", "label": "Enable Retry", "default": false
    },
    {
      "type": "number", "id": "max_retries", "label": "Max Retries",
      "integer": true, "default": 3, "min": 1, "max": 10,
      "visible_when": { "op": "is_true", "field": "retry_enabled" }
    }
  ],
  "ui": [
    {
      "kind": "notice", "severity": "warning",
      "text": "API may return 429 errors. Enable retry for stability.",
      "visible_when": { "op": "is_false", "field": "retry_enabled" }
    }
  ],
  "groups": [
    { "label": "Request",  "fields": ["method", "url"] },
    { "label": "Advanced", "fields": ["timeout", "retry_enabled", "max_retries"],
      "collapsed": true }
  ]
}
```

---

## Rust Types

### `Schema`

```rust
/// Complete parameter schema.
///
/// Serializes directly to the JSON API contract. Frontends render forms,
/// validate input, and evaluate visibility conditions from this struct.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Schema {
    /// Ordered field definitions. Declaration order is canonical.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<Field>,

    /// UI-only elements (notices, buttons). Never appear in the value payload.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ui: Vec<UiElement>,

    /// Visual field grouping. References field `id`s.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<Group>,
}
```

---

### `Field`

The `"type"` key is the discriminator. `FieldMeta` (containing `id` and `label`) is
flattened into every variant, so the JSON shape is flat.

```rust
/// A single schema field.
///
/// `type` discriminates the widget and which variant properties are valid.
/// `id` and `label` plus conditional/security metadata live in the flattened `FieldMeta`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Field {
    // ── Scalar fields ────────────────────────────────────────────────────────
    /// Free-form text input. Set `multiline: true` for a textarea.
    Text {
        #[serde(flatten)]
        meta: FieldMeta,
        /// Render as multi-line textarea.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        multiline: bool,
    },

    /// Numeric input. `integer: true` restricts to whole numbers.
    Number {
        #[serde(flatten)]
        meta: FieldMeta,
        /// Restrict to integers only (no decimal point).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        integer: bool,
        /// Minimum accepted value (inclusive).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<serde_json::Number>,
        /// Maximum accepted value (inclusive).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<serde_json::Number>,
        /// UI increment step hint.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        step: Option<serde_json::Number>,
    },

    /// On/off toggle.
    Boolean {
        #[serde(flatten)]
        meta: FieldMeta,
    },

    /// Dropdown / multi-select. Options come from `source`.
    Select {
        #[serde(flatten)]
        meta: FieldMeta,
        /// Where options come from.
        #[serde(flatten)]
        source: OptionSource,
        /// Allow selecting multiple values.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        multiple: bool,
        /// Allow values not present in the option list.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        allow_custom: bool,
        /// Enable type-ahead search/filter in the dropdown.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        searchable: bool,
    },

    /// Syntax-highlighted code editor.
    Code {
        #[serde(flatten)]
        meta: FieldMeta,
        /// Language hint for the editor: `"json"`, `"javascript"`, `"sql"`, etc.
        pub language: String,
    },

    /// Color picker. Value is a CSS hex string (`"#rrggbb"`).
    Color {
        #[serde(flatten)]
        meta: FieldMeta,
    },

    /// Date picker. Value is an ISO 8601 date string (`"YYYY-MM-DD"`).
    Date {
        #[serde(flatten)]
        meta: FieldMeta,
    },

    /// Date-time picker. Value is an ISO 8601 datetime string.
    DateTime {
        #[serde(flatten)]
        meta: FieldMeta,
    },

    /// Time picker. Value is an ISO 8601 time string (`"HH:MM:SS"`).
    Time {
        #[serde(flatten)]
        meta: FieldMeta,
    },

    /// File upload. Value is the uploaded file reference.
    File {
        #[serde(flatten)]
        meta: FieldMeta,
        /// MIME type filter: `"image/*"`, `"application/pdf"`, etc.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        accept: Option<String>,
        /// Maximum file size in bytes.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_size: Option<u64>,
        /// Allow selecting multiple files.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        multiple: bool,
    },

    /// Hidden field with a stored value but no visible editor.
    Hidden {
        #[serde(flatten)]
        meta: FieldMeta,
    },

    // ── Compound fields ──────────────────────────────────────────────────────
    /// Nested object with a fixed set of child fields.
    Object {
        #[serde(flatten)]
        meta: FieldMeta,
        /// Child field definitions.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        fields: Vec<Field>,
    },

    /// Ordered, repeated items using a single item template.
    List {
        #[serde(flatten)]
        meta: FieldMeta,
        /// Template for each list item.
        item: Box<Field>,
        /// Minimum number of items required.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min_items: Option<u32>,
        /// Maximum number of items allowed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_items: Option<u32>,
    },

    /// Discriminated union: user selects a variant, each has its own content field.
    ///
    /// Value shape: `{ "mode": "<variant_key>", "value": <content_value> }`.
    Mode {
        #[serde(flatten)]
        meta: FieldMeta,
        /// Ordered variant definitions.
        variants: Vec<ModeVariant>,
        /// Key of the variant selected by default.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default_variant: Option<String>,
    },

    // ── Dynamic fields (RFC 0004) ────────────────────────────────────────────
    /// A record whose field set is defined by a runtime provider.
    ///
    /// The provider returns `Vec<DynamicFieldSpec>` through the shared
    /// `DynamicProviderEnvelope`. The user fills values for each returned field.
    /// Value shape: `{ "<field_id>": <value>, ... }`.
    DynamicRecord {
        #[serde(flatten)]
        meta: FieldMeta,
        /// Provider key registered in the runtime registry.
        provider: String,
        /// Re-fetch fields when these sibling field ids change.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        depends_on: Vec<String>,
        /// Which provider fields to show initially.
        #[serde(default)]
        mode: DynamicRecordMode,
        /// Policy for values whose field was removed from the provider response.
        #[serde(default)]
        unknown_field_policy: UnknownFieldPolicy,
    },

    /// Structured visual condition builder for runtime data evaluation.
    ///
    /// Used by flow-control nodes (IF, Filter, Switch).
    /// Value is a `PredicateExpr` tree evaluated against the workflow data payload.
    /// This is distinct from `Condition`, which controls *form field visibility*.
    Predicate {
        #[serde(flatten)]
        meta: FieldMeta,
        /// Restrict available operators. `None` = all operators available.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        operators: Option<Vec<PredicateOp>>,
        /// Allow nested AND/OR groups.
        #[serde(default = "default_true")]
        allow_groups: bool,
        /// Maximum nesting depth. Default: 3.
        #[serde(default = "default_depth")]
        max_depth: u8,
    },
}

fn default_true() -> bool { true }
fn default_depth() -> u8 { 3 }
```

---

### `FieldMeta`

Shared across all field types. Flattened into every `Field` variant, so all these
properties appear at the same JSON level as `"type"`.

```rust
/// Metadata common to every field type.
///
/// Flattened into every `Field` variant: all properties appear at the same
/// JSON level as the `"type"` discriminator.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldMeta {
    /// Unique identifier within the schema. Used as the key in value payloads.
    pub id: String,

    /// Human-readable label shown next to the widget.
    pub label: String,

    /// Extended help text shown as a tooltip or below-field hint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Grey placeholder text inside the empty widget.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,

    /// Default value encoded as a JSON value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,

    /// The field is always required (static).
    /// For conditional: use `required_when`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub required: bool,

    /// Mask the value in UI, redact in logs, API read responses, and telemetry.
    /// Values of secret fields must never appear in `FieldError.message`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub secret: bool,

    /// This field accepts expression-interpolated values
    /// (`{{ $json.field }}`, `{{ $env.VAR }}`).
    /// Frontend renders an expression editor instead of a plain input.
    /// Evaluated through `nebula-expression` under runtime policy.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub expression: bool,

    /// Declarative validation rules beyond type constraints.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<Rule>,

    /// Show this field only when the condition evaluates to `true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_when: Option<Condition>,

    /// This field is required only when the condition evaluates to `true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_when: Option<Condition>,

    /// Render this field as read-only when the condition evaluates to `true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled_when: Option<Condition>,
}
```

---

### `Condition`

Deterministic form-state conditions. Evaluated on both Rust backend and JS/TS frontend.
Used for `visible_when`, `required_when`, `disabled_when` on fields and `visible_when`
on UI elements. **Not** evaluated through `nebula-expression`.

```rust
/// A deterministic condition over the current form field values.
///
/// Evaluable without the expression engine on both Rust and JS/TS.
/// References other fields by `id`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Condition {
    // ── Value comparisons ────────────────────────────────────────────────────
    /// `field == value`
    Eq { field: String, value: serde_json::Value },
    /// `field != value`
    Ne { field: String, value: serde_json::Value },
    /// `field > value` (numeric)
    Gt { field: String, value: serde_json::Number },
    /// `field < value` (numeric)
    Lt { field: String, value: serde_json::Number },
    /// `field >= value` (numeric)
    Gte { field: String, value: serde_json::Number },
    /// `field <= value` (numeric)
    Lte { field: String, value: serde_json::Number },
    /// `field ∈ values`
    In { field: String, values: Vec<serde_json::Value> },
    /// Field value contains substring (string) or element (array).
    Contains { field: String, value: serde_json::Value },
    /// Field value matches regex pattern.
    Matches { field: String, pattern: String },

    // ── Presence checks ──────────────────────────────────────────────────────
    /// Field has a non-null, non-empty value.
    Set { field: String },
    /// Field is `null`, empty string `""`, or empty array `[]`.
    Empty { field: String },

    // ── Boolean checks ───────────────────────────────────────────────────────
    /// `field == true`
    IsTrue { field: String },
    /// `field == false`
    IsFalse { field: String },

    // ── Combinators ──────────────────────────────────────────────────────────
    /// All inner conditions must be true.
    All { conditions: Vec<Condition> },
    /// At least one inner condition must be true.
    Any { conditions: Vec<Condition> },
    /// Negates the inner condition.
    Not { condition: Box<Condition> },
}

impl Condition {
    pub fn eq(field: impl Into<String>, value: impl Into<serde_json::Value>) -> Self {
        Self::Eq { field: field.into(), value: value.into() }
    }
    pub fn ne(field: impl Into<String>, value: impl Into<serde_json::Value>) -> Self {
        Self::Ne { field: field.into(), value: value.into() }
    }
    pub fn gt(field: impl Into<String>, value: impl Into<serde_json::Number>) -> Self {
        Self::Gt { field: field.into(), value: value.into() }
    }
    pub fn lt(field: impl Into<String>, value: impl Into<serde_json::Number>) -> Self {
        Self::Lt { field: field.into(), value: value.into() }
    }
    pub fn gte(field: impl Into<String>, value: impl Into<serde_json::Number>) -> Self {
        Self::Gte { field: field.into(), value: value.into() }
    }
    pub fn lte(field: impl Into<String>, value: impl Into<serde_json::Number>) -> Self {
        Self::Lte { field: field.into(), value: value.into() }
    }
    pub fn in_list(field: impl Into<String>, values: Vec<serde_json::Value>) -> Self {
        Self::In { field: field.into(), values }
    }
    pub fn is_true(field: impl Into<String>) -> Self { Self::IsTrue { field: field.into() } }
    pub fn is_false(field: impl Into<String>) -> Self { Self::IsFalse { field: field.into() } }
    pub fn is_set(field: impl Into<String>) -> Self { Self::Set { field: field.into() } }
    pub fn is_empty(field: impl Into<String>) -> Self { Self::Empty { field: field.into() } }
    pub fn matches(field: impl Into<String>, pattern: impl Into<String>) -> Self {
        Self::Matches { field: field.into(), pattern: pattern.into() }
    }
    pub fn all(conditions: Vec<Self>) -> Self { Self::All { conditions } }
    pub fn any(conditions: Vec<Self>) -> Self { Self::Any { conditions } }
    pub fn not(condition: Self) -> Self { Self::Not { condition: Box::new(condition) } }
}
```

**Condition evaluator — shared algorithm (Rust and JS/TS):**

This small evaluator must produce identical results in both languages.
It **must not** call `nebula-expression`.

```rust
/// Evaluate a condition against the current form values.
///
/// `values` is a flat map of `field_id → serde_json::Value`.
/// Returns `true` if the condition is satisfied.
pub fn evaluate_condition(cond: &Condition, values: &serde_json::Map<String, serde_json::Value>) -> bool {
    match cond {
        Condition::Eq { field, value } => values.get(field).map_or(false, |v| v == value),
        Condition::Ne { field, value } => values.get(field).map_or(true,  |v| v != value),
        Condition::Gt { field, value } => cmp_number(values.get(field), value, |a, b| a > b),
        Condition::Lt { field, value } => cmp_number(values.get(field), value, |a, b| a < b),
        Condition::Gte { field, value } => cmp_number(values.get(field), value, |a, b| a >= b),
        Condition::Lte { field, value } => cmp_number(values.get(field), value, |a, b| a <= b),
        Condition::In { field, values: members } =>
            values.get(field).map_or(false, |v| members.contains(v)),
        Condition::Contains { field, value } => {
            match values.get(field) {
                Some(serde_json::Value::String(s)) =>
                    value.as_str().map_or(false, |v| s.contains(v)),
                Some(serde_json::Value::Array(arr)) => arr.contains(value),
                _ => false,
            }
        }
        Condition::Matches { field, pattern } =>
            values.get(field)
                  .and_then(|v| v.as_str())
                  .and_then(|s| regex::Regex::new(pattern).ok().map(|re| re.is_match(s)))
                  .unwrap_or(false),
        Condition::Set { field } =>
            matches!(values.get(field), Some(v) if !v.is_null()),
        Condition::Empty { field } =>
            values.get(field).map_or(true, |v| match v {
                serde_json::Value::Null => true,
                serde_json::Value::String(s) => s.is_empty(),
                serde_json::Value::Array(a) => a.is_empty(),
                _ => false,
            }),
        Condition::IsTrue  { field } => values.get(field) == Some(&serde_json::Value::Bool(true)),
        Condition::IsFalse { field } => values.get(field) == Some(&serde_json::Value::Bool(false)),
        Condition::All { conditions } =>
            conditions.iter().all(|c| evaluate_condition(c, values)),
        Condition::Any { conditions } =>
            conditions.iter().any(|c| evaluate_condition(c, values)),
        Condition::Not { condition } => !evaluate_condition(condition, values),
    }
}
```

---

### `Rule`

Declarative validation rules. Evaluated on both Rust (server) and JS/TS (client).
`Rule::Custom.expression` is the only variant that invokes `nebula-expression`.

```rust
/// Declarative validation rule attached to a field.
///
/// All variants except `Custom` are evaluated by a built-in validator.
/// `Custom.expression` is delegated to `nebula-expression` under the runtime's
/// `ExpressionPolicy`. If the policy is `Skip`, the rule is silently bypassed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "rule", rename_all = "snake_case")]
pub enum Rule {
    /// Minimum string length (in Unicode scalar values).
    MinLength {
        min: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    /// Maximum string length.
    MaxLength {
        max: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    /// Regex pattern (ECMA 262 / Rust `regex` crate; must compile on both sides).
    Pattern {
        pattern: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    /// Minimum numeric value (inclusive).
    Min {
        min: serde_json::Number,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    /// Maximum numeric value (inclusive).
    Max {
        max: serde_json::Number,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    /// Value must be one of the listed values.
    OneOf {
        values: Vec<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    /// All items in a `List<Object>` must have a unique value at `path`.
    ///
    /// Error code: `duplicate_value`. Error path: `<list_id>.<index>.<path>`.
    UniqueBy {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    /// Expression-backed custom rule evaluated by `nebula-expression`.
    ///
    /// The expression receives the field value as `$value` and must evaluate to
    /// a boolean. A `false` result fails validation with `message`.
    Custom {
        expression: String,
        message: String,
    },
}
```

---

### `UiElement`

Non-value elements. Never appear in the runtime value payload.

```rust
/// A non-value visual element in the schema.
///
/// Rendered in declaration order, interleaved with field groups.
/// Never contributes to the value payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UiElement {
    /// Informational banner (info, warning, or error).
    Notice {
        severity: Severity,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        visible_when: Option<Condition>,
    },
    /// Runtime-triggered action button (test connection, refresh schema, etc.).
    Button {
        label: String,
        /// Action key dispatched to the runtime handler. Convention: `"verb.noun"`.
        action: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        enabled_when: Option<Condition>,
    },
}

/// Severity level for `UiElement::Notice`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Error,
}
```

---

### `Group`

Visual grouping for layout. References field `id`s; does not duplicate field definitions.

```rust
/// A named group of fields for layout purposes.
///
/// Field order within a group follows the `fields` array.
/// Fields not referenced by any group are rendered ungrouped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    /// Group heading shown in the UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Ordered list of field `id`s in this group.
    pub fields: Vec<String>,
    /// Start in collapsed state; the user can expand.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub collapsed: bool,
}
```

---

### `ModeVariant`

Each variant of a `Field::Mode` contains exactly one content field of any type.

```rust
/// A single variant in a discriminated-union `Mode` field.
///
/// Value shape for a Mode: `{ "mode": "<key>", "value": <content_value> }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModeVariant {
    /// Stable key used in the value payload.
    pub key: String,
    /// Display label shown in the variant selector.
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Content field for this variant. Any `Field` type is valid including `Object`.
    pub content: Box<Field>,
}
```

---

### `OptionSource`

```rust
/// Where select options originate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum OptionSource {
    /// Options embedded directly in the schema JSON.
    Static { options: Vec<SelectOption> },
    /// Options loaded from a runtime provider. Provider is registered by key.
    Dynamic {
        provider: String,
        /// Re-fetch when these sibling field ids change. Part of the cache key.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        depends_on: Vec<String>,
    },
}

/// A single selectable option.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectOption {
    pub value: serde_json::Value,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
}
```

---

### `DynamicRecord` support types (RFC 0004)

```rust
/// Controls which fields from the provider are shown initially.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DynamicRecordMode {
    /// Show all returned fields.
    #[default]
    All,
    /// Show only provider-marked required fields; user can add optional ones.
    RequiredOnly,
}

/// Policy for saved values whose provider field no longer exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnknownFieldPolicy {
    /// Keep the value; emit a validation warning. (Default.)
    #[default]
    WarnKeep,
    /// Remove the value silently on save.
    Strip,
    /// Fail validation with a structured error.
    Error,
}

/// Field specification returned by a `DynamicRecordProvider`.
///
/// Subset of `Field` types that a runtime provider may define.
/// Cannot be recursive (no nested `DynamicRecord`).
/// No provider-returned executable expressions allowed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DynamicFieldSpec {
    Text {
        id: String,
        label: String,
        #[serde(default)]
        required: bool,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        rules: Vec<Rule>,
    },
    Number {
        id: String,
        label: String,
        #[serde(default)]
        required: bool,
        #[serde(default)]
        integer: bool,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        rules: Vec<Rule>,
    },
    Boolean {
        id: String,
        label: String,
        #[serde(default)]
        required: bool,
    },
    Select {
        id: String,
        label: String,
        #[serde(default)]
        required: bool,
        options: Vec<SelectOption>,
        #[serde(default)]
        multiple: bool,
    },
}
```

---

### `Predicate` support types (RFC 0004)

```rust
/// A runtime condition tree evaluated against workflow data (not form state).
///
/// This is the value type for a `Field::Predicate`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PredicateExpr {
    Group(PredicateGroup),
    Rule(PredicateRule),
}

/// A logical group combining child conditions with AND or OR.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PredicateGroup {
    pub combine: PredicateCombinator,
    pub rules: Vec<PredicateExpr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PredicateCombinator {
    And,
    Or,
}

/// A single data comparison rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PredicateRule {
    /// Canonical dot-path into the data payload, e.g. `"order.total"`, `"items[0].name"`.
    pub field: String,
    pub op: PredicateOp,
    /// Comparison value. Absent for `IsSet`, `IsEmpty`, `IsTrue`, `IsFalse`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
}

/// Operators available in a predicate rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PredicateOp {
    // Equality
    Eq, Ne,
    // Numeric
    Gt, Gte, Lt, Lte,
    // String
    Contains, NotContains, StartsWith, EndsWith, Matches,
    // Presence
    IsSet, IsEmpty,
    // Boolean
    IsTrue, IsFalse,
    // Array membership
    InList, NotInList,
}
```

---

## Provider System

### `DynamicProviderEnvelope`

All dynamic providers (options and record fields) share one versioned response envelope.

```rust
/// Shared versioned envelope for all dynamic provider responses.
///
/// Callers must validate `response_version` before reading `items`.
/// `schema_version` is a provider-controlled opaque hint for cache invalidation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DynamicProviderEnvelope<T> {
    /// Contract version of this payload. Starts at `1`.
    pub response_version: u16,
    /// Logical kind of items in this response.
    pub kind: DynamicResponseKind,
    /// Ordered items in this page.
    pub items: Vec<T>,
    /// Opaque cursor for the next page. `None` = no more pages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    /// Provider-controlled version tag for the upstream schema (used for cache busting).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DynamicResponseKind {
    Options,
    Fields,
}
```

### `OptionProvider`

RPITIT avoids `async_trait` boxing for the common case. Implementations that require
`dyn OptionProvider` should wrap in `Arc<dyn OptionProvider + Send + Sync>` with an
async wrapper.

```rust
/// Runtime provider for dynamic select options.
///
/// Registered by key, called when a `Dynamic` select field needs options
/// or when a `depends_on` field changes.
pub trait OptionProvider: Send + Sync {
    fn key(&self) -> &str;

    fn resolve(
        &self,
        request: &ProviderRequest,
    ) -> impl Future<Output = Result<DynamicProviderEnvelope<SelectOption>, ProviderError>> + Send;
}
```

### `DynamicRecordProvider`

Uses `async_trait` because `DynamicRecord` providers are always object-safe: the
plugin host registry stores `Arc<dyn DynamicRecordProvider>` across heterogeneous
plugin types.

```rust
/// Runtime provider for `DynamicRecord` field definitions.
///
/// Returns a list of field specs the user fills values for.
/// `kind` in the envelope must be `DynamicResponseKind::Fields`.
#[async_trait::async_trait]
pub trait DynamicRecordProvider: Send + Sync {
    fn key(&self) -> &str;

    async fn resolve_fields(
        &self,
        request: &ProviderRequest,
    ) -> Result<DynamicProviderEnvelope<DynamicFieldSpec>, ProviderError>;
}
```

### `ProviderRequest`

Shared request context for both provider traits.

```rust
/// Context passed to a provider when resolving options or fields.
pub struct ProviderRequest {
    /// Which field triggered the request.
    pub field_id: String,
    /// All current form values (for dependency resolution).
    pub values: ParameterValues,
    /// Search filter text for searchable dropdowns.
    pub filter: Option<String>,
    /// Pagination cursor for subsequent pages.
    pub cursor: Option<String>,
}
```

### Canonical Provider Keys

| Provider key         | Trait                  | `kind`    | Item value shape                |
|----------------------|------------------------|-----------|---------------------------------|
| `workflow.branches`  | `OptionProvider`       | `options` | stable branch key string        |
| `eventbus.channels`  | `OptionProvider`       | `options` | stable channel key string       |
| `workflow.catalog`   | `OptionProvider`       | `options` | stable workflow id string       |
| `sheets.columns`     | `DynamicRecordProvider`| `fields`  | `DynamicFieldSpec` per column   |
| `airtable.fields`    | `DynamicRecordProvider`| `fields`  | `DynamicFieldSpec` per field    |
| `db.columns`         | `DynamicRecordProvider`| `fields`  | `DynamicFieldSpec` per column   |
| `notion.properties`  | `DynamicRecordProvider`| `fields`  | `DynamicFieldSpec` per property |

**Stale value rules for all providers:**
- Provider unavailable → keep existing values, mark affected field invalid on submit.
- Previously selected value disappears → value persists but triggers `unknown_branch_key` /
  `unknown_channel` validation error.
- `depends_on` field changes → invalidate only dependent caches, not global state.

---

## Validation

### `FieldError`

```rust
/// A structured validation error returned to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldError {
    /// Dot-path to the failing field. Examples: `"url"`, `"headers.0.key"`,
    /// `"condition.rules.1.value"`.
    pub path: String,
    /// Machine-readable error code understood by frontend:
    /// `"required"`, `"min_length"`, `"pattern"`, `"min"`, `"max"`,
    /// `"one_of"`, `"duplicate_value"`, `"unknown_branch_key"`,
    /// `"unknown_channel"`, `"custom"`.
    pub code: String,
    /// Human-readable message. Must never contain the field value if `secret: true`.
    pub message: String,
}
```

JSON shape sent to the frontend:

```json
{
  "errors": [
    { "path": "url",            "code": "required",  "message": "URL is required" },
    { "path": "url",            "code": "pattern",   "message": "Must be a valid URL" },
    { "path": "headers.0.key",  "code": "required",  "message": "Header name is required" }
  ]
}
```

### Validation ordering contract

Errors are emitted in **field declaration order** (schema traversal order).
Within one field, rule errors follow **rule declaration order**.
Nested field paths use dot-separated indices (`"cases.3.pattern"`).

Hidden fields (where `visible_when` evaluates to `false`) are **skipped** unless
they already hold a value.

Secret field constraint messages must be generic and not expose the value:

```json
// ✅ Correct
{ "path": "token", "code": "min_length", "message": "Token must be at least 10 characters" }

// ❌ Wrong — leaks the value
{ "path": "token", "code": "min_length", "message": "'sk-abc' is too short" }
```

### `validate` signature (v1)

```rust
/// Validate `values` against `schema`.
///
/// Returns errors in field declaration order.
/// Hidden fields are skipped unless they carry a value.
/// Secret field values are never included in error messages.
pub fn validate(schema: &Schema, values: &ParameterValues) -> Vec<FieldError>;
```

---

## Core Field Presets (RFC 0002)

Reusable builders reduce duplication across core action schemas.

```rust
pub mod core_fields {
    use crate::schema::{Field, OptionSource, Rule};

    /// Branch target selection (dynamic, provider `workflow.branches`).
    pub fn branch_target(id: &str) -> Field {
        Field::Select {
            meta: meta(id, "Branch"),
            source: OptionSource::Dynamic {
                provider: "workflow.branches".into(),
                depends_on: vec![],
            },
            multiple: false,
            allow_custom: false,
            searchable: true,
        }
    }

    /// Event-bus channel selection (dynamic, provider `eventbus.channels`).
    pub fn signal_channel(id: &str) -> Field {
        Field::Select {
            meta: meta(id, "Channel"),
            source: OptionSource::Dynamic {
                provider: "eventbus.channels".into(),
                depends_on: vec![],
            },
            multiple: false,
            allow_custom: false,
            searchable: true,
        }
    }

    /// Timeout in milliseconds (integer, 1 ms – 300 s).
    pub fn timeout_ms(id: &str) -> Field {
        Field::Number {
            meta: meta(id, "Timeout (ms)"),
            integer: true,
            min: Some(serde_json::Number::from(1u32)),
            max: Some(serde_json::Number::from(300_000u32)),
            step: None,
        }
    }

    /// Retry policy (mode with `none` / `fixed` variants).
    pub fn retry_policy(id: &str) -> Field {
        use crate::schema::{ModeVariant, FieldMeta};
        Field::Mode {
            meta: meta(id, "Retry Policy"),
            variants: vec![
                ModeVariant {
                    key: "none".into(),
                    label: "No Retry".into(),
                    description: None,
                    content: Box::new(Field::Hidden {
                        meta: FieldMeta { id: "_".into(), label: "_".into(), ..Default::default() },
                    }),
                },
                ModeVariant {
                    key: "fixed".into(),
                    label: "Fixed".into(),
                    description: None,
                    content: Box::new(Field::Object {
                        meta: FieldMeta { id: "cfg".into(), label: "Config".into(), ..Default::default() },
                        fields: vec![
                            Field::Number {
                                meta: FieldMeta {
                                    id: "max_attempts".into(),
                                    label: "Max Attempts".into(),
                                    required: true,
                                    default: Some(serde_json::json!(3)),
                                    ..Default::default()
                                },
                                integer: true,
                                min: Some(serde_json::Number::from(1u32)),
                                max: Some(serde_json::Number::from(100u32)),
                                step: None,
                            },
                            Field::Number {
                                meta: FieldMeta {
                                    id: "delay_ms".into(),
                                    label: "Delay (ms)".into(),
                                    required: true,
                                    default: Some(serde_json::json!(1000)),
                                    ..Default::default()
                                },
                                integer: true,
                                min: Some(serde_json::Number::from(0u32)),
                                max: None,
                                step: None,
                            },
                        ],
                    }),
                },
            ],
            default_variant: Some("none".into()),
        }
    }
}
```

---

## Internal Architecture (non-normative for wire contract)

The internal `nebula-parameter` implementation follows a 4-layer model.
These layers are **not** exposed in the v2 public API surface; they are
implementation details behind `Schema`, `ParameterValues`, and `validate()`.

```
┌──────────────────────────────────────────────────────┐
│  1. Schema Layer                                     │
│     Schema, Field, FieldMeta, Condition, Rule        │
│     Canonical v2 authoring surface.                  │
└─────────────────────────┬────────────────────────────┘
                          │  Schema::compile()
┌─────────────────────────▼────────────────────────────┐
│  2. Validation Engine (nebula-validator)             │
│     ValidationPlan, CompiledField, ValidatorSet      │
│     Pre-compiles regexes; topological traversal.     │
└─────────────────────────┬────────────────────────────┘
                          │  ValidationPlan::validate()
┌─────────────────────────▼────────────────────────────┐
│  3. Runtime Values                                   │
│     ParameterValues   — HashMap<String, Value>       │
│     ValidatedValues   — schema-bound typed view      │
└─────────────────────────┬────────────────────────────┘
                          │  nebula-expression (policy-gated)
┌─────────────────────────▼────────────────────────────┐
│  4. Expression integration (nebula-expression)       │
│     Rule::Custom, expression: true fields            │
│     Evaluated only where ExpressionPolicy permits.  │
└──────────────────────────────────────────────────────┘
```

### Cross-crate responsibilities

| Crate              | Responsibility                                                              |
|--------------------|-----------------------------------------------------------------------------|
| `nebula-parameter` | Schema authoring, Condition evaluation, FieldError, provider traits         |
| `nebula-validator` | `ValidationPlan` execution, `Rule` evaluation, deterministic error ordering |
| `nebula-expression`| `Rule::Custom` eval, `expression: true` field value interpolation           |
| `nebula-action`    | Consumes compiled parameter contracts for action configuration              |
| `nebula-credential`| Consumes secret-annotated fields for encryption and vault integration       |
| `nebula-runtime`   | Orchestrates: resolve expressions → dynamic options → validate → execute    |

---

## Builder API

Full builder API for ergonomic schema authoring. Schemas are typically declared once
in static descriptors and never mutated; the consuming builder pattern fits this idiom.

### `Schema` builder

```rust
impl Schema {
    pub fn new() -> Self;
    /// Add a field.
    #[must_use] pub fn field(self, field: Field) -> Self;
    /// Add a UI-only element.
    #[must_use] pub fn ui(self, element: UiElement) -> Self;
    /// Add a named, ungrouped field group.
    #[must_use] pub fn group(self, label: &str, fields: &[&str]) -> Self;
    /// Add a named collapsed field group.
    #[must_use] pub fn group_collapsed(self, label: &str, fields: &[&str]) -> Self;
    /// Add a schema-level warning notice.
    #[must_use] pub fn notice(self, severity: Severity, text: &str) -> Self;
}
```

### `Field` constructors

```rust
impl Field {
    // ── Scalar ──────────────────────────────────────────────────────────
    pub fn text(id: &str) -> FieldBuilder;
    pub fn number(id: &str) -> FieldBuilder;        // decimal
    pub fn integer(id: &str) -> FieldBuilder;       // integer shortcut (integer: true)
    pub fn boolean(id: &str) -> FieldBuilder;
    pub fn color(id: &str) -> FieldBuilder;
    pub fn date(id: &str) -> FieldBuilder;
    pub fn datetime(id: &str) -> FieldBuilder;
    pub fn time(id: &str) -> FieldBuilder;
    pub fn hidden(id: &str) -> FieldBuilder;
    pub fn code(id: &str, language: &str) -> FieldBuilder;
    pub fn file(id: &str) -> FieldBuilder;
    // ── Select ──────────────────────────────────────────────────────────
    pub fn select(id: &str) -> FieldBuilder;
    pub fn multi_select(id: &str) -> FieldBuilder;  // multiple: true shortcut
    // ── Compound ────────────────────────────────────────────────────────
    pub fn object(id: &str) -> FieldBuilder;
    pub fn list(id: &str, item: Field) -> FieldBuilder;
    pub fn mode(id: &str) -> FieldBuilder;
    // ── Dynamic (RFC 0004) ───────────────────────────────────────────────
    pub fn dynamic_record(id: &str) -> FieldBuilder;
    pub fn predicate(id: &str) -> FieldBuilder;
}
```

### `FieldBuilder` methods

```rust
impl FieldBuilder {
    // ── Common ──────────────────────────────────────────────────────────
    pub fn label(self, label: &str) -> Self;
    pub fn description(self, desc: &str) -> Self;
    pub fn placeholder(self, text: &str) -> Self;
    pub fn default(self, value: impl Into<serde_json::Value>) -> Self;
    pub fn required(self) -> Self;
    pub fn secret(self) -> Self;
    pub fn expression(self) -> Self;
    pub fn rule(self, rule: Rule) -> Self;
    pub fn visible_when(self, cond: Condition) -> Self;
    pub fn required_when(self, cond: Condition) -> Self;
    pub fn disabled_when(self, cond: Condition) -> Self;
    // ── Text ─────────────────────────────────────────────────────────────
    pub fn multiline(self) -> Self;
    // ── Number ───────────────────────────────────────────────────────────
    pub fn range(self, min: impl Into<serde_json::Number>,
                       max: impl Into<serde_json::Number>) -> Self;
    pub fn step(self, step: impl Into<serde_json::Number>) -> Self;
    // ── Select ───────────────────────────────────────────────────────────
    pub fn option(self, value: impl Into<serde_json::Value>, label: &str) -> Self;
    pub fn options(self, options: Vec<SelectOption>) -> Self;
    pub fn dynamic(self, provider: &str) -> Self;
    pub fn depends_on(self, fields: &[&str]) -> Self;
    pub fn searchable(self) -> Self;
    pub fn allow_custom(self) -> Self;
    pub fn multiple(self) -> Self;
    // ── Object ───────────────────────────────────────────────────────────
    pub fn fields(self, fields: Vec<Field>) -> Self;
    // ── List ─────────────────────────────────────────────────────────────
    pub fn min_items(self, n: u32) -> Self;
    pub fn max_items(self, n: u32) -> Self;
    // ── File ─────────────────────────────────────────────────────────────
    pub fn accept(self, mime: &str) -> Self;
    pub fn max_size(self, bytes: u64) -> Self;
    // ── Mode ─────────────────────────────────────────────────────────────
    pub fn variant(self, key: &str, label: &str, content: Field) -> Self;
    pub fn default_variant(self, key: &str) -> Self;
    // ── DynamicRecord (RFC 0004) ──────────────────────────────────────────
    pub fn provider(self, key: &str) -> Self;
    pub fn mode(self, mode: DynamicRecordMode) -> Self;
    pub fn unknown_field_policy(self, policy: UnknownFieldPolicy) -> Self;
    // ── Predicate (RFC 0004) ─────────────────────────────────────────────
    pub fn operators(self, ops: &[PredicateOp]) -> Self;
    pub fn allow_groups(self, allow: bool) -> Self;
    pub fn max_depth(self, depth: u8) -> Self;
    // ── Build ─────────────────────────────────────────────────────────────
    /// Consumes the builder and returns the `Field`. Cannot fail.
    pub fn build(self) -> Field;
}
```

---

## Complete Example

HTTP node schema using every RFC decision:

```rust
use nebula_parameter::schema::{
    Condition, Field, Group, Rule, Schema, Severity, UiElement,
};

let schema = Schema::new()
    // ── Authentication ────────────────────────────────────────────────
    .field(
        Field::mode("auth")
            .label("Authentication")
            .variant("none",   "None",
                Field::hidden("_none").build())
            .variant("bearer", "Bearer Token",
                Field::text("token").label("Token").secret().required().build())
            .variant("basic",  "Basic Auth",
                Field::object("credentials")
                    .label("Credentials")
                    .fields(vec![
                        Field::text("username").label("Username").required().build(),
                        Field::text("password").label("Password").secret().required().build(),
                    ])
                    .build())
            .default_variant("none")
            .build()
    )
    // ── Request ───────────────────────────────────────────────────────
    .field(
        Field::select("method")
            .label("HTTP Method").required().default("GET")
            .option("GET",    "GET")
            .option("POST",   "POST")
            .option("PUT",    "PUT")
            .option("DELETE", "DELETE")
            .option("PATCH",  "PATCH")
            .build()
    )
    .field(
        Field::text("url")
            .label("URL")
            .placeholder("https://api.example.com/endpoint")
            .required()
            .rule(Rule::Pattern {
                pattern: "^https?://".into(),
                message:  Some("Must be a valid URL".into()),
            })
            .build()
    )
    .field(
        Field::list("headers",
            Field::object("_item")
                .label("Header")
                .fields(vec![
                    Field::text("key").label("Name").required()
                        .placeholder("Header name").build(),
                    Field::text("value").label("Value")
                        .placeholder("Header value").build(),
                ])
                .build()
        )
        .label("Headers")
        .build()
    )
    .field(
        Field::code("body", "json")
            .label("Request Body")
            .visible_when(Condition::in_list(
                "method",
                vec!["POST".into(), "PUT".into(), "PATCH".into()],
            ))
            .build()
    )
    // ── Advanced ──────────────────────────────────────────────────────
    .field(
        Field::integer("timeout")
            .label("Timeout (ms)").default(30_000u64)
            .range(100u32, 120_000u32)
            .build()
    )
    .field(
        Field::boolean("retry_enabled").label("Enable Retry").default(false).build()
    )
    .field(
        Field::integer("max_retries")
            .label("Max Retries").default(3u32).range(1u32, 10u32)
            .visible_when(Condition::is_true("retry_enabled"))
            .build()
    )
    // ── UI ────────────────────────────────────────────────────────────
    .notice(
        Severity::Warning,
        "API may return 429 errors. Enable retry for stability.",
    )
    // ── Groups ────────────────────────────────────────────────────────
    .group("Authentication", &["auth"])
    .group("Request",        &["method", "url", "headers", "body"])
    .group_collapsed("Advanced", &["timeout", "retry_enabled", "max_retries"]);
```

### IF node (RFC 0004 `Predicate`)

```rust
let schema = Schema::new()
    .field(
        Field::predicate("condition")
            .label("Condition")
            .required()
            .build()
    );
```

Value routed to `true` or `false` branch based on predicate evaluation.

### Switch node (RFC 0004 `Predicate` + RFC 0002 uniqueness)

```rust
let schema = Schema::new()
    .field(
        Field::list("cases",
            Field::object("_case")
                .label("Case")
                .fields(vec![
                    Field::predicate("condition")
                        .label("When")
                        .allow_groups(false)  // single-rule switch cases
                        .required()
                        .build(),
                    Field::select("branch_key")
                        .label("Go to Branch")
                        .dynamic("workflow.branches")
                        .searchable()
                        .required()
                        .build(),
                ])
                .build()
        )
        .label("Cases")
        .rule(Rule::UniqueBy { path: "branch_key".into(), message: None })
        .build()
    )
    .field(
        Field::select("fallback")
            .label("Default Branch (fallback)")
            .dynamic("workflow.branches")
            .searchable()
            .build()
    );
```

### Google Sheets — Add Row (RFC 0004 `DynamicRecord`)

```rust
let schema = Schema::new()
    .field(
        Field::select("spreadsheet_id")
            .label("Spreadsheet").required()
            .dynamic("sheets.spreadsheets")
            .searchable()
            .build()
    )
    .field(
        Field::select("sheet_id")
            .label("Sheet").required()
            .dynamic("sheets.sheets")
            .depends_on(&["spreadsheet_id"])
            .searchable()
            .build()
    )
    .field(
        Field::dynamic_record("row_data")
            .label("Row Data")
            .provider("sheets.columns")
            .depends_on(&["spreadsheet_id", "sheet_id"])
            .mode(DynamicRecordMode::RequiredOnly)
            .build()
    );
```

---

## Migration

### Phase 1 — Cutover Preparation (v0.9.x)

1. Add all missing `Field` variants: `Color`, `Date`, `DateTime`, `Time`, `File`,
   `DynamicRecord`, `Predicate`.
2. Complete `Condition`: add `Gte`, `Lte`, `Set`, `Empty`, `Contains`, `Matches`.
3. Complete `Rule`: add `MinLength`, `MaxLength`, `OneOf`, `UniqueBy`.
4. Complete `Field::Select`: add `allow_custom`, `searchable`.
5. Complete `Field::Number`: add `min`, `max`, `step`.
6. Complete `Field::List`: add `min_items`, `max_items`.
7. Add `DynamicRecordMode`, `UnknownFieldPolicy`, `DynamicFieldSpec`, `PredicateExpr`
   with all sub-types.
8. Move `id` and `label` into `FieldMeta`; update all `Field` variants to use
   `#[serde(flatten)] meta: FieldMeta`.
9. Implement `core_fields` preset module.
10. Implement `evaluate_condition(cond, values)` as a pure function.
11. Implement `validate(schema, values) -> Vec<FieldError>`.
12. Implement `import_v1_json` with `ConversionWarning` list.
13. Deprecate `ParameterDef`, `ParameterCollection`, legacy display types.

### Phase 2 — Clean Cut (v1.0.0, breaking)

1. Remove `ParameterDef`, `ParameterCollection` from runtime API surface.
2. Remove `subtype`, `kind`, `display` from `prelude` (keep as migration-only).
3. Stabilize `Schema` serde representation. Enforce SemVer.
4. Make `validation::validate` the single server-side entry point.
5. Publish migration guide and `nebula-parameter-migrate` CLI tool.

---

## Acceptance Criteria

| Category       | Criterion                                                                        |
|----------------|----------------------------------------------------------------------------------|
| Correctness    | Zero silent subtype-shortcut downgrades in v1 → v2 import                        |
| Correctness    | Deterministic error order (stable between runs; topological + declaration order)  |
| Correctness    | No precision loss for integer-only domains (ports, timestamps, indices)           |
| Correctness    | `Mode` validation checks the active variant's nested fields                       |
| Correctness    | Secret field errors never include the field value                                 |
| Correctness    | `DynamicRecord` provider cycles fail schema compilation                           |
| Correctness    | `Predicate.max_depth` is validated at schema compile time                         |
| Performance    | Regex compilation is cached per `ValidationPlan`; no per-call recompilation       |
| Performance    | Condition evaluation is O(n) in condition tree size; no heap allocation per call  |
| Compatibility  | All existing v1 schemas import with explicit warnings, zero silent coercion       |
| Compatibility  | Existing `ParameterDef` schemas continue to work in v0.9.x                       |
| DX             | Typed builder covers all 16 field variants without runtime panics               |
| DX             | `core_fields` presets produce correct JSON without additional configuration       |
| DX             | `FieldError.path` maps directly to a field widget in the frontend form            |

---

## Superseded Documents

The following documents are replaced by this RFC and should not be referenced
for new implementation work:

| File                                   | Reason superseded                              |
|----------------------------------------|------------------------------------------------|
| `0001-parameter-schema-v2.md`          | Canonical wire contract → merged here          |
| `0001-parameter-api-v2.md`             | Working paper / impl guide → merged here       |
| `0001-v2-universality-playground.md`   | Exploratory playground → reconciled here       |
| `0002-core-flow-schema-extensions.md`  | Merged: `core_fields`, `Rule::UniqueBy`        |
| `0004-new-field-types.md`              | Merged: `DynamicRecord`, `Predicate`           |

`0003-cross-platform-core-nodes-gap-analysis.md` remains as an informational reference.
No design changes arise from it beyond what is already in this RFC.

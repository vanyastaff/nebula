# RFC 0001: Parameter Schema v2

**Status:** Draft  
**Created:** 2026-03-08  
**Target:** `nebula-parameter` v1.0  

---

## Summary

A new parameter schema that is the **single source of truth** for:

1. **Form rendering** — frontends auto-generate UIs from the schema JSON
2. **Validation** — server and client validate against the same rules
3. **Conditional logic** — show/hide/require/disable fields based on other field values
4. **HTTP transport** — the Rust types serialize directly to the JSON API contract

**Design rule:** the Rust struct IS the JSON shape. No translation layer, no adapters.

---

## JSON Contract

This is what a frontend receives over HTTP. Every design decision optimizes for readability
and ease of form rendering.

```json
{
  "fields": [
    {
      "id": "auth",
      "type": "mode",
      "label": "Authentication",
      "default_variant": "none",
      "variants": [
        {
          "key": "none",
          "label": "None",
          "content": { "id": "_none", "type": "hidden", "default": true }
        },
        {
          "key": "bearer",
          "label": "Bearer Token",
          "content": { "id": "token", "type": "text", "label": "Token", "secret": true, "required": true }
        },
        {
          "key": "basic",
          "label": "Basic Auth",
          "content": {
            "id": "credentials",
            "type": "object",
            "fields": [
              { "id": "username", "type": "text", "label": "Username", "required": true },
              { "id": "password", "type": "text", "label": "Password", "secret": true, "required": true }
            ]
          }
        }
      ]
    },
    {
      "id": "method",
      "type": "select",
      "label": "HTTP Method",
      "required": true,
      "default": "GET",
      "options": [
        { "value": "GET", "label": "GET" },
        { "value": "POST", "label": "POST" },
        { "value": "PUT", "label": "PUT" },
        { "value": "DELETE", "label": "DELETE" },
        { "value": "PATCH", "label": "PATCH" }
      ]
    },
    {
      "id": "url",
      "type": "text",
      "label": "URL",
      "placeholder": "https://api.example.com/endpoint",
      "required": true,
      "rules": [
        { "rule": "pattern", "pattern": "^https?://", "message": "Must be a valid URL" }
      ]
    },
    {
      "id": "headers",
      "type": "list",
      "label": "Headers",
      "item": {
        "id": "_item",
        "type": "object",
        "label": "Header",
        "fields": [
          { "id": "key", "type": "text", "label": "Name", "required": true, "placeholder": "Header name" },
          { "id": "value", "type": "text", "label": "Value", "placeholder": "Header value" }
        ]
      }
    },
    {
      "id": "body",
      "type": "code",
      "label": "Request Body",
      "language": "json",
      "visible_when": {
        "op": "in",
        "field": "method",
        "values": ["POST", "PUT", "PATCH"]
      }
    },
    {
      "id": "timeout",
      "type": "number",
      "label": "Timeout (ms)",
      "default": 30000,
      "integer": true,
      "min": 100,
      "max": 120000
    },
    {
      "id": "retry_enabled",
      "type": "boolean",
      "label": "Enable Retry",
      "default": false
    },
    {
      "id": "max_retries",
      "type": "number",
      "label": "Max Retries",
      "integer": true,
      "min": 1,
      "max": 10,
      "default": 3,
      "visible_when": { "op": "eq", "field": "retry_enabled", "value": true }
    }
  ],
  "ui": [
    {
      "kind": "notice",
      "severity": "warning",
      "text": "API may return 429 errors. Enable retry for stability.",
      "visible_when": { "op": "eq", "field": "retry_enabled", "value": false }
    }
  ],
  "groups": [
    { "label": "Authentication", "fields": ["auth"] },
    { "label": "Request", "fields": ["method", "url", "headers", "body"] },
    { "label": "Advanced", "fields": ["timeout", "retry_enabled", "max_retries"], "collapsed": true }
  ]
}
```

### Frontend rendering algorithm

```
for group in schema.groups:
    render group header (collapsible if group.collapsed)
    for field_id in group.fields:
        field = schema.fields.find(id == field_id)
        if field.visible_when and not evaluate(field.visible_when, current_values):
            skip
        render widget by field.type
        if field.required or evaluate(field.required_when, current_values):
            mark as required
        if field.disabled_when and evaluate(field.disabled_when, current_values):
            mark as disabled

for ui_element in schema.ui:
    if ui_element.visible_when and not evaluate(ui_element.visible_when, current_values):
        skip
    render based on ui_element.kind
```

---

## Rust Types

### Schema

```rust
/// Complete parameter schema.
///
/// Serializes directly to the JSON API contract. Frontends consume this
/// to render forms, validate input, evaluate visibility conditions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schema {
    /// Ordered list of field definitions.
    pub fields: Vec<Field>,

    /// UI-only elements (notices, buttons). Not part of the value payload.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ui: Vec<UiElement>,

    /// Field grouping for layout. References field ids.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<Group>,
}
```

### Field

Each field is an internally-tagged enum. The `"type"` key acts as the discriminator.
Common metadata is shared via `FieldMeta` flattened into every variant.

```rust
/// A single field in the schema.
///
/// The `type` tag determines the widget and which properties are relevant.
/// Common properties (id, label, conditions, etc.) are in `FieldMeta`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Field {
    Text {
        #[serde(flatten)]
        meta: FieldMeta,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        multiline: bool,
    },
    Number {
        #[serde(flatten)]
        meta: FieldMeta,
        /// true = integer only, false = decimal allowed.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        integer: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        step: Option<f64>,
    },
    Boolean {
        #[serde(flatten)]
        meta: FieldMeta,
    },
    Select {
        #[serde(flatten)]
        meta: FieldMeta,
        #[serde(flatten)]
        source: OptionSource,
        /// true = multiple selections allowed.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        multiple: bool,
        /// Allow values not in the option list.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        allow_custom: bool,
        /// Enable search/filter in the dropdown.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        searchable: bool,
    },
    Object {
        #[serde(flatten)]
        meta: FieldMeta,
        /// Nested field definitions.
        fields: Vec<Field>,
    },
    List {
        #[serde(flatten)]
        meta: FieldMeta,
        /// Schema for each list item.
        item: Box<Field>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min_items: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_items: Option<u32>,
    },
    Mode {
        #[serde(flatten)]
        meta: FieldMeta,
        /// Discriminated union: user picks a variant, each has its own content node.
        /// Value shape: `{ "mode": "variant_key", "value": <content_value> }`
        variants: Vec<ModeVariant>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default_variant: Option<String>,
    },
    Code {
        #[serde(flatten)]
        meta: FieldMeta,
        /// Language for syntax highlighting: "json", "javascript", "sql", "html", etc.
        language: String,
    },
    Color {
        #[serde(flatten)]
        meta: FieldMeta,
    },
    Date {
        #[serde(flatten)]
        meta: FieldMeta,
    },
    DateTime {
        #[serde(flatten)]
        meta: FieldMeta,
    },
    Time {
        #[serde(flatten)]
        meta: FieldMeta,
    },
    Hidden {
        #[serde(flatten)]
        meta: FieldMeta,
    },
    File {
        #[serde(flatten)]
        meta: FieldMeta,
        /// MIME filter: "image/*", "application/pdf", etc.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        accept: Option<String>,
        /// Maximum file size in bytes.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_size: Option<u64>,
        /// Allow selecting multiple files.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        multiple: bool,
    },
}
```

### FieldMeta — shared across all field types

```rust
/// Properties common to every field type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldMeta {
    /// Unique identifier within the schema. Used as the key in form values.
    pub id: String,

    /// Human-readable label shown next to the widget.
    pub label: String,

    /// Extended help text (tooltip, below-field hint).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Grey text inside the empty widget.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,

    /// Default value (must match the field type).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,

    /// Always required (static). For conditional, use `required_when`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub required: bool,

    /// Mask the value in UI, logs, API responses.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub secret: bool,

    /// Field supports expression interpolation (`{{ $json.field }}`, `{{ $env.VAR }}`).
    /// When true, the frontend renders an expression editor instead of a plain input.
    /// Applies to any field type, not just Text.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub expression: bool,

    /// Validation rules beyond type constraints.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<Rule>,

    /// Show this field only when the condition is true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_when: Option<Condition>,

    /// Require this field only when the condition is true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_when: Option<Condition>,

    /// Disable (read-only) this field when the condition is true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled_when: Option<Condition>,
}
```

### Conditions — the show/hide/require engine

Conditions are evaluable on both server (Rust) and client (JS/TS).
They reference other fields by id and compare against values.

```rust
/// A condition that references form field values.
///
/// Evaluable on both Rust backend and JS frontend.
/// Used for `visible_when`, `required_when`, `disabled_when`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Condition {
    // ── Value comparisons ───────────────────────────────
    /// field == value
    Eq { field: String, value: serde_json::Value },
    /// field != value
    Ne { field: String, value: serde_json::Value },
    /// field > value (numeric)
    Gt { field: String, value: f64 },
    /// field < value (numeric)
    Lt { field: String, value: f64 },
    /// field >= value (numeric)
    Gte { field: String, value: f64 },
    /// field <= value (numeric)
    Lte { field: String, value: f64 },
    /// field value is one of the listed values
    In { field: String, values: Vec<serde_json::Value> },
    /// field value contains substring (string) or element (array)
    Contains { field: String, value: serde_json::Value },
    /// field matches regex pattern
    Matches { field: String, pattern: String },

    // ── Presence checks ─────────────────────────────────
    /// field has a non-null value
    Set { field: String },
    /// field is null, empty string, or empty array
    Empty { field: String },

    // ── Boolean field checks ────────────────────────────
    /// field == true
    IsTrue { field: String },
    /// field == false
    IsFalse { field: String },

    // ── Combinators ─────────────────────────────────────
    /// All conditions must be true.
    All { conditions: Vec<Condition> },
    /// At least one condition must be true.
    Any { conditions: Vec<Condition> },
    /// Negate the inner condition.
    Not { condition: Box<Condition> },
}
```

**JSON examples:**

```json
// Simple: show when auth.mode == "bearer"
{ "op": "eq", "field": "auth", "value": { "mode": "bearer" } }

// Composite: show when resource is "user" AND operation is "create"
{
  "op": "all",
  "conditions": [
    { "op": "eq", "field": "resource", "value": "user" },
    { "op": "eq", "field": "operation", "value": "create" }
  ]
}

// Show when method is one of POST, PUT, PATCH
{ "op": "in", "field": "method", "values": ["POST", "PUT", "PATCH"] }

// Show when retry is enabled AND attempts > 0
{
  "op": "all",
  "conditions": [
    { "op": "is_true", "field": "retry_enabled" },
    { "op": "gt", "field": "retry_count", "value": 0 }
  ]
}
```

### Validation Rules

```rust
/// Declarative validation rule. Evaluated by both Rust and JS.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "rule", rename_all = "snake_case")]
pub enum Rule {
    MinLength {
        min: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    MaxLength {
        max: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    Pattern {
        pattern: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    Min {
        min: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    Max {
        max: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    OneOf {
        values: Vec<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    /// Ensure values are unique across list items by object field path.
    ///
    /// Example: `unique_by("pattern")` on `List<Object>` ensures no duplicate
    /// `pattern` values across the list.
    UniqueBy {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    /// Expression-based custom rule (evaluated by expression engine).
    Custom {
        expression: String,
        message: String,
    },
}
```

### Select Options

```rust
/// Where select options come from.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum OptionSource {
    /// Options defined inline in the schema.
    Static {
        options: Vec<SelectOption>,
    },
    /// Options loaded from an async provider at runtime.
    Dynamic {
        /// Provider key (registered in the runtime).
        provider: String,
        /// Re-fetch options when these fields change.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        depends_on: Vec<String>,
    },
}

/// A single option in a select/multi-select field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectOption {
    pub value: serde_json::Value,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
}

/// Discriminated union variant.
///
/// Each variant contains exactly one content node — can be any `Field`:
/// a scalar (Text, Number), a compound (Object), or even a nested Mode.
///
/// Value shape: `{ "mode": "variant_key", "value": <content_value> }`
/// - If content is Text → `{ "mode": "by_id", "value": "12345" }`
/// - If content is Object → `{ "mode": "basic", "value": { "user": "...", "pass": "..." } }`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModeVariant {
    pub key: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The content field for this variant. Any Field type.
    pub content: Box<Field>,
}
```

### Dynamic Option Provider (Rust runtime)

```rust
/// Runtime interface for loading select options.
///
/// Registered by provider key. Called when a dynamic select field
/// needs options or when a `depends_on` field changes.
pub trait OptionProvider: Send + Sync {
    fn key(&self) -> &str;

    fn resolve(
        &self,
        request: &OptionRequest,
    ) -> impl Future<Output = Result<OptionPage, OptionError>> + Send;
}

pub struct OptionRequest {
    /// Which field requested options.
    pub field_id: String,
    /// Current form values (for dependency resolution).
    pub values: ParameterValues,
    /// Search filter text (for searchable dropdowns).
    pub filter: Option<String>,
    /// Pagination cursor.
    pub cursor: Option<String>,
}

pub struct OptionPage {
    pub options: Vec<SelectOption>,
    /// Cursor for the next page. `None` = no more pages.
    pub next_cursor: Option<String>,
}
```

### Canonical Dynamic Providers

The following provider keys are canonical for reusable parameter schemas.

| Provider key | Purpose | Value shape |
|---|---|---|
| `workflow.branches` | Branch target selection | stable branch key string |
| `eventbus.channels` | Event/signal channel selection | stable channel key string |
| `workflow.catalog` | Workflow/subflow selection | stable workflow id string |

All providers return paged `SelectOption` values (`value`, `label`, optional
`description`). Stale selected values may remain visible in UI but must fail
validation with field-specific error codes.

### UI Elements

```rust
/// Non-data visual elements. Never appear in the value payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UiElement {
    /// Informational banner.
    Notice {
        severity: Severity,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        visible_when: Option<Condition>,
    },
    /// Action button (test connection, refresh schema, etc.)
    Button {
        label: String,
        /// Action key handled by the runtime.
        action: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        enabled_when: Option<Condition>,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Error,
}
```

### Groups

```rust
/// Visual grouping of fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Group {
    /// Group heading.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Field ids in this group. Display order follows this list.
    pub fields: Vec<String>,
    /// Start collapsed (user can expand).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub collapsed: bool,
}
```

### Validation Errors

```rust
/// Structured validation error returned by the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldError {
    /// Dot-path to the field: "headers.0.key", "body", "auth.username"
    pub path: String,
    /// Machine-readable code: "required", "min_length", "pattern",
    /// "duplicate_value", "unknown_branch_key", "unknown_channel", etc.
    pub code: String,
    /// Human-readable message.
    pub message: String,
}
```

**JSON shape sent to the frontend:**

```json
{
  "errors": [
    { "path": "url", "code": "required", "message": "URL is required" },
    { "path": "url", "code": "pattern", "message": "Must be a valid URL" },
    { "path": "headers.0.key", "code": "required", "message": "Header name is required" }
  ]
}
```

The frontend matches errors to fields by `path` and renders inline messages.

---

## Rust Builder API

The builder API optimizes for author DX. Schemas are typically defined once
in action/node descriptors and never mutated.

```rust
let schema = Schema::builder()
    // ── Authentication ──────────────────────────────────
    .field(
        Field::mode("auth")
            .label("Authentication")
            .variant("none", "None",
                Field::hidden("_none").default(true).build()
            )
            .variant("bearer", "Bearer Token",
                Field::text("token")
                    .label("Token")
                    .secret()
                    .required()
                    .build()
            )
            .variant("basic", "Basic Auth",
                Field::object("credentials")
                    .fields(vec![
                        Field::text("username").label("Username").required().build(),
                        Field::text("password").label("Password").secret().required().build(),
                    ])
                    .build()
            )
            .default_variant("none")
    )
    // ── Request ─────────────────────────────────────────
    .field(
        Field::select("method")
            .label("HTTP Method")
            .required()
            .default("GET")
            .option("GET", "GET")
            .option("POST", "POST")
            .option("PUT", "PUT")
            .option("DELETE", "DELETE")
            .option("PATCH", "PATCH")
    )
    .field(
        Field::text("url")
            .label("URL")
            .placeholder("https://api.example.com/endpoint")
            .required()
            .rule(Rule::pattern("^https?://", "Must be a valid URL"))
    )
    .field(
        Field::list("headers",
            Field::object("_item")
                .label("Header")
                .fields(vec![
                    Field::text("key").label("Name").required().placeholder("Header name").build(),
                    Field::text("value").label("Value").placeholder("Header value").build(),
                ])
                .build()
        )
        .label("Headers")
    )
    .field(
        Field::code("body", "json")
            .label("Request Body")
            .visible_when(Condition::any_of("method", &["POST", "PUT", "PATCH"]))
    )
    // ── Advanced ────────────────────────────────────────
    .field(
        Field::integer("timeout")
            .label("Timeout (ms)")
            .default(30_000)
            .range(100, 120_000)
    )
    .field(
        Field::boolean("retry_enabled")
            .label("Enable Retry")
            .default(false)
    )
    .field(
        Field::integer("max_retries")
            .label("Max Retries")
            .default(3)
            .range(1, 10)
            .visible_when(Condition::is_true("retry_enabled"))
    )
    // ── UI ──────────────────────────────────────────────
    .notice(
        Severity::Warning,
        "API may return 429 errors. Enable retry for stability.",
    )
    // ── Layout ──────────────────────────────────────────
    .group("Authentication", &["auth"])
    .group("Request", &["method", "url", "headers", "body"])
    .group_collapsed("Advanced", &["timeout", "retry_enabled", "max_retries"])
    .build()?;

// Serialize to JSON for HTTP response:
let json = serde_json::to_string(&schema)?;
```

### Builder convenience methods

```rust
impl Field {
    // ── Constructors (set type + id in one call) ──────────────
    pub fn text(id: &str) -> FieldBuilder;
    pub fn number(id: &str) -> FieldBuilder;       // decimal
    pub fn integer(id: &str) -> FieldBuilder;      // integer shortcut
    pub fn boolean(id: &str) -> FieldBuilder;
    pub fn select(id: &str) -> FieldBuilder;
    pub fn multi_select(id: &str) -> FieldBuilder;
    pub fn code(id: &str, language: &str) -> FieldBuilder;
    pub fn object(id: &str) -> FieldBuilder;
    pub fn list(id: &str, item: Field) -> FieldBuilder;
    pub fn mode(id: &str) -> FieldBuilder;
    pub fn color(id: &str) -> FieldBuilder;
    pub fn date(id: &str) -> FieldBuilder;
    pub fn datetime(id: &str) -> FieldBuilder;
    pub fn time(id: &str) -> FieldBuilder;
    pub fn hidden(id: &str) -> FieldBuilder;
    pub fn file(id: &str) -> FieldBuilder;
}

impl FieldBuilder {
    // ── Common ──────────────────────────────────────────
    pub fn label(self, label: &str) -> Self;
    pub fn description(self, desc: &str) -> Self;
    pub fn placeholder(self, text: &str) -> Self;
    pub fn default(self, value: impl Into<serde_json::Value>) -> Self;
    pub fn required(self) -> Self;
    pub fn secret(self) -> Self;
    pub fn rule(self, rule: Rule) -> Self;

    // ── Conditions ──────────────────────────────────────
    pub fn visible_when(self, cond: Condition) -> Self;
    pub fn required_when(self, cond: Condition) -> Self;
    pub fn disabled_when(self, cond: Condition) -> Self;

    // ── Expression ──────────────────────────────────────
    pub fn expression(self) -> Self;

    // ── Text-specific ───────────────────────────────────
    pub fn multiline(self) -> Self;

    // ── Number-specific ─────────────────────────────────
    pub fn range(self, min: impl Into<f64>, max: impl Into<f64>) -> Self;
    pub fn step(self, step: impl Into<f64>) -> Self;

    // ── Select-specific ─────────────────────────────────
    pub fn option(self, value: &str, label: &str) -> Self;
    pub fn options(self, options: Vec<SelectOption>) -> Self;
    pub fn dynamic(self, provider: &str) -> Self;
    pub fn depends_on(self, fields: &[&str]) -> Self;
    pub fn searchable(self) -> Self;
    pub fn allow_custom(self) -> Self;

    // ── Object-specific ─────────────────────────────────
    pub fn fields(self, fields: Vec<Field>) -> Self;

    // ── List-specific ───────────────────────────────────
    pub fn min_items(self, n: u32) -> Self;
    pub fn max_items(self, n: u32) -> Self;

    // ── File-specific ────────────────────────────────────
    pub fn accept(self, mime: &str) -> Self;
    pub fn max_size(self, bytes: u64) -> Self;
    pub fn multiple(self) -> Self;

    // ── Mode-specific ───────────────────────────────────
    pub fn variant(self, key: &str, label: &str, content: Field) -> Self;
    pub fn default_variant(self, key: &str) -> Self;

    // ── Build ───────────────────────────────────────────
    pub fn build(self) -> Field;
}
```

### Condition convenience constructors

```rust
impl Condition {
    pub fn eq(field: &str, value: impl Into<serde_json::Value>) -> Self;
    pub fn ne(field: &str, value: impl Into<serde_json::Value>) -> Self;
    pub fn gt(field: &str, value: f64) -> Self;
    pub fn lt(field: &str, value: f64) -> Self;
    pub fn is_true(field: &str) -> Self;
    pub fn is_false(field: &str) -> Self;
    pub fn is_set(field: &str) -> Self;
    pub fn is_empty(field: &str) -> Self;
    pub fn any_of(field: &str, values: &[impl Into<serde_json::Value>]) -> Self;
    pub fn matches(field: &str, pattern: &str) -> Self;
    pub fn all(conditions: Vec<Condition>) -> Self;
    pub fn any(conditions: Vec<Condition>) -> Self;
    pub fn not(condition: Condition) -> Self;
}
```

---

## Validation Flow

### Server-side (Rust)

```rust
/// Validate form values against the schema.
///
/// Returns field errors with paths, codes, and human-readable messages.
/// Errors are ordered by field declaration order in the schema.
/// Hidden fields (visible_when = false) are skipped unless they have a value.
/// Secret field values are never included in error messages.
pub fn validate(schema: &Schema, values: &ParameterValues) -> Vec<FieldError>;
```

### Client-side (JS pseudocode)

```javascript
function validate(schema, values) {
    const errors = [];
    for (const field of schema.fields) {
        // Skip hidden fields
        if (field.visible_when && !evaluate(field.visible_when, values)) continue;

        const value = values[field.id];

        // Required check
        const isRequired = field.required ||
            (field.required_when && evaluate(field.required_when, values));
        if (isRequired && isEmpty(value)) {
            errors.push({ path: field.id, code: "required", message: `${field.label} is required` });
            continue;
        }

        // Type-specific checks
        if (field.type === "number" && value != null) {
            if (field.min != null && value < field.min) {
                errors.push({ path: field.id, code: "min", message: `Minimum is ${field.min}` });
            }
            if (field.max != null && value > field.max) {
                errors.push({ path: field.id, code: "max", message: `Maximum is ${field.max}` });
            }
        }

        // Custom rules
        for (const rule of field.rules || []) {
            if (rule.rule === "pattern" && !new RegExp(rule.pattern).test(value)) {
                errors.push({ path: field.id, code: "pattern", message: rule.message });
            }
            // ... other rules
        }
    }
    return errors;
}
```

### Condition evaluator (shared logic)

The same algorithm works in Rust and JS:

```javascript
function evaluate(condition, values) {
    switch (condition.op) {
        case "eq":     return values[condition.field] === condition.value;
        case "ne":     return values[condition.field] !== condition.value;
        case "gt":     return values[condition.field] > condition.value;
        case "lt":     return values[condition.field] < condition.value;
        case "gte":    return values[condition.field] >= condition.value;
        case "lte":    return values[condition.field] <= condition.value;
        case "in":     return condition.values.includes(values[condition.field]);
        case "set":    return values[condition.field] != null;
        case "empty":  return !values[condition.field];
        case "is_true":  return values[condition.field] === true;
        case "is_false": return values[condition.field] === false;
        case "contains": return String(values[condition.field]).includes(condition.value);
        case "matches":  return new RegExp(condition.pattern).test(values[condition.field]);
        case "all":    return condition.conditions.every(c => evaluate(c, values));
        case "any":    return condition.conditions.some(c => evaluate(c, values));
        case "not":    return !evaluate(condition.condition, values);
    }
}
```

---

## Security

### Secret fields

Fields with `"secret": true`:
- UI renders a password-masked input
- Values are redacted in: logs, error messages, API read responses, telemetry
- Values are never included in `FieldError.message`
- Server stores values encrypted; API never returns plaintext in GET responses

No separate `SecurityPolicy` struct. The `secret` flag is the single control.
Implementation-level details (encryption, vault integration, redaction logic)
belong in `nebula-credential`, not in the schema.

### Validation of secrets

When validating a secret field, the error must never leak the value:

```json
// Good
{ "path": "bearer_token", "code": "required", "message": "Bearer Token is required" }

// Bad (leaks the value)
{ "path": "bearer_token", "code": "min_length", "message": "'sk-abc123' is too short" }
```

---

## Object and List — Nested Data

### Object — fixed structure

An Object field contains nested fields. The form renders them indented or in a sub-panel.
The value is a JSON object with keys matching nested field ids.

**Schema JSON:**

```json
{
  "id": "connection",
  "type": "object",
  "label": "Database Connection",
  "fields": [
    { "id": "host", "type": "text", "label": "Host", "required": true, "default": "localhost" },
    { "id": "port", "type": "number", "label": "Port", "integer": true, "min": 1, "max": 65535 },
    { "id": "username", "type": "text", "label": "Username", "required": true },
    { "id": "password", "type": "text", "label": "Password", "secret": true }
  ]
}
```

**Value:**

```json
{
  "connection": {
    "host": "db.example.com",
    "port": 5432,
    "username": "admin",
    "password": "s3cret"
  }
}
```

**Errors — nested paths use dot notation:**

```json
[
  { "path": "connection.host", "code": "required", "message": "Host is required" },
  { "path": "connection.port", "code": "min", "message": "Minimum is 1" }
]
```

**Nested conditionals and path resolution:**

Condition paths follow a **relative-first** rule:

| Path syntax | Resolution | Example |
|---|---|---|
| Bare name (`engine`) | Sibling within the same Object scope | A field inside `connection` referencing another field in `connection` |
| Dotted path (`connection.engine`) | Absolute from schema root | A top-level field referencing deep into `connection` |

**Rule:** When a condition is defined inside an Object (or inside a List item Object),
a bare field name always refers to a sibling in that same Object.
To reference a field outside the current scope, use the full dot-path from the schema root.

This avoids the ambiguity of deeply nested paths like
`inline_keyboard._row._btn.action_type` — inside the `_btn` Object, just write `action_type`.

```json
{
  "id": "connection",
  "type": "object",
  "label": "Connection",
  "fields": [
    {
      "id": "engine",
      "type": "select",
      "label": "Engine",
      "options": [
        { "value": "postgres", "label": "PostgreSQL" },
        { "value": "sqlite", "label": "SQLite" }
      ]
    },
    {
      "id": "host",
      "type": "text",
      "label": "Host",
      "visible_when": { "op": "ne", "field": "engine", "value": "sqlite" }
    },
    {
      "id": "file_path",
      "type": "text",
      "label": "DB File",
      "visible_when": { "op": "eq", "field": "engine", "value": "sqlite" }
    }
  ]
}
```

**Rust builder:**

```rust
Field::object("connection")
    .label("Database Connection")
    .fields(vec![
        Field::text("host").label("Host").required().default("localhost").build(),
        Field::integer("port").label("Port").range(1, 65535).build(),
        Field::text("username").label("Username").required().build(),
        Field::text("password").label("Password").secret().build(),
    ])
    .build()
```

### List — repeated items

A List field contains zero or more items of the same shape. The frontend renders
an "Add Item" button, and each item can be removed individually.

**Schema JSON:**

```json
{
  "id": "recipients",
  "type": "list",
  "label": "Recipients",
  "min_items": 1,
  "max_items": 10,
  "item": {
    "id": "_item",
    "type": "object",
    "label": "Recipient",
    "fields": [
      { "id": "email", "type": "text", "label": "Email", "required": true,
        "rules": [{ "rule": "pattern", "pattern": "^[^\\s@]+@[^\\s@]+\\.[^\\s@]+$", "message": "Invalid email format" }] },
      { "id": "role", "type": "select", "label": "Role",
        "source": "static",
        "options": [
          { "value": "to", "label": "To" },
          { "value": "cc", "label": "CC" },
          { "value": "bcc", "label": "BCC" }
        ]
      }
    ]
  }
}
```

**Value:**

```json
{
  "recipients": [
    { "email": "alice@example.com", "role": "to" },
    { "email": "bob@example.com", "role": "cc" }
  ]
}
```

**Errors — array index in path:**

```json
[
  { "path": "recipients", "code": "min_items", "message": "At least 1 item required" },
  { "path": "recipients.0.email", "code": "pattern", "message": "Invalid email format" },
  { "path": "recipients.1.role", "code": "required", "message": "Role is required" }
]
```

**List of scalars:**

When items are simple values (not objects), the `item` is a scalar field:

```json
{
  "id": "tags",
  "type": "list",
  "label": "Tags",
  "max_items": 20,
  "item": {
    "id": "_item",
    "type": "text",
    "label": "Tag",
    "rules": [{ "rule": "max_length", "max": 50 }]
  }
}
```

**Value:**

```json
{ "tags": ["urgent", "backend", "v2"] }
```

**Errors:**

```json
[
  { "path": "tags.2", "code": "max_length", "message": "Maximum length is 50" }
]
```

**Rust builder:**

```rust
// List of objects
Field::list("recipients",
    Field::object("_item")
        .label("Recipient")
        .fields(vec![
            Field::text("email")
              .label("Email")
              .required()
              .rule(Rule::pattern("^[^\\s@]+@[^\\s@]+\\.[^\\s@]+$", "Invalid email format"))
              .build(),
            Field::select("role").label("Role")
                .option("to", "To")
                .option("cc", "CC")
                .option("bcc", "BCC")
                .build(),
        ])
        .build()
)
.label("Recipients")
.min_items(1)
.max_items(10)
.build()

// List of scalars
Field::list("tags",
    Field::text("_item").label("Tag").rule(Rule::max_length(50)).build()
)
.label("Tags")
.max_items(20)
.build()
```

### Key-value pairs via List

HTTP headers, environment variables, custom metadata — all modeled as
`List<Object{key, value}>`. No special type needed.

**Schema JSON:**

```json
{
  "id": "env_vars",
  "type": "list",
  "label": "Environment Variables",
  "item": {
    "id": "_item",
    "type": "object",
    "label": "Variable",
    "fields": [
      { "id": "key", "type": "text", "label": "Name", "required": true },
      { "id": "value", "type": "text", "label": "Value" }
    ]
  }
}
```

**Value:**

```json
{
  "env_vars": [
    { "key": "DATABASE_URL", "value": "postgres://localhost/db" },
    { "key": "API_KEY", "value": "sk-abc123" }
  ]
}
```

**Errors:**

```json
[
  { "path": "env_vars.0.key", "code": "required", "message": "Name is required" }
]
```

**Rust builder:**

```rust
Field::list("env_vars",
    Field::object("_item")
        .label("Variable")
        .fields(vec![
            Field::text("key").label("Name").required().build(),
            Field::text("value").label("Value").build(),
        ])
        .build()
)
.label("Environment Variables")
.build()
```

### Error path conventions

| Field type | Value shape | Error path example |
|---|---|---|
| Scalar | `"value"` | `"url"` |
| Object | `{ "k": "v" }` | `"connection.host"` |
| List of scalars | `["a", "b"]` | `"tags.0"` |
| List of objects | `[{ "k": "v" }]` | `"recipients.0.email"` |
| Nested 2 levels | `{ "a": { "b": "c" } }` | `"config.db.host"` |
| Mode active variant | `{ "mode": "cron", "value": { "expr": "..." } }` | `"trigger.value.expression"` |

### Frontend rendering pseudocode

```javascript
function renderField(field, values, parentPath = "") {
    const path = parentPath ? `${parentPath}.${field.id}` : field.id;
    const value = getByPath(values, path);

    switch (field.type) {
        case "object":
            renderGroupHeader(field.label);
            for (const child of field.fields) {
                renderField(child, values, path);
            }
            break;

        case "list":
            renderLabel(field.label);
            const items = value || [];
            for (let i = 0; i < items.length; i++) {
                renderField(field.item, values, `${path}.${i}`);
                renderRemoveButton(i);
            }
            if (!field.max_items || items.length < field.max_items) {
                renderAddButton();
            }
            break;

        default:
            // scalar types: text, number, boolean, select, etc.
            renderWidget(field, value);
    }
}
```

---

## Real-World Examples

### Database Connection

```rust
let schema = Schema::builder()
    .field(
        Field::select("engine")
            .label("Database Engine")
            .required()
            .option("postgres", "PostgreSQL")
            .option("mysql", "MySQL")
            .option("sqlite", "SQLite")
    )
    .field(
        Field::text("host")
            .label("Host")
            .required()
            .default("localhost")
            .visible_when(Condition::ne("engine", "sqlite"))
    )
    .field(
        Field::integer("port")
            .label("Port")
            .range(1, 65535)
            .visible_when(Condition::ne("engine", "sqlite"))
            .required_when(Condition::ne("engine", "sqlite"))
    )
    .field(
        Field::text("username")
            .label("Username")
            .required()
            .visible_when(Condition::ne("engine", "sqlite"))
    )
    .field(
        Field::text("password")
            .label("Password")
            .secret()
            .visible_when(Condition::ne("engine", "sqlite"))
    )
    .field(
        Field::text("database")
            .label("Database")
            .required()
    )
    .field(
        Field::text("file_path")
            .label("Database File")
            .placeholder("/data/my.db")
            .required()
            .visible_when(Condition::eq("engine", "sqlite"))
    )
    .field(
        Field::code("extra_options", "json")
            .label("Connection Options")
            .description("Additional connection parameters as JSON")
    )
    .field(
        Field::boolean("ssl")
            .label("Enable SSL")
            .default(true)
            .visible_when(Condition::ne("engine", "sqlite"))
    )
    .group("Connection", &["engine", "host", "port", "username", "password", "database", "file_path"])
    .group_collapsed("Advanced", &["extra_options", "ssl"])
    .build()?;
```

### Dynamic Select (depends on another field)

```rust
// Schema definition
let schema = Schema::builder()
    .field(
        Field::select("project")
            .label("Project")
            .required()
            .dynamic("projects.list")
            .searchable()
    )
    .field(
        Field::select("environment")
            .label("Environment")
            .required()
            .dynamic("environments.list")
            .depends_on(&["project"])
    )
    .build()?;

// Provider implementation
struct EnvironmentsProvider { /* ... */ }

impl OptionProvider for EnvironmentsProvider {
    fn key(&self) -> &str { "environments.list" }

    async fn resolve(&self, req: &OptionRequest) -> Result<OptionPage, OptionError> {
        let project_id = req.values.get_string("project")
            .ok_or_else(|| OptionError::missing_dependency("project"))?;

        let envs = self.api.list_environments(project_id).await?;

        Ok(OptionPage {
            options: envs.into_iter().map(|e| SelectOption {
                value: json!(e.id),
                label: e.name,
                description: Some(e.region),
                disabled: false,
            }).collect(),
            next_cursor: None,
        })
    }
}
```

### Mode (discriminated union)

Mode is a **field-level input method switcher**. Each variant contains one content
node — a scalar for simple cases, an Object for multiple fields.

Value shape is always `{ "mode": "variant_key", "value": <content_value> }`.

**Single-field variants (scalar content):**

```rust
// Chat ID: user chooses how to provide the value
let schema = Schema::builder()
    .field(
        Field::mode("chat_id")
            .label("Chat ID")
            .required()
            .variant("by_id", "Enter ID",
                Field::text("id")
                    .placeholder("123456789")
                    .expression()
                    .build()
            )
            .variant("from_list", "Select Chat",
                Field::select("chat")
                    .dynamic("telegram.chats")
                    .searchable()
                    .build()
            )
            .default_variant("by_id")
    )
    .build()?;

// Value: { "mode": "by_id", "value": "123456789" }
// or:    { "mode": "from_list", "value": "chat_abc123" }
```

**Multi-field variants (Object content):**

```rust
// Authentication: variant complexity varies
let schema = Schema::builder()
    .field(
        Field::mode("auth")
            .label("Authentication")
            .variant("none", "No Auth",
                Field::hidden("_none").default(true).build()
            )
            .variant("bearer", "Bearer Token",
                Field::text("token")
                    .label("Token")
                    .secret()
                    .required()
                    .build()
            )
            .variant("basic", "Basic Auth",
                Field::object("credentials")
                    .fields(vec![
                        Field::text("username").label("Username").required().build(),
                        Field::text("password").label("Password").secret().required().build(),
                    ])
                    .build()
            )
            .default_variant("none")
    )
    .build()?;

// Value: { "mode": "none", "value": true }
// or:    { "mode": "bearer", "value": "sk-abc123" }
// or:    { "mode": "basic", "value": { "username": "admin", "password": "***" } }
```

**Trigger type (mixed content):**

```rust
let schema = Schema::builder()
    .field(
        Field::mode("trigger")
            .label("Trigger Type")
            .variant("cron", "Schedule",
                Field::object("config")
                    .fields(vec![
                        Field::text("expression")
                            .label("Cron Expression")
                            .required()
                            .placeholder("0 * * * *")
                            .build(),
                        Field::text("timezone")
                            .label("Timezone")
                            .default("UTC")
                            .build(),
                    ])
                    .build()
            )
            .variant("webhook", "Webhook",
                Field::object("config")
                    .fields(vec![
                        Field::text("path")
                            .label("Webhook Path")
                            .required()
                            .build(),
                        Field::select("http_method")
                            .label("Method")
                            .option("POST", "POST")
                            .option("GET", "GET")
                            .default("POST")
                            .build(),
                    ])
                    .build()
            )
            .variant("event", "Event",
                Field::text("event_name")
                    .label("Event Name")
                    .required()
                    .build()
            )
            .default_variant("cron")
    )
    .build()?;

// Value: { "mode": "cron", "value": { "expression": "0 * * * *", "timezone": "UTC" } }
// or:    { "mode": "event", "value": "user.created" }
```

**JSON schema for Mode:**

```json
{
  "id": "chat_id",
  "type": "mode",
  "label": "Chat ID",
  "required": true,
  "default_variant": "by_id",
  "variants": [
    {
      "key": "by_id",
      "label": "Enter ID",
      "content": {
        "id": "id",
        "type": "text",
        "placeholder": "123456789",
        "expression": true
      }
    },
    {
      "key": "from_list",
      "label": "Select Chat",
      "content": {
        "id": "chat",
        "type": "select",
        "source": "dynamic",
        "provider": "telegram.chats",
        "searchable": true
      }
    }
  ]
}
```

---

## Migration from v1

### What changes

| v1 (ParameterDef) | v2 (Schema/Field) |
|---|---|
| `TextParameter` | `Field::text(id)` |
| `NumberParameter` with `f64` | `Field::number(id)` or `Field::integer(id)` |
| `CheckboxParameter` | `Field::boolean(id)` |
| `SelectParameter` | `Field::select(id)` |
| `MultiSelectParameter` | `Field::select(id).multiple()` |
| `SecretParameter` | `Field::text(id).secret()` |
| `CodeParameter` | `Field::code(id, lang)` |
| `NoticeParameter` | `UiElement::Notice` (not a field) |
| `HiddenParameter` | `Field::hidden(id)` |
| `GroupParameter` | `Group` in schema layout |
| `ExpirableParameter` | Dropped (runtime concern, not schema) |
| `ParameterMetadata` | `FieldMeta` (inlined) |
| `DisplayCondition` + `DisplayRuleSet` | `Condition` enum |
| `ValidationRule` | `Rule` enum |
| `ParameterCollection` | `Schema` |

### What's removed

- **ExpirableParameter** — TTL/caching is a runtime concern, not a form schema concept.
- **Subtype wrappers** — removed from schema surface.
  In Nebula, subtype-like behavior is modeled as explicit validation rules
  (for example `Rule::Pattern` with predefined regex shortcuts), while UI behavior
  still comes from `type`, `rules`, `expression`, and provider-based selects.
- **IntBits/NumberKind** — replaced by `integer: bool`. The schema tells the frontend
  "this is an integer" or "this is a decimal". Rust-side range enforcement handles precision.
- **SecurityPolicy/RedactionTargets/PersistPolicy** — replaced by `secret: bool`.
  The rest is implementation detail in credential/runtime crates.
- **LayoutNode/GroupNode/PanelNode hierarchy** — replaced by flat `Group`.
  Complex layout (tabs, cards, collapsible sections) is frontend responsibility,
  not schema bloat.
- **MatrixSpec/RoutingSpec/ReferenceSpec** — over-specialized containers excluded from v1.
- **ValidationPlan/CompiledSchema** — validation is simple sequential evaluation.
  Pre-compilation is an implementation optimization, not an API surface.
- **FieldId newtype** — plain `String`. A newtype adds ceremony without safety benefit
  in a JSON-first API.
- **ExprTarget** — conditions always reference fields by id. No "local" vs "field" distinction.
- **ValueSource** — how values are resolved (literal, expression, secret ref) is runtime,
  not schema.
- **OrderingContract** — deterministic ordering is an implementation guarantee, not a type.
- **Legacy compat module** — migration is a one-time operation, not a permanent API surface.

---

## Invariants

1. **UI elements never appear in values.** `UiElement::Notice` and `UiElement::Button` produce
   no form data. Validation ignores them.

2. **Hidden fields skip visibility checks.** `Field::Hidden` always has a value but is never
   shown. Used for internal node state.

3. **Conditions reference field ids only.** No arbitrary expressions in the schema JSON.
   Complex logic belongs in custom validation rules or the expression engine.

4. **Validation order = declaration order.** Errors are reported in the order fields
   appear in `schema.fields`. Deterministic across runs.

5. **Secret values never appear in errors.** Error messages for secret fields use
   generic text ("value is too short") without the actual value.

6. **Dynamic options are fetched, not embedded.** The schema declares the provider key.
   The runtime fetches options when needed. Options are not cached in the schema.

7. **Groups are advisory.** A field not mentioned in any group still renders (after all
   groups, in declaration order). Groups control presentation, not data.

---

## References

- Industry research: [13-INDUSTRY-REFERENCE.md](13-INDUSTRY-REFERENCE.md)
- n8n type system: [interfaces.ts](interfaces.ts)
- Current implementation: `crates/parameter/`

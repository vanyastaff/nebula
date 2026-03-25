# nebula-parameter — High-Level Design (v2)

## Overview

nebula-parameter defines how configurable surfaces work across the Nebula
workflow engine. It provides schema definition, value storage, validation,
normalization, and serialization for forms used by actions, credentials,
and resources.

### What it solves

Every configurable surface in Nebula needs the same thing: describe a set
of typed fields with labels, conditions, validation rules, and let
something (frontend, CLI, OpenAPI generator) render a form from it.

| Consumer | Schema describes | User sees |
|----------|------------------|-----------|
| Action node | Node input parameters | Workflow editor settings panel |
| Credential | Auth setup (OAuth2, API key, DB) | "Add credential" dialog |
| Resource config | Operational settings (pool, timeouts) | Admin configuration page |
| Trigger | Event filter configuration | Trigger settings panel |

```rust
// Action
let params = ParameterCollection::new()
    .add(Parameter::string("chat_id").label("Chat ID").required())
    .add(Parameter::string("text").label("Text").multiline().required());

// Credential
let cred = ParameterCollection::new()
    .add(Parameter::string("token").label("Bot Token").secret().required());

// Resource config
let config = ParameterCollection::new()
    .add(Parameter::integer("pool_size").label("Pool Size").default(json!(10)));
```

### Core guarantees

1. **Serde-first** — every type round-trips through JSON. The wire format is
   the source of truth. Frontend consumes JSON, backend produces JSON,
   schemas persist as JSON in database.

2. **Headless** — zero UI dependencies. Schema + validation + normalization
   only. Rendering is the consumer's problem.

3. **Developer-friendly** — fluent builder API. Common case is one line.
   Complex cases compose naturally.

### System boundaries

| Concern | Owner | Integration |
|---------|-------|-------------|
| Schema definition | nebula-parameter | `ParameterCollection` type |
| Value validation | nebula-parameter + nebula-validator | `Rule` delegates to validator |
| Value normalization | nebula-parameter | `normalize()` on collection |
| Expression evaluation | nebula-engine (runtime) | Stores `$expr` marker, engine resolves |
| Dynamic loading | nebula-parameter defines contract | `OptionLoader` / `RecordLoader` closures |
| UI rendering | frontend (React/Vue/CLI) | Consumes JSON schema |
| Persistence | nebula-engine / nebula-credential | Stores `ParameterValues` as JSON |

### Portable schema vs runtime-attached schema

nebula-parameter produces two layers of schema:

**Portable schema** — everything that serializes to JSON. This is what gets
stored in database, sent to frontend, compared for equality, cached, and
transported across process boundaries. It is the contract.

**Runtime-attached schema** — portable schema plus non-serializable closures
(`OptionLoader`, `RecordLoader`). These exist only in the Rust process that
defined them. They are `#[serde(skip)]` and excluded from equality/hashing.

Rule: schema equality = structural equality of portable schema only.
Runtime-attached behavior is never part of equality, debug fingerprint,
or cache keys.

---

## Schema Author View

### The one pattern

```rust
use nebula_parameter::prelude::*;

let schema = ParameterCollection::new()
    .add(Parameter::string("name").label("Name").required())
    .add(Parameter::integer("age").label("Age").default(json!(18)))
    .add(Parameter::boolean("active").label("Active").default(json!(true)));
```

Collection serializes to JSON. Frontend renders form. User fills values.
Values come back as `ParameterValues`. You validate.

### Parameter types

| Type | Data | Example |
|------|------|---------|
| `String` | `"hello"` | Text input, textarea |
| `Number` | `42` or `3.14` | Number input, slider |
| `Boolean` | `true` / `false` | Toggle, checkbox |
| `Select` | `"value"` or `["a","b"]` | Dropdown, radio, multi-select |
| `Object` | `{ "host": "...", "port": 5432 }` | Nested field group, collapsible section |
| `List` | `[item1, item2, ...]` | Repeatable items |
| `Mode` | `{ "mode": "bearer", "value": {...} }` | Discriminated union |
| `Code` | `"SELECT * FROM ..."` | Syntax-highlighted editor |
| `Date` | `"2025-01-15"` | Date picker |
| `DateTime` | `"2025-01-15T10:30:00Z"` | Date+time picker |
| `Time` | `"14:30"` | Time picker |
| `Color` | `"#ff6600"` | Color picker |
| `File` | file reference | File upload |
| `Hidden` | any JSON value | Not displayed, stored |
| `Filter` | predicate expression | Visual condition builder |
| `Computed` | derived from siblings | Read-only calculated field |
| `Dynamic` | resolved at runtime | Provider-driven fields |

### Fluent builder

```rust
// Minimal
Parameter::string("name")

// Common
Parameter::string("name").label("Name").required()

// Full metadata
Parameter::string("api_key")
    .label("API Key")
    .description("Your service API key")
    .placeholder("sk-...")
    .hint("Find this in your dashboard settings")
    .secret()
    .required()
    .with_rule(Rule::MinLength { min: 10, message: None })

// Number with constraints
Parameter::number("timeout_ms")
    .label("Timeout")
    .integer()
    .min(100)
    .max(60_000)
    .step(100)
    .default(json!(5000))

// Select — static options
Parameter::select("method")
    .label("HTTP Method")
    .option("GET", "GET")
    .option("POST", "POST")
    .option("PUT", "PUT")
    .option("DELETE", "DELETE")
    .default(json!("GET"))

// Select — dynamic options with loader
Parameter::select("table")
    .label("Table")
    .depends_on(&["database"])
    .searchable()
    .loader(|ctx: LoaderContext| async move {
        let db = ctx.values.get_string("database").unwrap_or("default");
        let tables = fetch_tables(db, &ctx.credential).await?;
        Ok(tables.into_iter()
            .map(|t| SelectOption::new(json!(t), t))
            .collect())
    })

// Multi-select with custom values
Parameter::select("tags")
    .label("Tags")
    .multiple()
    .allow_custom()
    .option("urgent", "Urgent")
    .option("bug", "Bug")
    .option("feature", "Feature")

// input_type — HTML input type override
Parameter::string("email").label("Email").input_type("email")
Parameter::string("token").label("Token").input_type("password").secret()
Parameter::string("phone").label("Phone").input_type("tel")
Parameter::number("opacity").label("Opacity").input_type("range").min(0).max(100)

// computed — read-only derived fields (separate type, no .required()/.secret()/.placeholder())
Parameter::computed("connection_string")
    .label("Connection String")
    .returns_string()
    .expression("postgres://{{ host }}:{{ port }}/{{ database }}")
Parameter::computed("total")
    .label("Total")
    .returns_number()
    .expression("{{ price * quantity }}")
Parameter::computed("can_submit")
    .label("Ready")
    .returns_boolean()
    .expression("{{ quantity > 0 && price > 0 }}")
```

### Nested structures

**Object** — nested field group:

```rust
Parameter::object("auth")
    .label("Authentication")
    .add(Parameter::string("username").label("Username").required())
    .add(Parameter::string("password").label("Password").secret().required())

// Collapsed — renders as collapsible section in UI
Parameter::object("advanced")
    .label("Advanced Settings")
    .collapsed()
    .add(Parameter::integer("timeout_ms").label("Timeout (ms)").default(json!(5000)))
    .add(Parameter::boolean("debug").label("Debug Mode").default(json!(false)))
```

**List** — repeatable items:

```rust
Parameter::list("headers")
    .label("HTTP Headers")
    .item(Parameter::object("header")
        .add(Parameter::string("key").label("Key").required())
        .add(Parameter::string("value").label("Value").required()))
    .min_items(0)
    .max_items(50)
    .sortable()

Parameter::list("tags")
    .label("Tags")
    .item(Parameter::string("tag"))
    .unique()
    .sortable()
```

**Mode** — discriminated union:

```rust
Parameter::mode("auth_type")
    .label("Authentication")
    .variant("none", "None",
        Parameter::hidden("_placeholder"))
    .variant("api_key", "API Key",
        Parameter::string("key").label("API Key").secret().required())
    .variant("oauth2", "OAuth2",
        Parameter::object("oauth2_config")
            .add(Parameter::string("client_id").label("Client ID").required())
            .add(Parameter::string("client_secret").label("Client Secret").secret().required())
            .add(Parameter::string("scope").label("Scope")))
    .variant("bearer", "Bearer Token",
        Parameter::string("token").label("Token").secret().required())
    .default_variant("none")
```

**Dynamic** — runtime-resolved fields:

```rust
Parameter::dynamic("row_data")
    .label("Row Data")
    .depends_on(&["sheet_id"])
    .loader(|ctx: LoaderContext| async move {
        let sheet = ctx.values.get_string("sheet_id").unwrap_or("");
        let columns = fetch_sheet_columns(sheet, &ctx.credential).await?;
        Ok(columns.into_iter()
            .map(|col| Parameter::string(&col.id).label(&col.name))
            .collect())
    })
```

### Conditional fields

```rust
// Show only when operation is "getForecast"
Parameter::integer("days")
    .label("Forecast Days")
    .default(json!(5))
    .visible_when(Condition::eq("operation", json!("getForecast")))

// Require only when auth_mode is "bearer"
Parameter::string("token")
    .label("Token")
    .required_when(Condition::eq("auth_mode", json!("bearer")))

// Combined conditions
Parameter::string("proxy_url")
    .label("Proxy URL")
    .visible_when(Condition::all(vec![
        Condition::eq("use_proxy", json!(true)),
        Condition::ne("environment", json!("local")),
    ]))
```

### Resource → Operation pattern

Not a built-in primitive — expressed with selects and conditions:

```rust
let telegram = ParameterCollection::new()
    .add(Parameter::select("resource")
        .label("Resource")
        .option("message", "Message")
        .option("chat", "Chat")
        .default(json!("message")))
    .add(Parameter::select("operation")
        .label("Operation")
        .option("sendMessage", "Send Message")
        .option("sendPhoto", "Send Photo")
        .default(json!("sendMessage")))
    .add(Parameter::string("text")
        .label("Text")
        .multiline()
        .required_when(Condition::eq("operation", json!("sendMessage")))
        .visible_when(Condition::eq("operation", json!("sendMessage"))))
    .add(Parameter::file("photo")
        .label("Photo")
        .accept("image/*")
        .required_when(Condition::eq("operation", json!("sendPhoto")))
        .visible_when(Condition::eq("operation", json!("sendPhoto"))));
```

### Real-world examples

**Credential form:**

```rust
let oauth2_github = ParameterCollection::new()
    .add(Parameter::string("client_id")
        .label("Client ID")
        .description("GitHub OAuth App client ID")
        .required())
    .add(Parameter::string("client_secret")
        .label("Client Secret")
        .description("GitHub OAuth App client secret")
        .secret()
        .required())
    .add(Parameter::string("scope")
        .label("Scope")
        .default(json!("repo,user")));
```

**Resource config:**

```rust
let postgres_config = ParameterCollection::new()
    .add(Parameter::integer("connect_timeout_ms")
        .label("Connect Timeout (ms)")
        .default(json!(5000))
        .min(0).max(60_000))
    .add(Parameter::integer("statement_timeout_ms")
        .label("Statement Timeout (ms)")
        .default(json!(30_000))
        .min(0))
    .add(Parameter::string("application_name")
        .label("Application Name")
        .default(json!("nebula")))
    .add(Parameter::string("search_path")
        .label("Search Path")
        .placeholder("public"))
    .add(Parameter::select("recycle_method")
        .label("Recycle Method")
        .option("full", "Full (DISCARD ALL always)")
        .option("smart", "Smart (DISCARD ALL only if needed)")
        .default(json!("smart")));
```

---

## Core Types

### Parameter

```rust
/// A single parameter definition. Struct with shared metadata +
/// ParameterType for type-specific configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Parameter {
    /// Stable identifier, unique within parent scope.
    pub id: String,

    /// Data type and type-specific configuration.
    #[serde(flatten)]
    pub param_type: ParameterType,

    /// User-facing label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,

    /// Longer description / tooltip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Placeholder text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,

    /// Short contextual hint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,

    /// Default JSON value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,

    /// Whether the field must have a value.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub required: bool,

    /// Whether the value is sensitive (masked in UI, excluded from logs).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub secret: bool,

    /// Whether the field accepts expression-backed values (`{ "$expr": "..." }`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub expression: bool,

    /// HTML input type override. Defaults based on ParameterType when absent.
    /// Common: "email", "url", "password", "tel", "range", "search".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_type: Option<String>,

    /// Validation rules.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<Rule>,

    /// Show this field only when condition is true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_when: Option<Condition>,

    /// Require this field only when condition is true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_when: Option<Condition>,

    /// Disable this field when condition is true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled_when: Option<Condition>,
}

impl Parameter {
    /// Shorthand: sets both `visible_when` and `required_when` to the same condition.
    /// Covers the 80% case where a field should appear AND be required together.
    /// Use separate `.visible_when()` / `.required_when()` when they differ.
    pub fn active_when(mut self, condition: Condition) -> Self {
        self.visible_when = Some(condition.clone());
        self.required_when = Some(condition);
        self
    }
}
```

**Why struct + ParameterType, not an enum:**
- Shared fields (label, required, rules, conditions) — one implementation.
  No match on 17 variants.
- Type-specific fields (multiline, min/max, options) — inside ParameterType.
  Match only for type-specific logic.
- Adding a new type: one variant + serde tag. Zero changes to validation,
  normalization, lint for the shared metadata path.

**Constructor inventory — every ParameterType gets a shorthand:**

```rust
Parameter::string(id)       // String { multiline: false }
Parameter::number(id)       // Number { integer: false, ... }
Parameter::integer(id)      // Number { integer: true, ... }
Parameter::boolean(id)      // Boolean
Parameter::select(id)       // Select { options: [], ... }
Parameter::object(id)       // Object { parameters: [], collapsed: false }
Parameter::list(id)         // List { item, ... }
Parameter::mode(id)         // Mode { variants: [], ... }
Parameter::code(id)         // Code { language: "" }
Parameter::date(id)         // Date
Parameter::datetime(id)     // DateTime
Parameter::time(id)         // Time
Parameter::color(id)        // Color
Parameter::file(id)         // File { accept: None, ... }
Parameter::hidden(id)       // Hidden
Parameter::filter(id)       // Filter { operators: None, ... }
Parameter::computed(id)     // Computed { expression: "", returns: String }
Parameter::dynamic(id)      // Dynamic { depends_on: [], loader: None }
```

**Numeric builder ergonomics:**

`.min()`, `.max()`, `.step()` accept `impl Into<serde_json::Number>`.
Implementations provided for `i32`, `i64`, `u32`, `u64`, `f64`:

```rust
Parameter::number("temperature").min(0.0).max(2.0).step(0.1)  // f64 → serde_json::Number
Parameter::integer("port").min(1).max(65535)                    // i32 → serde_json::Number
```

### ParameterType

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ParameterType {
    /// Free-form text.
    String {
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        multiline: bool,
    },

    /// Numeric value.
    Number {
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        integer: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<serde_json::Number>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<serde_json::Number>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        step: Option<serde_json::Number>,
    },

    /// Boolean toggle.
    Boolean,

    /// Single or multi-select from options.
    Select {
        /// Static options (may be empty for dynamic-only selects).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        options: Vec<SelectOption>,
        /// When true, runtime loads options via attached OptionLoader.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        dynamic: bool,
        /// Re-load when these parameter values change. Accepts ParameterPath.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        depends_on: Vec<ParameterPath>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        multiple: bool,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        allow_custom: bool,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        searchable: bool,
        /// Async closure. Not serialized. `.loader()` sets `dynamic = true`.
        #[serde(skip)]
        loader: Option<OptionLoader>,
    },

    /// Nested object with ordered sub-parameters.
    Object {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        parameters: Vec<Parameter>,
        /// Starts collapsed in UI.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        collapsed: bool,
    },

    /// Repeated items from a template.
    List {
        item: Box<Parameter>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min_items: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_items: Option<u32>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        unique: bool,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        sortable: bool,
    },

    /// Discriminated union with named variants.
    Mode {
        variants: Vec<ModeVariant>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default_variant: Option<String>,
    },

    /// Syntax-highlighted code editor.
    Code { language: String },

    /// Calendar date picker.
    Date,

    /// Date and time picker.
    DateTime,

    /// Time picker.
    Time,

    /// Color picker.
    Color,

    /// File upload.
    File {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        accept: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_size: Option<u64>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        multiple: bool,
    },

    /// Hidden — stored value, no editor.
    Hidden,

    /// Visual filter / condition builder.
    Filter {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        operators: Option<Vec<FilterOp>>,
        #[serde(default)]
        allow_groups: bool,
        #[serde(default = "default_depth")]
        max_depth: u8,
    },

    /// Computed field — read-only, value derived from siblings.
    /// Unlike other types, Computed does not accept user input.
    /// Runtime evaluates the expression and produces the value.
    Computed {
        /// Expression in `{{ }}` syntax.
        /// Examples: `"{{ price * quantity }}"`, `"{{ host }}:{{ port }}"`.
        expression: String,
        /// What type the expression produces (frontend rendering hint).
        returns: ComputedReturn,
    },

    /// Dynamic fields resolved at runtime by loader.
    /// Loader returns `Vec<Parameter>` rendered as if inline.
    Dynamic {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        depends_on: Vec<ParameterPath>,
        #[serde(skip)]
        loader: Option<RecordLoader>,
    },
}
```

### Supporting types

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputedReturn {
    String,
    Number,
    Boolean,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectOption {
    /// Any JSON value — string, number, bool.
    pub value: serde_json::Value,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModeVariant {
    pub key: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub content: Box<Parameter>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ParameterCollection {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<Parameter>,
}
```

### ParameterPath

```rust
/// Typed reference to a parameter within a schema.
/// Resolves relative to the scope where it's used.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ParameterPath(String);

impl ParameterPath {
    /// Sibling reference: "field_name"
    pub fn sibling(id: &str) -> Self { Self(id.to_owned()) }

    /// Nested child path: "object.child.field"
    pub fn nested(path: &str) -> Self { Self(path.to_owned()) }

    /// Absolute from root: "$root.field_name"
    pub fn root(id: &str) -> Self { Self(format!("$root.{id}")) }

    /// Is this an absolute ($root) reference?
    pub fn is_absolute(&self) -> bool { self.0.starts_with("$root.") }

    /// Path segments: "auth.token" → ["auth", "token"]
    pub fn segments(&self) -> Vec<&str> {
        let s = self.0.strip_prefix("$root.").unwrap_or(&self.0);
        s.split('.').collect()
    }
}

/// Auto-conversion from &str — DX shorthand for the 80% sibling case.
impl From<&str> for ParameterPath {
    fn from(s: &str) -> Self { Self(s.to_owned()) }
}
```

**Scope resolution rules:**

1. Paths resolve **relative to the scope** where the parameter is defined.
2. Inside `Object { parameters }` → siblings are the other object fields.
3. Inside `List { item }` → siblings are the item's fields (per-item scope).
4. Inside `Mode { variant.content }` → scope is the variant content.
5. `$root.field` escapes to the root collection (absolute reference).
6. 95% of conditions use sibling references (plain `"field_name"`).
   `$root` is the escape hatch for cross-scope references.

```rust
// Simple sibling — 80% case. &str auto-converts to ParameterPath.
Condition::eq("operation", json!("sendMessage"))

// Cross-scope — inside nested Object referencing root-level field.
Condition::eq(ParameterPath::root("method"), json!("GET"))
```

### Condition

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Condition {
    Eq { field: ParameterPath, value: serde_json::Value },
    Ne { field: ParameterPath, value: serde_json::Value },
    OneOf { field: ParameterPath, values: Vec<serde_json::Value> },
    Set { field: ParameterPath },
    NotSet { field: ParameterPath },
    IsTrue { field: ParameterPath },
    Gt { field: ParameterPath, value: serde_json::Value },
    Lt { field: ParameterPath, value: serde_json::Value },
    All { conditions: Vec<Condition> },
    Any { conditions: Vec<Condition> },
    Not { condition: Box<Condition> },
}
```

`Condition` is separate from `Rule`. Condition = predicate on sibling field
values (visibility, conditional required). Rule = validation constraint on
the field's own value (min_length, pattern, etc.). They never mix.

`ParameterPath` in Condition fields accepts `&str` via `From<&str>`, so
`Condition::eq("field", value)` still works for simple sibling references.

---

## Serialization

### Wire format

Flat JSON with `"type"` discriminator via `#[serde(flatten)]` on `param_type`:

```json
{
  "id": "chat_id",
  "type": "string",
  "label": "Chat ID",
  "required": true
}
```

```json
{
  "id": "method",
  "type": "select",
  "label": "HTTP Method",
  "options": [
    { "value": "GET", "label": "GET" },
    { "value": "POST", "label": "POST" }
  ],
  "default": "GET"
}
```

```json
{
  "id": "table",
  "type": "select",
  "label": "Table",
  "dynamic": true,
  "depends_on": ["database"],
  "searchable": true
}
```

```json
{
  "id": "auth",
  "type": "mode",
  "label": "Authentication",
  "default_variant": "none",
  "variants": [
    {
      "key": "none",
      "label": "None",
      "content": { "id": "_", "type": "hidden" }
    },
    {
      "key": "bearer",
      "label": "Bearer Token",
      "content": { "id": "token", "type": "string", "secret": true, "required": true }
    }
  ]
}
```

```json
{
  "id": "total",
  "type": "computed",
  "label": "Total",
  "returns": "number",
  "expression": "{{ price * quantity }}"
}
```

```json
{
  "id": "row_data",
  "type": "dynamic",
  "depends_on": ["sheet_id"]
}
```

**Collection:**

```json
{
  "parameters": [
    { "id": "name", "type": "string", "label": "Name", "required": true },
    { "id": "age", "type": "number", "label": "Age", "integer": true, "default": 18 }
  ]
}
```

### Round-trip guarantee

```rust
let param = Parameter::string("name").label("Name").required();
let json = serde_json::to_value(&param).unwrap();
let back: Parameter = serde_json::from_value(json).unwrap();
assert_eq!(param, back);
```

Applies to portable schema only. Loaders are `#[serde(skip)]` and excluded
from round-trip, equality, and hashing.

---

## Dynamic Loading

### OptionLoader and RecordLoader

Both are type-erased async closures. Calling `.loader()` automatically sets
`dynamic = true`. Frontend sees `"dynamic": true` in JSON, requests options
by parameter id. Backend finds the parameter, calls its closure.

```rust
/// Shared context for all loader types.
pub struct LoaderContext {
    pub parameter_id: String,
    /// Current user-entered form values.
    pub values: ParameterValues,
    /// Optional text filter (for searchable selects).
    pub filter: Option<String>,
    /// Pagination cursor from previous load.
    pub cursor: Option<String>,
    /// Resolved credential (if available).
    pub credential: Option<serde_json::Value>,
    /// Engine-injected context (upstream schemas, tenant info, etc.).
    /// Separate from `values` to avoid polluting user data and validation.
    pub metadata: Option<serde_json::Value>,
}

/// Resolves select options. Attached to Select via `.loader()`.
pub struct OptionLoader(
    Arc<dyn Fn(LoaderContext) -> Pin<Box<dyn Future<
        Output = Result<Vec<SelectOption>, LoaderError>
    > + Send>> + Send + Sync>
);

/// Resolves dynamic field definitions. Attached to Dynamic via `.loader()`.
pub struct RecordLoader(
    Arc<dyn Fn(LoaderContext) -> Pin<Box<dyn Future<
        Output = Result<Vec<Parameter>, LoaderError>
    > + Send>> + Send + Sync>
);
```

**Key design points:**

- Closure-based — no trait to implement.
- `Result` return — I/O errors surfaced, not hidden in empty vectors.
- `#[serde(skip)]` — loaders exist only in-process, never serialized.
- `PartialEq` always `true` — loaders excluded from schema comparison.
  Schema equality = portable structural equality only.

### Select with loader

```rust
Parameter::select("assigned_to")
    .label("Assigned To")
    .depends_on(&["project_id"])
    .searchable()
    .loader(|ctx: LoaderContext| async move {
        let project = ctx.values.get_string("project_id").unwrap_or("");
        let users = fetch_project_users(project, &ctx.credential).await?;
        let filter = ctx.filter.as_deref().unwrap_or("").to_lowercase();
        Ok(users.into_iter()
            .filter(|u| filter.is_empty() || u.name.to_lowercase().contains(&filter))
            .map(|u| SelectOption::new(json!(u.id), &u.name))
            .collect())
    })
```

### Dynamic with loader

```rust
Parameter::dynamic("row_data")
    .label("Row Data")
    .depends_on(&["sheet_id"])
    .loader(|ctx: LoaderContext| async move {
        let sheet = ctx.values.get_string("sheet_id").unwrap_or("");
        let columns = fetch_sheet_columns(sheet, &ctx.credential).await?;
        Ok(columns.into_iter()
            .map(|col| Parameter::string(&col.id).label(&col.name))
            .collect())
    })
```

---

## Validation

### API

```rust
impl ParameterCollection {
    /// Strict validation. Unknown fields are errors.
    pub fn validate(&self, values: &ParameterValues)
        -> Result<(), Vec<ParameterError>>;

    /// Validation with profile control.
    pub fn validate_with_profile(
        &self, values: &ParameterValues, profile: ValidationProfile
    ) -> ValidationReport;
}

pub enum ValidationProfile {
    Strict,      // unknown fields → error (default)
    Warn,        // unknown fields → warning
    Permissive,  // unknown fields → silent
}
```

### Validation flow

```
values arrive (ParameterValues)
       │
for each Parameter in schema:
       ├─ type == Computed? → skip (derived by runtime)
       ├─ visible_when → hidden + no value? skip
       ├─ required / required_when → missing? error
       ├─ expression check: value is { "$expr": "..." }?
       │    → yes: validate wrapper shape only, skip type/rule checks
       │    → no: continue
       ├─ type check (string is string, number is number)
       ├─ type-specific:
       │    Select: value in options (unless allow_custom)?
       │    Number: min/max/integer?
       │    Object: recurse into sub-parameters
       │    List: min_items/max_items + unique + recurse items
       │    Mode: valid variant? recurse variant content
       │    Dynamic: skip (resolved at runtime)
       └─ apply Rule[] → nebula-validator
       │
check for unknown fields (per ValidationProfile)
       │
ValidationReport { errors, warnings }
```

### Field activity semantics

| State | Validation | Normalization | Submission |
|-------|-----------|---------------|------------|
| Visible + present | Full validation | Preserve value | Include |
| Visible + missing | Required check | Apply default | Include default or absent |
| Hidden + missing | Skip entirely | Skip default | Exclude |
| Hidden + present | Skip validation | Preserve value | Include (user set it before hiding) |
| Disabled + present | Full validation | Preserve value | Include (read-only, not invisible) |
| Disabled + missing | Required check | Apply default | Include default or absent |
| Computed | Skip all | Runtime evaluates | Runtime includes result |

**Precedence:** normalize() applies defaults first, then validate() checks
required. `null` = absent. Empty string = present. Empty array = present.

### Expression validation policy

When a value is `{ "$expr": "{{ ... }}" }`:

- This crate validates wrapper shape only (object with one `$expr` string key).
- Type checks, rules, type-specific validation are all skipped.
- Runtime evaluates the expression and validates the resolved value.
- `required` check passes if the `$expr` wrapper is present.
- `default` does NOT apply if value is an expression wrapper.

### depends_on semantics

`depends_on` means: **re-trigger loader when any listed field changes.**
The loader receives all current values and decides itself whether it has
enough context to produce results. If a required field is empty, loader
returns an appropriate `LoaderError` (e.g. "Select a spreadsheet first").

Frontend behavior: when any `depends_on` field changes, clear current
selection and call loader again.

### Dynamic field values

Dynamic field values are **nested under the parent parameter ID** as an
object. This prevents collision with static sibling fields:

```json
{
  "spreadsheet_id": "abc123",
  "sheet_name": "Sheet1",
  "row_data": {
    "Name": "Alice",
    "Email": "alice@example.com"
  }
}
```

The Dynamic parameter `row_data` owns the namespace. Resolved fields
(`Name`, `Email`) live inside `row_data` object.

### Mode value contract

Runtime value shape is always:

```json
{ "mode": "variant_key", "value": { ... } }
```

This shape is mandatory and canonical. Rules:

- `mode` key must match a variant key or the `default_variant`.
- Only the selected variant's content is validated.
- Inactive branch values are NOT validated but MAY be preserved in storage
  (enables UI to remember settings when switching variants).
- Normalization injects `{ "mode": "<default>" }` when absent.
- Normalization recurses into selected variant for nested defaults.
- Normalization does NOT invent defaults for inactive branches.
- `required` on a parameter inside variant content means "required when
  this variant is active." Inactive variant content is never validated.

---

## Normalization

```rust
impl ParameterCollection {
    pub fn normalize(&self, values: &ParameterValues) -> ParameterValues;
}
```

Rules:

1. Parameter has `default` and value absent → set to default.
2. Parameter type is `Computed` → skip (runtime evaluates).
3. Mode with `default_variant` and value absent → inject `{ "mode": "<key>" }`.
4. Mode with selected variant → recurse into variant content.
5. Existing user values NEVER overwritten.
6. Extra keys not in schema are preserved (normalize ≠ validate).
7. Hidden parameters do NOT get defaults backfilled.

---

## ParameterValues

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ParameterValues {
    #[serde(flatten)]
    values: HashMap<String, serde_json::Value>,
}
```

Features:
- Typed accessors: `get_string`, `get_i64`, `get_f64`, `get_bool`, `get_array`
- Expression values: `{ "$expr": "{{ $input.value }}" }`
- Mode values: `{ "mode": "bearer", "value": { "token": "abc" } }`
- Snapshot / restore / diff

**ParameterValues are always raw.** Validation is a check, not a
transformation. There is no `ValidatedValues` wrapper — consumers treat
values as raw and call `validate()` explicitly when needed.

---

## Lint

```rust
pub fn lint(collection: &ParameterCollection) -> Vec<LintDiagnostic>;
```

Static schema diagnostics (no runtime values needed):

- Duplicate parameter IDs within same scope
- Empty parameter IDs
- Contradictory rules (min_length > max_length)
- Contradictory min_items > max_items
- Duplicate mode variant keys
- Invalid default_variant (references non-existent variant)
- `depends_on` references to non-existent parameters (validated via ParameterPath)
- Circular `visible_when`/`required_when`/`depends_on` references (cycle detection)
- `$root.x` references to non-existent root-level parameters

---

## Integration

### nebula-action

```rust
pub struct ActionMetadata {
    pub key: String,
    pub name: String,
    pub parameters: ParameterCollection,
    // ...
}
```

### nebula-credential

```rust
pub struct CredentialTypeSchema {
    pub type_id: String,
    pub display_name: String,
    pub params: ParameterCollection,
    // ...
}
```

### nebula-resource

```rust
pub trait ResourceDescriptor {
    fn config_schema(&self) -> ParameterCollection;
    // ...
}
```

---

## DX Guidelines

### .active_when() vs separate visible/required

Use `.active_when(condition)` when field should appear and be required
together (80% of cases). Use separate `.visible_when()` / `.required_when()`
when they differ — e.g. WHERE clause visible for update+delete, but required
only for delete.

```rust
// Common — same condition for both
.active_when(Condition::eq("operation", json!("sendPhoto")))

// Uncommon — different conditions
.visible_when(Condition::any(vec![
    Condition::eq("operation", json!("update")),
    Condition::eq("operation", json!("delete")),
]))
.required_when(Condition::eq("operation", json!("delete")))
```

### Reusable parameter templates

When the same structure appears multiple times (key-value pairs, email
inputs), extract into Rust functions:

```rust
fn kv_item(id: &str) -> Parameter {
    Parameter::object(id)
        .add(Parameter::string("key").label("Key").required())
        .add(Parameter::string("value").label("Value").required())
}

fn email_input(id: &str) -> Parameter {
    Parameter::string(id)
        .input_type("email")
        .with_rule(Rule::Pattern {
            pattern: r"^[^@]+@[^@]+\.[^@]+$".to_owned(),
            message: Some("must be a valid email".to_owned()),
        })
}

// Usage
.add(Parameter::list("headers").item(kv_item("header")).sortable())
.add(Parameter::list("query_params").item(kv_item("param")).sortable())
.add(Parameter::list("to").item(email_input("email")).min_items(1).unique())
```

### Dedicated type vs String + input_type

When a dedicated `ParameterType` exists and produces the same value,
prefer the dedicated type for semantic clarity:

```rust
// Prefer:
Parameter::color("bg_color").label("Background")
// Over:
Parameter::string("bg_color").label("Background").input_type("color")

// Prefer:
Parameter::date("start_date").label("Start Date")
// Over:
Parameter::string("start_date").label("Start Date").input_type("date")
```

`input_type` is for HTML override within a type (e.g. String → "email",
"tel", "url", "password", "search"; Number → "range").

### .secret() vs .input_type("password")

Independent concerns:
- `.secret()` = backend: exclude from logs, mask in debug output, encrypt at rest.
- `.input_type("password")` = frontend: render masked dots.

Usually set together, but can diverge. Example: API key displayed in
plaintext but `.secret()` excludes it from logs.

---

## Module layout

```
nebula-parameter/src/
├── lib.rs               // pub use, prelude
├── parameter.rs         // Parameter struct, fluent builders
├── parameter_type.rs    // ParameterType enum
├── collection.rs        // ParameterCollection
├── select.rs            // SelectOption
├── condition.rs         // Condition enum
├── path.rs              // ParameterPath
├── variant.rs           // ModeVariant, ComputedReturn
├── values.rs            // ParameterValues
├── error.rs             // ParameterError, LoaderError
├── validate.rs          // Validation engine
├── normalize.rs         // Default backfilling
├── lint.rs              // Static schema diagnostics
├── loader.rs            // OptionLoader, RecordLoader, LoaderContext
├── filter.rs            // FilterOp, FilterExpr, FilterRule, FilterGroup
├── report.rs            // ValidationReport
├── profile.rs           // ValidationProfile
└── prelude.rs           // Common imports
```

---

## Migration from v1

| v1 (current) | v2 (new) |
|--------------|----------|
| `Field` enum (16 variants) | `Parameter` struct + `ParameterType` enum |
| `Schema` | `ParameterCollection` |
| `FieldMetadata` (separate struct) | Fields on `Parameter` struct |
| `FieldValues` | `ParameterValues` |
| `Condition` (alias for `Rule`) | `Condition` (dedicated enum) |
| `Field::text("id")` | `Parameter::string("id")` |
| `.with_label("L")` | `.label("L")` |
| `OptionLoader` (closure) | `OptionLoader` (closure, same + `Result`) |
| N/A | `RecordLoader` for Dynamic |
| N/A | `Computed` parameter type |
| N/A | `input_type` field |
| N/A | `ParameterPath` (typed field references) |
| N/A | `.active_when()` shorthand |
| N/A | `LoaderContext.metadata` for engine context |

### Wire format compatibility

```json
// v1: { "type": "text", "id": "name", ... }
// v2: { "type": "string", "id": "name", ... }
```

Compatibility shim can accept both `"text"` and `"string"` during migration.

---

## Implementation phases

| Phase | Content | Estimate |
|-------|---------|----------|
| 1 | Core types: Parameter, ParameterType, ParameterCollection, ParameterValues, serde | 1 week |
| 2 | Fluent builders, Condition, ModeVariant, SelectOption | 1 week |
| 3 | Validation engine, normalization, lint | 1 week |
| 4 | OptionLoader, RecordLoader, LoaderContext | 3-4 days |
| 5 | Integration: update nebula-action, nebula-credential, nebula-resource | 1 week |
| 6 | Migration shim for v1 wire format, contract tests | 3-4 days |

**Total: ~5-6 weeks.**

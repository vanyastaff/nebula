# nebula-parameter — High-Level Design (v3)

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
| Value transformation | nebula-parameter | `Transformer` pipeline, `get_transformed()` |
| Expression evaluation | nebula-engine (runtime) | Stores `$expr` marker, engine resolves |
| Dynamic loading | nebula-parameter defines contract | `OptionLoader` / `RecordLoader` / `FilterFieldLoader` |
| UI rendering | frontend (React/Vue/CLI) | Consumes JSON schema |
| Persistence | nebula-engine / nebula-credential | Stores `ParameterValues` as JSON |

### Portable schema vs runtime-attached schema

nebula-parameter produces two layers of schema:

**Portable schema** — everything that serializes to JSON. This is what gets
stored in database, sent to frontend, compared for equality, cached, and
transported across process boundaries. It is the contract.

**Runtime-attached schema** — portable schema plus non-serializable closures
(`OptionLoader`, `RecordLoader`, `FilterFieldLoader`). These exist only in the
Rust process that defined them. They are `#[serde(skip)]` and excluded from
equality/hashing.

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
| `Object` | `{ "host": "...", "port": 5432 }` | Nested field group, pick-fields, collapsible section |
| `List` | `[item1, item2, ...]` | Repeatable items |
| `Mode` | `{ "mode": "bearer", "value": "..." }` | Discriminated union |
| `Code` | `"SELECT * FROM ..."` | Syntax-highlighted editor |
| `Date` | `"2025-01-15"` | Date picker |
| `DateTime` | `"2025-01-15T10:30:00Z"` | Date+time picker |
| `Time` | `"14:30"` | Time picker |
| `Color` | `"#ff6600"` | Color picker |
| `File` | file reference | File upload |
| `Hidden` | any JSON value | Not displayed, stored |
| `Filter` | predicate expression | Visual condition builder with typed fields |
| `Computed` | derived from siblings | Read-only calculated field |
| `Dynamic` | resolved at runtime | Provider-driven fields |
| `Notice` | — (display-only) | Info/warning/danger block in form |

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
        Ok(LoaderResult::done(tables.into_iter()
            .map(|t| SelectOption::new(json!(t), t))
            .collect()))
    })

// Select — options with icons
Parameter::select("provider")
    .label("Provider")
    .option_with(SelectOption::new(json!("openai"), "OpenAI")
        .icon("openai").description("GPT-4o, GPT-4"))
    .option_with(SelectOption::new(json!("anthropic"), "Anthropic")
        .icon("anthropic").description("Claude 4, Claude 3.5"))

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

// Transformers — declarative value transforms
Parameter::string("email")
    .label("Email")
    .trim()
    .lowercase()

Parameter::string("video")
    .label("YouTube Video")
    .extract_regex(r"(?:v=|youtu\.be/)([a-zA-Z0-9_-]{11})", 1)

// computed — read-only derived fields
Parameter::computed("connection_string")
    .label("Connection String")
    .returns_string()
    .expression("postgres://{{ host }}:{{ port }}/{{ database }}")

// notice — display-only informational blocks
Parameter::warning("cred_notice")
    .label("Credentials Required")
    .description("Configure OAuth2 credentials before using this node.")
    .visible_when(Condition::not_set("credential_id"))
```

### Nested structures

**Object** — nested field group with display modes:

```rust
// Inline (default) — all fields always visible
Parameter::object("auth")
    .label("Authentication")
    .add(Parameter::string("username").label("Username").required())
    .add(Parameter::string("password").label("Password").secret().required())

// Collapsed — collapsible section
Parameter::object("advanced")
    .label("Advanced Settings")
    .collapsed()
    .add(Parameter::integer("timeout_ms").label("Timeout (ms)").default(json!(5000)))
    .add(Parameter::boolean("debug").label("Debug Mode").default(json!(false)))

// PickFields — "Add Field" dropdown, only added fields in values
Parameter::object("options")
    .label("Additional Fields")
    .pick_fields()
    .add(Parameter::integer("timeout_ms").label("Timeout (ms)").default(json!(30000)))
    .add(Parameter::string("proxy_url").label("Proxy URL").input_type("url"))
    .add(Parameter::boolean("ignore_ssl").label("Ignore SSL").default(json!(false)))

// Sections — grouped "Add Field" dropdown
Parameter::object("options")
    .label("Options")
    .sections()
    .add(Parameter::integer("timeout_ms").label("Timeout").group("Network"))
    .add(Parameter::string("proxy_url").label("Proxy").group("Network"))
    .add(Parameter::string("encoding").label("Encoding").group("Response"))
    .add(Parameter::integer("retry_count").label("Retries").group("Retry"))
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

**Mode** — discriminated union. Variants are Parameters directly:

```rust
Parameter::mode("auth_type")
    .label("Authentication")
    .variant(Parameter::hidden("none").label("None"))
    .variant(Parameter::string("bearer")
        .label("Bearer Token")
        .secret()
        .required())
    .variant(Parameter::object("oauth2")
        .label("OAuth2")
        .add(Parameter::string("client_id").label("Client ID").required())
        .add(Parameter::string("client_secret").label("Client Secret").secret().required())
        .add(Parameter::string("scope").label("Scope")))
    .default_variant("none")
```

Mode value shape:

```json
// Scalar variant:
{ "mode": "bearer", "value": "sk-abc123" }

// Object variant:
{ "mode": "oauth2", "value": { "client_id": "...", "client_secret": "..." } }

// Hidden/no-value variant:
{ "mode": "none" }
```

**Dynamic** — runtime-resolved fields:

```rust
Parameter::dynamic("row_data")
    .label("Row Data")
    .depends_on(&["sheet_id"])
    .loader(|ctx: LoaderContext| async move {
        let sheet = ctx.values.get_string("sheet_id").unwrap_or("");
        let columns = fetch_sheet_columns(sheet, &ctx.credential).await?;
        Ok(LoaderResult::done(columns.into_iter()
            .map(|col| Parameter::string(&col.id).label(&col.name))
            .collect()))
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

// Shorthand: both visible and required under same condition
Parameter::file("photo")
    .label("Photo")
    .active_when(Condition::eq("operation", json!("sendPhoto")))

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
        .active_when(Condition::eq("operation", json!("sendMessage"))))
    .add(Parameter::file("photo")
        .label("Photo")
        .accept("image/*")
        .active_when(Condition::eq("operation", json!("sendPhoto"))));
```

### Resource locator pattern

Not a dedicated type — expressed with Mode + Transformer:

```rust
Parameter::mode("spreadsheet")
    .label("Spreadsheet")
    .variant(
        Parameter::select("list")
            .label("From List")
            .searchable()
            .loader(|ctx| async move {
                let token = ctx.credential.as_ref()
                    .ok_or(LoaderError::new("No credential"))?;
                let sheets = google_drive::list_spreadsheets(token).await?;
                Ok(LoaderResult::done(sheets.into_iter()
                    .map(|s| SelectOption::new(json!(s.id), &s.name))
                    .collect()))
            }))
    .variant(
        Parameter::string("url")
            .label("By URL")
            .input_type("url")
            .placeholder("https://docs.google.com/spreadsheets/d/.../edit")
            .extract_regex(r"/spreadsheets/d/([a-zA-Z0-9_-]+)", 1))
    .variant(
        Parameter::string("id")
            .label("By ID")
            .placeholder("1BxiMVs0XRA5..."))
    .default_variant("list")
    .required()
```

Action code reads the extracted ID regardless of mode:
```rust
let spreadsheet_id = values.get_transformed("spreadsheet", &schema)?;
// → "ABC123" whether user picked from list, pasted URL, or typed ID
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
    .add(Parameter::integer("pool_size")
        .label("Pool Size")
        .default(json!(10))
        .min(1).max(100))
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

    /// Longer description / tooltip. Supports markdown.
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

    /// HTML input type override.
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

    /// Value transformers. Applied in order via get_transformed().
    /// Declarative pipeline: trim, lowercase, regex extract, etc.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transformers: Vec<Transformer>,

    /// Logical group name for Sections display mode.
    /// Frontend uses this to organize the "Add Field" dropdown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

impl Parameter {
    /// Shorthand: sets both `visible_when` and `required_when` to the same condition.
    pub fn active_when(mut self, condition: Condition) -> Self {
        self.visible_when = Some(condition.clone());
        self.required_when = Some(condition);
        self
    }
}
```

**Why struct + ParameterType, not an enum:**
- Shared fields (label, required, rules, conditions) — one implementation.
- Type-specific fields (multiline, min/max, options) — inside ParameterType.
- Adding a new type: one variant + serde tag. Zero changes to shared metadata path.

**Constructor inventory:**

```rust
Parameter::string(id)       // String { multiline: false }
Parameter::number(id)       // Number { integer: false, ... }
Parameter::integer(id)      // Number { integer: true, ... }
Parameter::boolean(id)      // Boolean
Parameter::select(id)       // Select { options: [], ... }
Parameter::object(id)       // Object { parameters: [], display_mode: Inline }
Parameter::list(id)         // List { item, ... }
Parameter::mode(id)         // Mode { variants: [], ... }
Parameter::code(id)         // Code { language: "" }
Parameter::date(id)         // Date
Parameter::datetime(id)     // DateTime
Parameter::time(id)         // Time
Parameter::color(id)        // Color
Parameter::file(id)         // File { accept: None, ... }
Parameter::hidden(id)       // Hidden
Parameter::filter(id)       // Filter { ... }
Parameter::computed(id)     // Computed { expression: "", returns: String }
Parameter::dynamic(id)      // Dynamic { depends_on: [], loader: None }
Parameter::notice(id)       // Notice { severity: Info }
Parameter::warning(id)      // Notice { severity: Warning }
Parameter::danger(id)       // Notice { severity: Danger }
Parameter::success(id)      // Notice { severity: Success }
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
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        options: Vec<SelectOption>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        dynamic: bool,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        depends_on: Vec<ParameterPath>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        multiple: bool,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        allow_custom: bool,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        searchable: bool,
        #[serde(skip)]
        loader: Option<OptionLoader>,
    },

    /// Nested object with ordered sub-parameters and display mode.
    Object {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        parameters: Vec<Parameter>,
        /// Controls UI presentation and normalization/validation behavior.
        #[serde(default, skip_serializing_if = "DisplayMode::is_default")]
        display_mode: DisplayMode,
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

    /// Discriminated union. Variants are Parameters directly.
    /// Parameter.id = variant key, Parameter.label = switcher label.
    Mode {
        variants: Vec<Parameter>,
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

    /// Visual filter / condition builder with typed field definitions.
    Filter {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        operators: Option<Vec<FilterOp>>,
        #[serde(default)]
        allow_groups: bool,
        #[serde(default = "default_depth")]
        max_depth: u8,
        /// Static filterable fields.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        fields: Vec<FilterField>,
        /// When true, fields are loaded at runtime via FilterFieldLoader.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        dynamic_fields: bool,
        /// Re-load fields when these parameter values change.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        depends_on: Vec<ParameterPath>,
        /// Runtime field loader. Not serialized.
        #[serde(skip)]
        fields_loader: Option<FilterFieldLoader>,
    },

    /// Computed field — read-only, value derived from siblings.
    Computed {
        expression: String,
        returns: ComputedReturn,
    },

    /// Dynamic fields resolved at runtime by loader.
    Dynamic {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        depends_on: Vec<ParameterPath>,
        #[serde(skip)]
        loader: Option<RecordLoader>,
    },

    /// Display-only informational block. No user input, no stored value.
    Notice {
        #[serde(default, skip_serializing_if = "NoticeSeverity::is_default")]
        severity: NoticeSeverity,
    },
}
```

### Supporting types

```rust
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayMode {
    /// All sub-parameters rendered inline, always visible.
    /// Normalize: backfills defaults for all sub-parameters.
    #[default]
    Inline,
    /// Collapsible section with expand/collapse toggle.
    /// Normalize: backfills defaults for all sub-parameters.
    Collapsed,
    /// "Add Field" dropdown. Only added fields present in values.
    /// Normalize: does NOT backfill defaults for unadded fields.
    PickFields,
    /// Like PickFields, but dropdown grouped by Parameter.group.
    /// Normalize: same as PickFields.
    Sections,
}

impl DisplayMode {
    pub fn is_default(&self) -> bool { matches!(self, Self::Inline) }
    pub fn is_pick_mode(&self) -> bool { matches!(self, Self::PickFields | Self::Sections) }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeSeverity {
    #[default]
    Info,
    Warning,
    Success,
    Danger,
}

impl NoticeSeverity {
    pub fn is_default(&self) -> bool { matches!(self, Self::Info) }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputedReturn {
    String,
    Number,
    Boolean,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectOption {
    pub value: serde_json::Value,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
    /// Icon key, URI, or emoji. Frontend-only rendering hint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ParameterCollection {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<Parameter>,
}
```

### FilterField and FilterFieldType

```rust
/// Describes a filterable field for the Filter condition builder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FilterField {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "FilterFieldType::is_default")]
    pub field_type: FilterFieldType,
}

/// Data type of a filterable field. Determines applicable operators
/// and value input widget.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterFieldType {
    #[default]
    String,
    Number,
    Boolean,
    Date,
    DateTime,
    Enum { options: Vec<SelectOption> },
}
```

### Transformer

```rust
/// Declarative value transformation pipeline.
/// Applied lazily when action reads values via get_transformed().
/// If a transformer fails to match, value passes through unchanged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Transformer {
    Trim,
    Lowercase,
    Uppercase,
    Replace { from: String, to: String },
    StripPrefix { prefix: String },
    StripSuffix { suffix: String },
    /// Extract regex capture group. No match → pass-through.
    Regex {
        pattern: String,
        #[serde(default = "default_group")]
        group: usize,
    },
    /// Extract value at JSON dot-path.
    JsonPath { path: String },
    /// Apply transformers in sequence.
    Chain { transformers: Vec<Transformer> },
    /// First transformer that changes the value wins.
    FirstMatch { transformers: Vec<Transformer> },
}

fn default_group() -> usize { 1 }
```

### ParameterPath

```rust
/// Typed reference to a parameter within a schema.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ParameterPath(String);

impl ParameterPath {
    pub fn sibling(id: &str) -> Self { Self(id.to_owned()) }
    pub fn nested(path: &str) -> Self { Self(path.to_owned()) }
    pub fn root(id: &str) -> Self { Self(format!("$root.{id}")) }
    pub fn is_absolute(&self) -> bool { self.0.starts_with("$root.") }
    pub fn segments(&self) -> Vec<&str> {
        let s = self.0.strip_prefix("$root.").unwrap_or(&self.0);
        s.split('.').collect()
    }
}

impl From<&str> for ParameterPath {
    fn from(s: &str) -> Self { Self(s.to_owned()) }
}
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

### LoaderResult

```rust
/// Paginated result from any loader.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoaderResult<T> {
    pub items: Vec<T>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
}

impl<T> LoaderResult<T> {
    pub fn done(items: Vec<T>) -> Self {
        Self { items, next_cursor: None, total: None }
    }
    pub fn page(items: Vec<T>, next_cursor: impl Into<String>) -> Self {
        Self { items, next_cursor: Some(next_cursor.into()), total: None }
    }
    pub fn with_total(mut self, total: u64) -> Self {
        self.total = Some(total); self
    }
    pub fn has_more(&self) -> bool { self.next_cursor.is_some() }
}

/// Existing loaders returning Vec<T> auto-convert.
impl<T> From<Vec<T>> for LoaderResult<T> {
    fn from(items: Vec<T>) -> Self { Self::done(items) }
}
```

---

## Dynamic Loading

### OptionLoader, RecordLoader, FilterFieldLoader

All are type-erased async closures returning `LoaderResult<T>`.
Calling `.loader()` automatically sets `dynamic = true`.
Frontend sees `"dynamic": true` in JSON, requests options by parameter id.

```rust
pub struct LoaderContext {
    pub parameter_id: String,
    pub values: ParameterValues,
    pub filter: Option<String>,
    pub cursor: Option<String>,
    pub credential: Option<serde_json::Value>,
    pub metadata: Option<serde_json::Value>,
}

pub struct OptionLoader(
    Arc<dyn Fn(LoaderContext) -> Pin<Box<dyn Future<
        Output = Result<LoaderResult<SelectOption>, LoaderError>
    > + Send>> + Send + Sync>
);

pub struct RecordLoader(
    Arc<dyn Fn(LoaderContext) -> Pin<Box<dyn Future<
        Output = Result<LoaderResult<Parameter>, LoaderError>
    > + Send>> + Send + Sync>
);

pub struct FilterFieldLoader(
    Arc<dyn Fn(LoaderContext) -> Pin<Box<dyn Future<
        Output = Result<LoaderResult<FilterField>, LoaderError>
    > + Send>> + Send + Sync>
);
```

Key design points:
- Closure-based — no trait to implement.
- `Result` return — I/O errors surfaced, not hidden.
- `LoaderResult` — pagination via `next_cursor`, total count via `total`.
- `#[serde(skip)]` — loaders exist only in-process.
- `PartialEq` always `true` — loaders excluded from schema comparison.

---

## Serialization

### Wire format

Flat JSON with `"type"` discriminator via `#[serde(flatten)]` on `param_type`:

```json
{ "id": "chat_id", "type": "string", "label": "Chat ID", "required": true }
```

```json
{
  "id": "method", "type": "select", "label": "HTTP Method",
  "options": [
    { "value": "GET", "label": "GET" },
    { "value": "POST", "label": "POST" }
  ],
  "default": "GET"
}
```

```json
{
  "id": "auth_type", "type": "mode", "label": "Authentication",
  "default_variant": "none",
  "variants": [
    { "id": "none", "type": "hidden", "label": "None" },
    { "id": "bearer", "type": "string", "label": "Bearer Token", "secret": true, "required": true }
  ]
}
```

```json
{
  "id": "options", "type": "object", "label": "Additional Fields",
  "display_mode": "pick_fields",
  "parameters": [
    { "id": "timeout_ms", "type": "number", "integer": true, "default": 30000 },
    { "id": "proxy_url", "type": "string", "input_type": "url" }
  ]
}
```

```json
{
  "id": "email", "type": "string", "label": "Email",
  "transformers": [ { "type": "trim" }, { "type": "lowercase" } ]
}
```

```json
{
  "id": "delete_warning", "type": "notice", "severity": "danger",
  "description": "This will permanently delete matching rows.",
  "visible_when": { "op": "eq", "field": "operation", "value": "delete" }
}
```

```json
{
  "id": "conditions", "type": "filter", "label": "Filter",
  "fields": [
    { "id": "subject", "label": "Subject" },
    { "id": "date", "label": "Date", "field_type": "date" },
    { "id": "status", "label": "Status", "field_type": { "enum": { "options": [
      { "value": "active", "label": "Active" }
    ]}}}
  ],
  "allow_groups": true, "max_depth": 3
}
```

### Round-trip guarantee

```rust
let param = Parameter::string("name").label("Name").required();
let json = serde_json::to_value(&param).unwrap();
let back: Parameter = serde_json::from_value(json).unwrap();
assert_eq!(param, back);
```

Applies to portable schema only. Loaders are `#[serde(skip)]`.

---

## Validation

### API

```rust
impl ParameterCollection {
    pub fn validate(&self, values: &ParameterValues)
        -> Result<(), Vec<ParameterError>>;

    pub fn validate_with_profile(
        &self, values: &ParameterValues, profile: ValidationProfile
    ) -> ValidationReport;
}

pub enum ValidationProfile {
    Strict,      // unknown fields → error
    Warn,        // unknown fields → warning
    Permissive,  // unknown fields → silent
}
```

### Validation flow

```
values arrive (ParameterValues)
       │
for each Parameter in schema:
       ├─ type == Computed? → skip
       ├─ type == Notice? → skip
       ├─ visible_when → hidden + no value? skip
       ├─ Object with PickFields/Sections? →
       │    key absent from values? skip entirely
       ├─ required / required_when → missing? error
       ├─ expression check: value is { "$expr": "..." }?
       │    → yes: validate wrapper shape only
       │    → no: continue
       ├─ type check (string is string, number is number)
       ├─ type-specific:
       │    Select: value in options (unless allow_custom)?
       │    Number: min/max/integer?
       │    Object: recurse into sub-parameters
       │    List: min_items/max_items + unique + recurse items
       │    Mode: valid variant? recurse variant content
       │    Filter: field exists? operator applicable? value type?
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
| Hidden + present | Skip validation | Preserve value | Include |
| Disabled + present | Full validation | Preserve value | Include |
| Disabled + missing | Required check | Apply default | Include default or absent |
| Computed | Skip all | Runtime evaluates | Runtime includes result |
| Notice | Skip all | Skip (no value) | Exclude |
| PickFields absent | Skip all | Skip default | Exclude |

### Mode validation (updated)

1. `"mode"` key must match a variant Parameter.id or default_variant.
2. Only the selected variant is validated.
3. For scalar variants: validate `"value"` as the variant Parameter type.
4. For Object variants: recurse into `"value"` sub-parameters.
5. `required` on a variant Parameter means "required when variant is active."
6. Inactive variant values are NOT validated.

### Filter validation (enhanced)

With field definitions available:
1. `rule.field` references a known field id → unknown field = warning.
2. `rule.op` is applicable to the field's FilterFieldType → mismatch = error.
3. `rule.value` type matches field type → mismatch = error.
4. Nesting depth ≤ max_depth.
5. Groups only present if allow_groups is true.

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
3. Parameter type is `Notice` → skip (no value).
4. Mode with `default_variant` and value absent → inject `{ "mode": "<id>" }`.
5. Mode with selected variant:
   - Scalar variant → normalize `"value"` per variant Parameter rules.
   - Object variant → recurse into `"value"` for nested defaults.
   - Hidden variant → no `"value"` key needed.
6. Existing user values NEVER overwritten.
7. Extra keys not in schema are preserved (normalize ≠ validate).
8. Hidden parameters do NOT get defaults backfilled.
9. Object with PickFields or Sections display_mode:
   - If Object key absent → set to `{}`.
   - Key present in values → apply default/recurse (standard).
   - Key absent from values → SKIP. Do NOT backfill default.

---

## Transformer Application

Transformers do NOT affect validation or normalization. They are applied
lazily when action code reads values via `get_transformed()`.

```rust
impl ParameterValues {
    /// Get raw value. Existing behavior, unchanged.
    pub fn get(&self, key: &str) -> Option<&Value>;

    /// Get value with schema transformers applied.
    pub fn get_transformed(
        &self,
        key: &str,
        schema: &ParameterCollection,
    ) -> Option<Value>;
}
```

For Mode parameters, `get_transformed()` applies the active variant's
content Parameter transformers, then the parent Mode Parameter transformers.

Stored values are always raw. Transform is a read-time convenience.

---

## Expression Scope Rules

```
1. Bare name → sibling within same scope.
2. Inside Object → siblings are other object fields.
3. Inside List item → siblings are current item's fields.
4. Inside Mode variant → scope is active variant content.
5. $root.field → root ParameterCollection. Absolute reference.
6. $item.index → current list item index (0-based).
```

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
- Transformed accessors: `get_transformed(key, schema)` — applies Transformer pipeline
- Expression values: `{ "$expr": "{{ $input.value }}" }`
- Mode values: `{ "mode": "bearer", "value": "abc" }`
- Snapshot / restore / diff

**ParameterValues are always raw.** Validation is a check, not a
transformation. Transformers are applied on-read via `get_transformed()`.

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
- Duplicate mode variant IDs
- Invalid default_variant (references non-existent variant)
- `depends_on` references to non-existent parameters
- Circular `visible_when`/`required_when`/`depends_on` references
- `$root.x` references to non-existent root-level parameters
- Mode variant missing label
- Sections Object with sub-parameters missing `group`
- `group` on parameter inside non-Sections Object
- `required` on sub-parameter of PickFields Object (unusual)
- PickFields/Sections Object with 0 or ≤2 sub-parameters
- Transformer on non-string parameter (Trim, Lowercase, Uppercase)
- Invalid regex pattern in Transformer::Regex
- Regex capture group 0 (likely unintended)
- Chain/FirstMatch with single transformer (unnecessary)
- Notice with `required`, `secret`, `default`, or `rules` set
- Notice without `description`
- Filter with no static fields and no fields_loader
- Filter with duplicate field IDs
- Filter global operator not applicable to any defined field type

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
when they differ.

### DisplayMode selection

| Situation | DisplayMode |
|-----------|-------------|
| 2-5 required fields | Inline (default) |
| 3-8 optional fields with defaults | Collapsed |
| 5-15 optional fields, user configures 0-3 | PickFields |
| 10-25 optional fields across categories | Sections |

### Reusable parameter templates

Extract repeated structures into Rust functions:

```rust
fn kv_item(id: &str) -> Parameter {
    Parameter::object(id)
        .add(Parameter::string("key").label("Key").required())
        .add(Parameter::string("value").label("Value").required())
}

fn email_input(id: &str) -> Parameter {
    Parameter::string(id)
        .input_type("email")
        .trim()
        .lowercase()
        .with_rule(Rule::Pattern {
            pattern: r"^[^@]+@[^@]+\.[^@]+$".to_owned(),
            message: Some("must be a valid email".to_owned()),
        })
}
```

### Dedicated type vs String + input_type

When a dedicated `ParameterType` exists, prefer it for semantic clarity:

```rust
// Prefer:
Parameter::color("bg_color").label("Background")
// Over:
Parameter::string("bg_color").label("Background").input_type("color")
```

### .secret() vs .input_type("password")

Independent concerns:
- `.secret()` = backend: exclude from logs, mask in debug, encrypt at rest.
- `.input_type("password")` = frontend: render masked dots.

### Transformers for common patterns

```rust
// Email normalization
Parameter::string("email").trim().lowercase()

// URL → ID extraction
Parameter::string("video").extract_regex(r"(?:v=|youtu\.be/)([a-zA-Z0-9_-]{11})", 1)

// Slug generation
Parameter::string("slug").transformer(Transformer::Chain {
    transformers: vec![Transformer::Trim, Transformer::Lowercase,
        Transformer::Replace { from: " ".into(), to: "-".into() }]
})
```

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
├── display_mode.rs      // DisplayMode enum
├── transformer.rs       // Transformer enum
├── notice.rs            // NoticeSeverity
├── filter_field.rs      // FilterField, FilterFieldType
├── loader_result.rs     // LoaderResult<T>
├── values.rs            // ParameterValues
├── error.rs             // ParameterError, LoaderError
├── validate.rs          // Validation engine
├── normalize.rs         // Default backfilling
├── lint.rs              // Static schema diagnostics
├── loader.rs            // OptionLoader, RecordLoader, FilterFieldLoader, LoaderContext
├── filter.rs            // FilterOp, FilterExpr, FilterRule, FilterGroup
├── report.rs            // ValidationReport
├── profile.rs           // ValidationProfile
└── prelude.rs           // Common imports
```

---

## Migration from v1

| v1 (current) | v3 (new) |
|--------------|----------|
| `Field` enum (16 variants) | `Parameter` struct + `ParameterType` enum (19 variants) |
| `Schema` | `ParameterCollection` |
| `FieldMetadata` (separate struct) | Fields on `Parameter` struct |
| `FieldValues` | `ParameterValues` |
| `Condition` (alias for `Rule`) | `Condition` (dedicated enum) |
| `Field::text("id")` | `Parameter::string("id")` |
| `.with_label("L")` | `.label("L")` |
| `OptionLoader` → `Vec` | `OptionLoader` → `LoaderResult` |
| N/A | `RecordLoader` for Dynamic |
| N/A | `FilterFieldLoader` for Filter |
| N/A | `Computed` parameter type |
| N/A | `Notice` parameter type |
| N/A | `Transformer` pipeline |
| N/A | `DisplayMode` (PickFields, Sections) |
| N/A | `FilterField` / `FilterFieldType` |
| N/A | `LoaderResult<T>` pagination |
| `ModeVariant` wrapper | Removed — variants are `Vec<Parameter>` |
| `Object { collapsed: bool }` | `Object { display_mode: DisplayMode }` |
| Mode value `{ "mode": "k", "value": { "content_id": v } }` | `{ "mode": "k", "value": v }` |

### Wire format compatibility

```json
// v1: { "type": "text", "id": "name", ... }
// v3: { "type": "string", "id": "name", ... }
```

Compatibility shims:
- `"text"` ↔ `"string"` type alias
- `"collapsed": true` → `"display_mode": "collapsed"`
- Mode value unwrapping (single-key value object → flat value)

---

## Implementation phases

| Phase | Content | Estimate |
|-------|---------|----------|
| 1 | Core types: Parameter, ParameterType, ParameterCollection, ParameterValues, DisplayMode, Transformer, NoticeSeverity, FilterField, LoaderResult, serde | 1.5 weeks |
| 2 | Fluent builders, Condition, SelectOption (with icon), Mode (with Vec\<Parameter\>) | 1.5 weeks |
| 3 | Validation engine, normalization (PickFields rules, Notice skip, Filter field validation), lint (20 diagnostics) | 1.5 weeks |
| 4 | OptionLoader, RecordLoader, FilterFieldLoader, LoaderContext, LoaderResult integration | 4 days |
| 5 | Integration: update nebula-action, nebula-credential, nebula-resource for Mode value shape | 1 week |
| 6 | Migration shim for v1 wire format, Mode value migration, contract tests | 4 days |

**Total: ~7 weeks.**

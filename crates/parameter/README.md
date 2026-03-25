# nebula-parameter

Parameter schema system for the Nebula workflow engine. Defines typed, validatable parameter
definitions for actions, credentials, resources, and any configurable surface.

## What it does

Every configurable surface in Nebula needs the same thing: describe a set of typed fields with
labels, conditions, validation rules, and let something (frontend, CLI, OpenAPI generator) render
a form from it. `nebula-parameter` is that shared vocabulary.

| Consumer | Schema describes | User sees |
|----------|------------------|-----------|
| Action node | Node input parameters | Workflow editor settings panel |
| Credential | Auth setup (OAuth2, API key, DB) | "Add credential" dialog |
| Resource config | Operational settings (pool, timeouts) | Admin configuration page |
| Trigger | Event filter configuration | Trigger settings panel |

## Quick start

```rust
use nebula_parameter::prelude::*;
use serde_json::json;

// Define a schema
let params = ParameterCollection::new()
    .add(Parameter::string("chat_id").label("Chat ID").required())
    .add(Parameter::string("text").label("Text").multiline().required())
    .add(Parameter::integer("timeout_ms")
        .label("Timeout")
        .min(100.0).max(60_000.0)
        .default(json!(5000)));

// Validate user input
let mut values = ParameterValues::new();
values.set("chat_id", json!("123"));
values.set("text", json!("Hello!"));

let validated = params.validate(&values).expect("should pass");
```

## Core types

| Type | Purpose |
|------|---------|
| [`Parameter`] | Single parameter definition — struct with shared metadata + type-specific config |
| [`ParameterType`] | 19-variant enum: String, Number, Boolean, Select, Object, List, Mode, Code, Date, DateTime, Time, Color, File, Hidden, Filter, Computed, Dynamic, Notice |
| [`ParameterCollection`] | Ordered list of parameters with validate / normalize / lint |
| [`ParameterValues`] | Runtime key-value map with typed accessors |
| [`Condition`] | Predicate on sibling values (visibility, required) |
| [`Transformer`] | Declarative value transform pipeline (trim, lowercase, regex extract) |

## All 19 parameter types

### String

```rust
// Simple text input
Parameter::string("name").label("Name").required()

// Multi-line textarea
Parameter::string("body").label("Body").multiline()

// With input type hint for the frontend
Parameter::string("email").label("Email").input_type("email")
Parameter::string("token").label("Token").input_type("password").secret()
```

### Number

```rust
// Decimal number
Parameter::number("ratio").label("Ratio").min(0.0).max(1.0)

// Integer-only (rejects 3.5, accepts 3)
Parameter::integer("port").label("Port").min(1.0).max(65535.0)

// With step for slider/stepper UI
Parameter::number("opacity").label("Opacity").min(0.0).max(100.0).step(1.0)
```

### Boolean

```rust
Parameter::boolean("active").label("Active").default(json!(true))
```

### Select

```rust
// Static options
Parameter::select("method").label("HTTP Method")
    .option(json!("GET"), "GET")
    .option(json!("POST"), "POST")
    .option(json!("PUT"), "PUT")
    .default(json!("GET"))

// With icons and descriptions
Parameter::select("provider").label("Provider")
    .option_with(SelectOption::new(json!("openai"), "OpenAI")
        .icon("openai").description("GPT-4o, GPT-4"))
    .option_with(SelectOption::new(json!("anthropic"), "Anthropic")
        .icon("anthropic").description("Claude 4, Claude 3.5"))

// Multi-select with custom values
Parameter::select("tags").label("Tags")
    .multiple()
    .allow_custom()
    .option(json!("urgent"), "Urgent")
    .option(json!("bug"), "Bug")

// Dynamic options loaded at runtime
Parameter::select("table").label("Table")
    .depends_on(&["database"])
    .searchable()
    .with_option_loader(|ctx: LoaderContext| async move {
        let tables = fetch_tables(&ctx).await?;
        Ok(LoaderResult::done(tables.into_iter()
            .map(|t| SelectOption::new(json!(t), &t))
            .collect()))
    })
```

### Object

```rust
// Inline — all fields always visible (default)
Parameter::object("auth").label("Authentication")
    .add(Parameter::string("username").label("Username").required())
    .add(Parameter::string("password").label("Password").secret().required())

// Collapsed — collapsible section
Parameter::object("advanced").label("Advanced Settings")
    .collapsed()
    .add(Parameter::integer("timeout_ms").label("Timeout (ms)").default(json!(5000)))
    .add(Parameter::boolean("debug").label("Debug Mode").default(json!(false)))

// PickFields — "Add Field" dropdown, only added fields in values
Parameter::object("options").label("Additional Fields")
    .pick_fields()
    .add(Parameter::integer("timeout_ms").label("Timeout (ms)").default(json!(30000)))
    .add(Parameter::string("proxy_url").label("Proxy URL").input_type("url"))

// Sections — grouped "Add Field" dropdown
Parameter::object("options").label("Options")
    .sections()
    .add(Parameter::integer("timeout_ms").label("Timeout").group("Network"))
    .add(Parameter::string("proxy_url").label("Proxy").group("Network"))
    .add(Parameter::string("encoding").label("Encoding").group("Response"))
```

### List

```rust
// List of objects
Parameter::list("headers", Parameter::object("header")
        .add(Parameter::string("key").label("Key").required())
        .add(Parameter::string("value").label("Value").required()))
    .label("HTTP Headers")
    .min_items(0)
    .max_items(50)
    .sortable()

// Simple string list
Parameter::list("tags", Parameter::string("tag"))
    .label("Tags")
    .unique()
```

### Mode

Discriminated union — variants are Parameters. `param.id` = variant key.

```rust
Parameter::mode("auth_type").label("Authentication")
    .variant(Parameter::hidden("none").label("None"))
    .variant(Parameter::string("bearer").label("Bearer Token")
        .secret().required())
    .variant(Parameter::object("oauth2").label("OAuth2")
        .add(Parameter::string("client_id").label("Client ID").required())
        .add(Parameter::string("client_secret").label("Client Secret").secret().required())
        .add(Parameter::string("scope").label("Scope")))
    .default_variant("none")
```

Mode value shape:
```json
{ "mode": "bearer", "value": "sk-abc123" }
{ "mode": "oauth2", "value": { "client_id": "...", "client_secret": "..." } }
{ "mode": "none" }
```

### Code

```rust
Parameter::code("query", "sql").label("SQL Query")
Parameter::code("script", "python").label("Script")
```

### Date / DateTime / Time

```rust
Parameter::date("start_date").label("Start Date")           // "2025-01-15"
Parameter::datetime("scheduled_at").label("Scheduled At")    // "2025-01-15T10:30:00Z"
Parameter::time("reminder_time").label("Reminder Time")      // "14:30"
```

### Color

```rust
Parameter::color("bg_color").label("Background Color")       // "#ff6600"
```

### File

```rust
Parameter::file("attachment").label("Attachment")
    .accept("application/pdf")
    .max_size(10_485_760)   // 10 MB

Parameter::file("photos").label("Photos")
    .accept("image/*")
    .multiple()
```

### Hidden

Stored value with no visible editor. Useful as a Mode variant for "none" or for internal state.

```rust
Parameter::hidden("internal_id").default(json!("auto-generated"))
```

### Filter

Visual condition builder for filtering data.

```rust
Parameter::filter("conditions").label("Filter")
    .filter_field(FilterField { id: "subject".into(), label: "Subject".into(), field_type: FilterFieldType::String })
    .filter_field(FilterField { id: "date".into(), label: "Date".into(), field_type: FilterFieldType::Date })
    .allow_groups(true)
    .max_depth(3)
```

### Computed

Read-only field derived from siblings at runtime. Not editable, not validated.

```rust
Parameter::computed("connection_string")
    .label("Connection String")
    .returns_string()
```

### Dynamic

Fields resolved at runtime by a loader. The loader returns a list of Parameters.

```rust
Parameter::dynamic("row_data").label("Row Data")
    .depends_on(&["sheet_id"])
    .with_record_loader(|ctx: LoaderContext| async move {
        let columns = fetch_columns(&ctx).await?;
        Ok(LoaderResult::done(columns))
    })
```

### Notice

Display-only informational block. No user input, no stored value.

```rust
Parameter::notice("info").label("Note")
    .description("Configure OAuth2 credentials before using this node.")

Parameter::warning("cred_warning").label("Warning")
    .description("Credentials required.")
    .visible_when(Condition::not_set("credential_id"))

Parameter::danger("delete_warning")
    .description("This will permanently delete matching rows.")

Parameter::success("connected")
    .description("Connection verified successfully.")
```

## Conditional fields

```rust
// Show field only when another field has a specific value
Parameter::string("token").label("Token")
    .required()
    .visible_when(Condition::eq("auth_type", json!("bearer")))

// Shorthand: both visible and required under same condition
Parameter::file("photo").label("Photo")
    .active_when(Condition::eq("operation", json!("sendPhoto")))
```

## Transformers

```rust
// Trim whitespace + lowercase on read
Parameter::string("email").label("Email").trim().lowercase()

// Extract YouTube video ID from URL
Parameter::string("video").label("Video")
    .extract_regex(r"(?:v=|youtu\.be/)([a-zA-Z0-9_-]{11})", 1)
```

Transformers are applied lazily via `values.get_transformed("key", &params.parameters)` —
they do **not** affect validation or stored values.

## Validation

```rust
let report = params.validate_with_profile(&values, ValidationProfile::Strict);
// Strict: unknown fields are errors
// Warn: unknown fields are warnings
// Permissive: unknown fields silently ignored
```

Validation checks: required fields, type correctness, number min/max, select option membership,
nested Object/List/Mode recursion, and custom `Rule` constraints from `nebula-validator`.

## Normalization

```rust
let normalized = params.normalize(&values);
// Backfills defaults, mode default variants
// Respects DisplayMode: PickFields/Sections skip absent fields
```

## Lint

```rust
use nebula_parameter::lint::lint_collection;

let diagnostics = lint_collection(&params);
// 23 static diagnostics: duplicate IDs, dangling references,
// contradictory rules, transformer warnings, notice misuse, etc.
```

## Dynamic loading

Select options, dynamic fields, and filter fields can be loaded at runtime via async closures:

```rust
Parameter::select("table").label("Table")
    .depends_on(&["database"])
    .searchable()
    .with_option_loader(|ctx: LoaderContext| async move {
        let tables = fetch_tables(&ctx).await?;
        Ok(LoaderResult::done(tables.into_iter()
            .map(|t| SelectOption::new(json!(t), &t))
            .collect()))
    })
```

Loaders are `#[serde(skip)]` — they exist only in-process and are excluded from schema
equality, debug output, and serialization.

## Design principles

- **Serde-first** — every type round-trips through JSON. The wire format is the contract.
- **Headless** — zero UI dependencies. Schema + validation + normalization only.
- **Fluent builders** — common case is one line. Complex cases compose naturally.
- **Portable vs runtime** — portable schema (JSON-serializable) + runtime-attached
  closures (loaders). Schema equality = structural equality of portable schema only.

## Verify locally

```bash
cargo check -p nebula-parameter
cargo nextest run -p nebula-parameter
cargo clippy -p nebula-parameter -- -D warnings
cargo test --doc -p nebula-parameter
```

## License

Licensed under the same terms as the Nebula project.

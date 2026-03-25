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

## Parameter types at a glance

```rust
Parameter::string("name").label("Name").required()
Parameter::integer("port").label("Port").min(1.0).max(65535.0)
Parameter::boolean("active").label("Active").default(json!(true))

Parameter::select("method").label("HTTP Method")
    .option(json!("GET"), "GET")
    .option(json!("POST"), "POST")

Parameter::object("auth").label("Authentication")
    .add(Parameter::string("username").label("Username").required())
    .add(Parameter::string("password").label("Password").secret().required())

Parameter::mode("auth_type").label("Auth Type")
    .variant(Parameter::hidden("none").label("None"))
    .variant(Parameter::string("bearer").label("Bearer Token").secret().required())
    .default_variant("none")
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

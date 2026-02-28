# API

## Public Surface

- **Stable APIs:** `ParameterDef`, `ParameterKind`, `ParameterCapability`, `ParameterMetadata`, `ParameterCollection`, `ParameterValues`, `ValidationRule`, `ParameterDisplay`, `DisplayRuleSet`, `DisplayCondition`, `DisplayContext`, `ParameterError`, `SelectOption`, `OptionsSource`
- **Experimental APIs:** None
- **Hidden/internal APIs:** `ParameterSnapshot`, `ParameterDiff` (returned by public methods but not primary surface)

## Usage Patterns

- Build schema with `ParameterCollection::new().with(...)` or `.add(...)`
- Populate values with `ParameterValues::set(key, value)`
- Validate before execution: `collection.validate(&values)`
- Use `prelude::*` for common imports

## Minimal Example

```rust
use nebula_parameter::prelude::*;
use serde_json::json;

let col = ParameterCollection::new()
    .with(ParameterDef::Text(TextParameter::new("host", "Host")))
    .with(ParameterDef::Number(NumberParameter::new("port", "Port")
        .with_validation(ValidationRule::range(1.0, 65535.0))));

let mut values = ParameterValues::new();
values.set("host", json!("localhost"));
values.set("port", json!(5432));

match col.validate(&values) {
    Ok(()) => println!("valid"),
    Err(errors) => {
        for e in &errors {
            eprintln!("{} [{}]", e, e.code());
        }
    }
}
```

## Advanced Example

```rust
use nebula_parameter::prelude::*;
use nebula_parameter::display::DisplayRule;
use serde_json::json;

// Nested object with display rules
let mut cert_path = TextParameter::new("cert_path", "Certificate Path");
cert_path.display = Some(ParameterDisplay {
    show_when: vec![DisplayRuleSet::Single(DisplayRule {
        field: "tls".into(),
        condition: DisplayCondition::IsTrue,
    })],
    hide_when: vec![],
});

let mut host = TextParameter::new("host", "Host");
host.metadata.required = true;

let mut port = NumberParameter::new("port", "Port");
port.default = Some(5432.0);
port.validation = ValidationRule::range(1.0, 65535.0);

let connection = ObjectParameter::new("connection", "Connection")
    .with_field(ParameterDef::Text(host))
    .with_field(ParameterDef::Number(port))
    .with_field(ParameterDef::Checkbox(CheckboxParameter::new("tls", "Use TLS")))
    .with_field(ParameterDef::Text(cert_path));

let col = ParameterCollection::new()
    .with(ParameterDef::Object(connection));

// Values for nested object: key "connection" holds JSON object
let mut values = ParameterValues::new();
values.set("connection", json!({
    "host": "db.example.com",
    "port": 5432,
    "tls": true,
    "cert_path": "/path/to/cert.pem"
}));

// Diff between two value sets
let mut updated = ParameterValues::new();
updated.set("connection", json!({
    "host": "db.example.com",
    "port": 99999,
    "tls": true
}));
let diff = values.diff(&updated);
// diff.changed contains "connection"
```

## Error Semantics

- **Retryable errors:** None (`ParameterError::is_retryable()` always false)
- **Fatal errors:** All validation/type errors; caller must fix input
- **Validation errors:** `MissingValue`, `InvalidType`, `ValidationError` — use `category()` and `code()` for mapping

## Compatibility Rules

- **Major version bump:** Changes to `ParameterDef` enum variants, `ValidationRule` variants, `ParameterError` variants, removal of public APIs
- **Deprecation policy:** Deprecate in minor; remove in next major; minimum 6 months (see MIGRATION.md)

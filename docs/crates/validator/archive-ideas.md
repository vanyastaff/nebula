# Validator — Archived Design Ideas

Ideas that were sketched in early architecture docs but not yet implemented.
Kept here as reference for future development.

---

## `#[validate(...)]` Attribute Macro (Unimplemented)

Several early design documents described a planned `#[derive(Parameters)]` +
`#[validate(...)]` attribute system that would generate validation code at
compile time — similar to the `validator` crate in the broader ecosystem.

### Field-Level Attributes

```rust
#[derive(Parameters)]
struct ValidationExample {
    #[validate(min_length = 3, max_length = 50)]
    #[validate(regex = r"^[a-zA-Z0-9_]+$")]
    username: String,

    #[validate(email)]
    email: String,

    #[validate(url)]
    webhook_url: String,

    #[validate(range = 1..=100)]
    percentage: u8,

    #[validate(custom = "validate_api_key")]
    api_key: String,
}

fn validate_api_key(value: &str) -> Result<(), ValidationError> {
    if value.starts_with("sk_") && value.len() == 32 {
        Ok(())
    } else {
        Err(ValidationError::custom("Invalid API key format"))
    }
}
```

### Advanced Field Attributes (from architecture-v2)

```rust
#[derive(Parameters)]
struct AdvancedNodeParams {
    #[validate(regex = r"^[A-Z][A-Z0-9_]*$")]
    #[validate(custom = "validate_env_var_name")]
    env_var_name: String,

    #[validate(range = 1..=1000)]
    #[validate(multiple_of = 10)]   // not in current impl
    batch_size: u32,

    #[validate(url)]
    #[validate(custom = "validate_accessible_url")]
    webhook_url: String,

    #[validate(json_schema = "schemas/user.json")]  // not in current impl
    user_data: serde_json::Value,

    // Cross-field comparison by field name — referencing another field
    #[validate(greater_than = "start_date")]
    end_date: DateTime<Utc>,

    // Conditional required
    #[validate(required_if(field = "mode", value = "advanced"))]  // not in current impl
    advanced_options: Option<AdvancedOptions>,
}
```

### Cross-Field / Struct-Level Validation

```rust
#[derive(Parameters)]
#[validate(custom = "validate_dates")]   // struct-level custom validator
struct DateRangeParams {
    start_date: DateTime<Utc>,

    #[validate(greater_than = "start_date")]
    end_date: DateTime<Utc>,
}

fn validate_dates(params: &DateRangeParams) -> Result<(), ValidationError> {
    if params.end_date <= params.start_date {
        return Err(ValidationError::custom("End date must be after start date"));
    }
    Ok(())
}
```

### Notes on the Design

- `multiple_of` — divisibility constraint, not yet in the crate.
- `json_schema` — JSON Schema validation against an external file, not yet implemented.
- `required_if(field, value)` — conditional required based on another field's value;
  currently achievable via `ContextualValidator` but not as a derive attribute.
- `greater_than = "field_name"` — cross-field ordering; currently needs `ContextualValidator`.
- Old error variant `ValidationError::Custom("msg")` — replaced by `ValidationError::custom("msg")`.

---

## `ContextualValidator` — Older Signature

Early docs showed a version of `ContextualValidator` that took a `ParameterValue`
instead of a generic input type:

```rust
// Old design (architecture-v2, not implemented)
pub trait ContextualValidator {
    fn validate_with_context(
        &self,
        value: &ParameterValue,
        context: &ValidationContext,
    ) -> Result<(), ValidationError>;
}
```

The current implementation uses a generic associated type instead:

```rust
pub trait ContextualValidator {
    type Input: ?Sized;
    fn validate_with_context(
        &self,
        input: &Self::Input,
        ctx: &ValidationContext,
    ) -> Result<(), ValidationError>;
}
```

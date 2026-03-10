# Advanced Validation Patterns

Recipes for workflow authors, plugin developers, and SDK consumers.

---

## Table of Contents

1. [Self-Validating DTOs](#1-self-validating-dtos)
2. [Axum Validated Extraction](#2-axum-validated-extraction)
3. [Config Schema Validation](#3-config-schema-validation)
4. [Multi-Field Form Validation](#4-multi-field-form-validation)
5. [Conditional and Context-Dependent Chains](#5-conditional-and-context-dependent-chains)
6. [Type-Erased Validation](#6-type-erased-validation)
7. [Plugin-Authored Custom Validators](#7-plugin-authored-custom-validators)
8. [Proof Tokens (Validated\<T\>)](#8-proof-tokens)
9. [JSON Config Validation](#9-json-config-validation)
10. [Error Handling and Reporting](#10-error-handling-and-reporting)
11. [Caching Expensive Validators](#11-caching-expensive-validators)
12. [Collection Validation](#12-collection-validation)
13. [Anti-Patterns](#13-anti-patterns)

---

## 1. Self-Validating DTOs

**Use case**: Structs that know how to validate themselves.

Implement `Validate<Self>` so the struct can be passed directly to extraction layers
(like `ValidatedJson<T>`) or validated inline.

```rust
use nebula_validator::prelude::*;
use serde::Deserialize;

#[derive(Deserialize)]
struct CreateUser {
    username: String,
    email: String,
    age: u16,
}

impl Validate<Self> for CreateUser {
    fn validate(&self, _input: &Self) -> Result<(), ValidationError> {
        // Compose field validators with explicit field paths
        min_length(3).and(max_length(30))
            .validate(&self.username)
            .map_err(|e| e.with_field("/username"))?;

        email().validate(&self.email)
            .map_err(|e| e.with_field("/email"))?;

        in_range(13_u16, 120_u16).validate(&self.age)
            .map_err(|e| e.with_field("/age"))?;

        Ok(())
    }
}
```

**When to use**: REST API payloads, workflow node configs, any struct deserialized
from external input.

---

## 2. Axum Validated Extraction

**Use case**: API handlers that reject invalid JSON at the extractor level.

The `nebula-api` crate provides `ValidatedJson<T>` which calls `T::validate(&T)`
before the handler runs:

```rust
use nebula_api::extractors::ValidatedJson;

async fn create_user(ValidatedJson(user): ValidatedJson<CreateUser>) -> impl IntoResponse {
    // `user` is guaranteed valid here — no manual validation needed
    // ...
}
```

**Rule**: If `T: Validate<T>` fails, the extractor returns `400 Bad Request`
with a structured error body before the handler executes.

---

## 3. Config Schema Validation

**Use case**: Validating tenant/project/workflow configs against a schema.

The `nebula-config` crate applies validator functions based on schema constraints:

```rust
use nebula_validator::prelude::*;

fn validate_config_field(value: &str, schema: &FieldSchema) -> Result<(), ValidationError> {
    if let Some(min) = schema.min_length {
        min_length(min).validate(value)?;
    }
    if let Some(max) = schema.max_length {
        max_length(max).validate(value)?;
    }
    if let Some(ref pattern) = schema.pattern {
        matches_regex(pattern).validate(value)?;
    }
    if schema.format == Some("email") {
        email().validate(value)?;
    }
    Ok(())
}
```

**Key insight**: Validators are cheap value-types. Construct them on-the-fly from
schema metadata rather than storing them.

---

## 4. Multi-Field Form Validation

**Use case**: Validate all fields and collect all errors (not short-circuit).

```rust
use nebula_validator::prelude::*;

fn validate_registration(form: &RegistrationForm) -> Result<(), ValidationErrors> {
    let mut errors = ValidationErrors::new();

    if let Err(e) = min_length(3).and(max_length(20)).and(alphanumeric())
        .validate(&form.username) {
        errors.push(e.with_field("/username"));
    }
    if let Err(e) = email().validate(&form.email) {
        errors.push(e.with_field("/email"));
    }
    if let Err(e) = min_length(8).validate(&form.password) {
        errors.push(e.with_field("/password"));
    }

    errors.into_result()
}
```

**Key insight**: Use `ValidationErrors::new()` + `.push()` + `.into_result()` for
collect-all semantics. Use `.and()` chains for short-circuit within a single field.

---

## 5. Conditional and Context-Dependent Chains

**Use case**: Fields that are only required under certain conditions.

```rust
use nebula_validator::prelude::*;

// Validate only when the feature flag is active
let validated_port = in_range(1_u16, 65535_u16)
    .when(|port| *port != 0);  // 0 means "auto-assign"

// Validate only when a companion field has a certain value
fn validate_tls_config(config: &ServerConfig) -> Result<(), ValidationError> {
    if config.tls_enabled {
        min_length(1).validate(&config.cert_path)
            .map_err(|e| e.with_field("/cert_path"))?;
        min_length(1).validate(&config.key_path)
            .map_err(|e| e.with_field("/key_path"))?;
    }
    Ok(())
}
```

**Available conditionals**:
- `.when(|v| predicate)` — validate only when predicate is true
- `.unless(|v| predicate)` — validate only when predicate is false
- `.optional()` — `None` always passes, `Some(v)` is validated

---

## 6. Type-Erased Validation

**Use case**: Store heterogeneous validators in a collection or registry.

```rust
use nebula_validator::prelude::*;

// AnyValidator erases the concrete type
let validators: Vec<AnyValidator<str>> = vec![
    min_length(3).into_any(),
    max_length(100).into_any(),
    alphanumeric().into_any(),
];

for v in &validators {
    v.validate("hello123")?;
}
```

**When to use**: Plugin registries, dynamic validation pipelines, config-driven
validator selection. Incurs one virtual dispatch per call (trait object).

**For JSON values** — use `validate_any()` to bridge typed validators to
`serde_json::Value`:

```rust
let v = min_length(5);
v.validate_any(&json!("hello world"))?;  // extracts str from Value automatically
```

---

## 7. Plugin-Authored Custom Validators

**Use case**: Plugin/SDK authors creating domain-specific validators.

### Using the `validator!` macro (preferred)

```rust
use nebula_validator::prelude::*;

// Simple unit validator (no fields)
validator! {
    /// Checks that a credit card number passes the Luhn checksum.
    pub LuhnCheck for str;
    rule(input) {
        // ... Luhn algorithm ...
        luhn_valid(input)
    }
    error(_input) {
        ValidationError::new("luhn_check", "invalid credit card number")
    }
    fn luhn_check();
}

// Struct validator with configuration
validator! {
    /// Ensures a string matches a domain-specific format.
    pub DomainFormat { domain: String } for str;
    rule(self, input) {
        input.ends_with(&format!(".{}", self.domain))
    }
    error(self, _input) {
        ValidationError::invalid_format("", &format!("*.{}", self.domain))
    }
    fn domain_format(domain: String);
}

// Usage
let v = luhn_check().and(min_length(13));
let v = domain_format("example.com".into());
```

### Manual `Validate<T>` implementation

For validators that need more flexibility than the macro provides:

```rust
use nebula_validator::prelude::*;

struct UniqueInDatabase {
    pool: Arc<PgPool>,
}

impl Validate<str> for UniqueInDatabase {
    fn validate(&self, input: &str) -> Result<(), ValidationError> {
        // NOTE: sync-only trait — pre-fetch or use blocking calls
        // For async checks, validate before calling this
        if self.is_taken(input) {
            return Err(ValidationError::new("unique", "already taken")
                .with_field("/username"));
        }
        Ok(())
    }
}
```

**Important**: `Validate<T>` is **synchronous**. For async checks (database
lookups, HTTP calls), perform the check before calling `validate()` and pass the
result as context, or wrap in `tokio::task::block_in_place` for blocking calls.

---

## 8. Proof Tokens

**Use case**: Carry proof that a value has been validated through the type system.

```rust
use nebula_validator::prelude::*;

fn process_email(email: Validated<String>) {
    // `email` is guaranteed to have passed validation
    send_welcome_email(email.into_inner());
}

// Obtain a proof token via validate_into
let validator = email().and(max_length(254));
let proven: Validated<String> = validator.validate_into("user@example.com".to_string())?;
process_email(proven);
```

**Key properties**:
- `Validated<T>` is a zero-cost newtype wrapper
- Can only be constructed via `validate_into()` — no manual construction
- `.into_inner()` unwraps, `.as_ref()` borrows

**When to use**: Function signatures that should only accept validated data.
Turns runtime validation into a compile-time contract.

---

## 9. JSON Config Validation

**Use case**: Validate structured JSON documents (workflow definitions, node configs).

```rust
use nebula_validator::prelude::*;
use serde_json::json;

let config = json!({
    "server": {
        "host": "localhost",
        "port": 8080,
        "workers": 4
    },
    "database": {
        "url": "postgres://localhost/mydb",
        "pool_size": 10
    }
});

// Build field-level validators with RFC 6901 JSON Pointer paths
let validators: Vec<Box<dyn Validate<serde_json::Value>>> = vec![
    Box::new(json_field("/server/host", min_length(1))),
    Box::new(json_field("/server/port", in_range::<i64>(1, 65535))),
    Box::new(json_field("/server/workers", in_range::<i64>(1, 256))),
    Box::new(json_field("/database/url", min_length(10))),
    Box::new(json_field("/database/pool_size", in_range::<i64>(1, 100))),
];

// Collect all errors
let mut errors = ValidationErrors::new();
for v in &validators {
    if let Err(e) = v.validate(&config) {
        errors.push(e);
    }
}

if !errors.is_empty() {
    eprintln!("Config validation failed: {errors}");
}
```

**Optional fields**: Use `json_field_optional(path, validator)` — missing fields
produce `Ok(())` instead of an error.

---

## 10. Error Handling and Reporting

### Structured error serialization

```rust
use nebula_validator::prelude::*;
use serde_json::json;

let err = ValidationError::min_length("/username", 3, 1)
    .with_severity(ErrorSeverity::Error)
    .with_param("policy", "username-policy-v2");

// Serialize to JSON for API responses
let json = serde_json::to_value(&err).unwrap();
// {
//   "code": "min_length",
//   "message": "length must be at least 3, got 1",
//   "field": "/username",
//   "severity": "error",
//   "params": { "policy": "username-policy-v2" }
// }
```

### Error code matching

```rust
match err.code.as_ref() {
    "required" => "This field is required",
    "min_length" => "Too short",
    "max_length" => "Too long",
    "invalid_format" => "Invalid format",
    code => &format!("Validation failed: {code}"),
};
```

### Nested errors

```rust
let parent = ValidationError::new("form_invalid", "form validation failed")
    .with_nested(vec![
        ValidationError::required("/email"),
        ValidationError::min_length("/password", 8, 3),
    ]);

for child in parent.nested().unwrap_or_default() {
    eprintln!("  - {}: {}", child.field_or_default(), child.message);
}
```

---

## 11. Caching Expensive Validators

**Use case**: Regex-based or complex validators called repeatedly with same inputs.

```rust
use nebula_validator::prelude::*;

// Wrap any validator — results memoized in a lock-free LRU cache
let v = cached(matches_regex(r"^[A-Z]{2}\d{6}$"));

// First call: computes and caches
v.validate("AB123456")?;

// Subsequent calls with same input: cache hit (~100ns vs ~35ns bare regex)
v.validate("AB123456")?;

// Custom capacity (default: 1000 entries)
use nebula_validator::combinators::cached::Cached;
let v = Cached::with_capacity(email(), 10_000);
```

**When to cache** (see [PERFORMANCE.md](PERFORMANCE.md#cache-strategy)):
- Regex validators with repeated inputs ✅
- Simple `min_length`/`max_length` ❌ (too fast, cache overhead dominates)
- Database-backed validators ✅ (when input space is bounded)

**Cache stats**:
```rust
let stats = v.cache_stats();
println!("hits: {}, misses: {}, size: {}", stats.hits, stats.misses, stats.size);
```

---

## 12. Collection Validation

**Use case**: Validate every element in a Vec, slice, or iterator.

```rust
use nebula_validator::prelude::*;

let items = vec!["alice", "b", "charlie"];

// Validate each element — collects all errors with indexed field paths
let each_v = min_length(2).each();
// Errors will have fields like "/0", "/1", "/2"

// AllOf: all validators must pass for all elements
let all = all_of(vec![min_length(2), max_length(20)]);

// AnyOf: at least one validator must pass for each element
let any = any_of(vec![exact_length(5), exact_length(7)]);
```

---

## 13. Anti-Patterns

### ❌ Avoid: Validation logic in business code

```rust
// BAD — scattered, untestable, no error codes
fn create_user(name: &str) -> Result<(), String> {
    if name.len() < 3 { return Err("too short".into()); }
    if name.len() > 20 { return Err("too long".into()); }
    Ok(())
}
```

### ✅ Prefer: Composable validators

```rust
// GOOD — reusable, testable, machine-readable error codes
let username = min_length(3).and(max_length(20)).and(alphanumeric());
username.validate(name)?;
```

### ❌ Avoid: `.unwrap()` on validation results

```rust
// BAD — panics on invalid input
validator.validate(input).unwrap();
```

### ✅ Prefer: Propagate with `?` or collect errors

```rust
// GOOD — propagate
validator.validate(input)?;

// GOOD — collect all
let mut errors = ValidationErrors::new();
if let Err(e) = validator.validate(input) { errors.push(e); }
```

### ❌ Avoid: Caching cheap validators

```rust
// BAD — cache overhead exceeds validation cost
let v = cached(not_empty());  // not_empty is ~5ns, cache lookup is ~100ns
```

### ❌ Avoid: Blocking I/O in `Validate::validate()`

The `Validate` trait is synchronous. Perform async checks before calling
`validate()`, not inside it.

# nebula-validator — Extending: Custom Validators

This document explains how to write custom validators, use the `validator!` macro, implement
`Validate<T>` manually, and work with the declarative `Rule` enum.

---

## The `validator!` Macro

`validator!` is the primary way to write a custom validator. It generates:
- A struct definition with `#[derive(Debug, Clone)]` always applied.
- A `Validate<T>` implementation.
- A `new(...)` constructor (or a fallible one if specified).
- An optional public factory function.

### 1. Unit validator — zero-sized struct, no fields

Use when the validator has no configuration.

```rust
use nebula_validator::{validator, foundation::ValidationError};

validator! {
    pub NotEmpty for str;
    rule(input) { !input.is_empty() }
    error(input) { ValidationError::new("not_empty", "must not be empty") }
    fn not_empty();
}

assert!(not_empty().validate("hello").is_ok());
assert!(not_empty().validate("").is_err());
```

For unit structs, the generated factory function is `const fn`, making it usable in
`static` and `const` contexts.

### 2. Struct with fields

`self` is available in `rule` and `error` bodies.

```rust
validator! {
    #[derive(Copy, PartialEq, Eq, Hash)]
    pub MinLength { min: usize } for str;
    rule(self, input) { input.chars().count() >= self.min }
    error(self, input) {
        ValidationError::min_length("", self.min, input.chars().count())
    }
    fn min_length(min: usize);
}
```

### 3. Custom constructor

Override the auto-generated `new` when construction requires validation.

```rust
validator! {
    pub LengthRange { min: usize, max: usize } for str;
    rule(self, input) {
        let n = input.chars().count();
        n >= self.min && n <= self.max
    }
    error(self, input) {
        ValidationError::length_range("", self.min, self.max, input.chars().count())
    }
    new(min: usize, max: usize) {
        Self { min, max }
    }
    fn length_range(min: usize, max: usize);
}
```

### 4. Fallible constructor

Return `Err(ValidationError)` from `new` to reject invalid configuration at construction time
rather than at validation time.

```rust
validator! {
    pub LengthRange { min: usize, max: usize } for str;
    rule(self, input) {
        let n = input.chars().count();
        n >= self.min && n <= self.max
    }
    error(self, input) {
        ValidationError::length_range("", self.min, self.max, input.chars().count())
    }
    new(min: usize, max: usize) -> ValidationError {
        if min > max {
            return Err(ValidationError::new(
                "invalid_range",
                format!("min ({min}) must not be greater than max ({max})"),
            ));
        }
        Ok(Self { min, max })
    }
    fn length_range(min: usize, max: usize) -> ValidationError;
}

// Factory function returns Result<LengthRange, ValidationError>:
let v = length_range(3, 20)?;
```

### 5. Bounded generic

The `<T: Bounds>` syntax in `validator!` generates a generic struct with trait bounds.

```rust
use std::fmt::Display;

validator! {
    #[derive(Copy, PartialEq, Eq, Hash)]
    pub Min<T: PartialOrd + Display + Copy> { min: T } for T;
    rule(self, input) { *input >= self.min }
    error(self, input) {
        ValidationError::new("min", format!("must be >= {}", self.min))
    }
    fn min(value: T);
}

min(0u32).validate(&5)?;  // Ok
min(0i64).validate(&-1)?; // Err
```

### 6. Phantom generic — generic type, no stored field

Use when the validator needs to be generic over `T` but carries no `T`-typed field.

```rust
validator! {
    pub Required<T> for Option<T>;
    rule(input) { input.is_some() }
    error(input) { ValidationError::new("required", "value is required") }
    fn required();
}

required::<String>().validate(&None)?; // Err
required::<i32>().validate(&Some(42))?; // Ok
```

---

## Implementing `Validate<T>` Manually

For complex validators that don't fit the macro syntax, implement the trait directly.

```rust
use nebula_validator::foundation::{Validate, ValidationError};

pub struct Divisible {
    pub divisor: u64,
}

impl Validate<u64> for Divisible {
    fn validate(&self, input: &u64) -> Result<(), ValidationError> {
        if self.divisor == 0 {
            return Err(ValidationError::new(
                "invalid_config",
                "divisor must not be zero",
            ));
        }
        if input % self.divisor != 0 {
            return Err(
                ValidationError::new("not_divisible", format!("{input} is not divisible by {}", self.divisor))
                    .with_param("divisor", self.divisor.to_string())
                    .with_param("input", input.to_string()),
            );
        }
        Ok(())
    }
}

// Compose freely with any combinator:
let v = Divisible { divisor: 3 }.and(Divisible { divisor: 5 });
v.validate(&15)?; // Ok — divisible by both 3 and 5
```

---

## Registering an Error Code

Every custom validator must register its error code in
`tests/fixtures/compat/error_registry_v1.json`. This is enforced by
`tests/contract/governance_policy_test.rs`.

Add an entry following the existing format:

```json
{
  "code": "not_divisible",
  "stability": "stable",
  "produced_by": ["Divisible"],
  "description": "Input is not divisible by the specified divisor."
}
```

Until registered, the governance test will fail with a list of unregistered codes.

---

## The Declarative `Rule` Enum

`Rule` is useful when validation logic must be stored in a database, serialized to JSON, or
configured by end users at runtime. It covers a fixed set of built-in rules; for arbitrary
logic, use the programmatic API.

### Defining and evaluating rules

```rust
use nebula_validator::{Rule, ExecutionMode, validate_rules};
use serde_json::json;

let rules = vec![
    Rule::MinLength { min: 3, message: None },
    Rule::MaxLength { max: 20, message: None },
    Rule::Pattern { pattern: "^[a-z0-9]+$".into(), message: None },
];

validate_rules(&json!("alice99"), &rules, ExecutionMode::StaticOnly)?;
```

### Storing rules

`Rule` implements `Serialize` and `Deserialize`, so rules can be persisted in any JSON-capable
store and loaded back at runtime:

```rust
let json_rules = serde_json::to_string(&rules)?;
let loaded: Vec<Rule> = serde_json::from_str(&json_rules)?;
```

### Context predicates

Context predicates check sibling fields in a map. They are typically used inside `Rule::All`
to express "validate field X only when field Y has value Z":

```rust
use std::collections::HashMap;
use serde_json::json;

let rules = vec![
    Rule::All {
        rules: vec![
            Rule::IsPresent { field: "email".into() },
            Rule::Eq { field: "role".into(), value: json!("admin") },
        ],
    },
];

let ctx: HashMap<String, serde_json::Value> =
    serde_json::from_value(json!({ "email": "a@b.com", "role": "admin" }))?;

// evaluate() returns bool (used in Rule context predicates)
let passes = rules[0].evaluate(&ctx);
```

### Mixing declarative and programmatic

`Rule` evaluation and `Validate<T>` are independent layers. Use declarative rules for
user-configured logic and programmatic validators for invariants that must never change:

```rust
// Hard-coded invariant: email format (programmatic, can't be overridden by users)
let format_check = email();

// User-configured constraints (loaded from DB at runtime)
let user_rules: Vec<Rule> = load_user_rules_from_db()?;

// Apply both:
format_check.validate(input)?;
validate_rules(&json!(input), &user_rules, ExecutionMode::StaticOnly)?;
```

---

## Tips for Writing Good Validators

**Register error codes before merging.** The governance test will catch unregistered codes in
CI; registering upfront avoids a last-minute failure.

**Make constructors fallible when configuration can be invalid.** A fallible `new` (see
variant 4 above) is better than returning a validator that always fails at the first call.

**Use `with_param` for diagnostic context.** Downstream consumers and logs benefit from
structured `params` (e.g., `min=3, actual=1`) rather than embedded values in the message
string. Avoid putting sensitive values in params; they will be redacted automatically only
for known sensitive key names.

**Implement `Clone` and make validators `Send + Sync`.** The combinator infrastructure and
`AnyValidator` require these bounds. The `validator!` macro always derives `Clone`.

**Prefer static strings for codes and messages.** `Cow<'static, str>` fields in
`ValidationError` allow static string literals to flow through without allocation.
Use `format!` only when the message must embed runtime values.

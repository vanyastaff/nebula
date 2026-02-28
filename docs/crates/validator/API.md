# API

## Public Surface

- stable APIs:
  - `foundation::Validate<T>`
  - `foundation::ValidateExt<T>`
  - `foundation::Validatable`
  - `foundation::ValidationError`, `ValidationErrors`
  - built-in validators from `validators::*`
  - core combinators from `combinators::*`
- experimental APIs:
  - none explicitly marked yet; treat advanced combinator internals as non-contract
- hidden/internal APIs:
  - internal macro plumbing and erased trait internals

## Usage Patterns

- direct typed validation:
  - `validator.validate(&value)`
- extension style:
  - `value.validate_with(&validator)`
- composition style:
  - `min_length(3).and(max_length(20)).and(alphanumeric())`

## Minimal Example

```rust
use nebula_validator::prelude::*;

let username = min_length(3).and(max_length(20)).and(alphanumeric());
username.validate("alice123")?;
```

## Advanced Example

```rust
use nebula_validator::prelude::*;
use serde_json::json;

let password_rule = min_length(12).and(contains("@"));
let payload = json!("very@secure");
password_rule.validate_any(&payload)?;
```

## Error Semantics

- retryable errors:
  - not applicable at validator layer; validation failures are deterministic contract failures.
- fatal errors:
  - invalid input/shape/range/pattern failures.
- validation errors:
  - represented by `ValidationError` (single) and `ValidationErrors` (aggregate).

## Compatibility Rules

- major bump required when:
  - behavior changes for existing validator semantics
  - existing error code meanings change
  - existing field-path format contract changes
- minor versions:
  - additive validators/combinators/error helpers only
- deprecation policy:
  - mark deprecated APIs with migration path and maintain for at least one minor cycle

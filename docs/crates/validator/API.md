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

Fixture-aligned expectations:

- `"alice123"` => pass
- `"ab"` => fail with `min_length`
- `"alice_123"` => fail with `alphanumeric`

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

## Canonical Error Code Catalog (Baseline)

- `required`: missing required value
- `min_length`: string shorter than minimum bound
- `max_length`: string longer than maximum bound
- `invalid_format`: pattern/content format failure
- `type_mismatch`: dynamic bridge type conversion mismatch
- `out_of_range`: numeric range failure
- `exact_length`: string length not equal to fixed bound
- `length_range`: string length outside inclusive range
- `or_failed` / `or_any_failed`: OR combinator alternatives all failed
- `not_failed`: NOT combinator inner validator unexpectedly passed

Code meanings are compatibility-critical for downstream mappings.

## Field-Path Contract

- typed field combinators (`field`, `named_field`) use dot notation:
  - examples: `user.email`, `config.timeout`, `items.0.name`
- JSON field combinators (`json_field`, `json_field_optional`) use RFC 6901 JSON Pointer:
  - examples: `/user/email`, `/items/0/name`
- contract rule:
  - field-path format for an existing API is stable across minor releases.
  - changing format semantics requires a major version and migration mapping.

Serialized envelope note:

- runtime JSON envelope currently uses `field` key.
- adapter layer may expose `field_path` alias for external consumers.

## Compatibility Rules

- major bump required when:
  - behavior changes for existing validator semantics
  - existing error code meanings change
  - existing field-path format contract changes
- minor versions:
  - additive validators/combinators/error helpers only
- deprecation policy:
  - mark deprecated APIs with migration path and maintain for at least one minor cycle

## Contract Test Fixtures

- compatibility fixture source:
  - `crates/validator/tests/fixtures/compat/minor_contract_v1.json`
- contract tests:
  - `crates/validator/tests/contract/compatibility_fixtures_test.rs`
  - `crates/validator/tests/contract/typed_dynamic_equivalence_test.rs`
  - `crates/validator/tests/contract/error_envelope_schema_test.rs`

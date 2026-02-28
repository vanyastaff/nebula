# API Reference (Human-Oriented)

## Foundation

- `Validate<T>`
  - core trait: `fn validate(&self, input: &T) -> Result<(), ValidationError>`
  - includes `validate_any` bridge for adaptable/dynamic inputs
- `Validatable`
  - extension trait for values: `value.validate_with(&validator)`
- `ValidateExt<T>`
  - combinator methods: `.and()`, `.or()`, `.not()`, `.when()`

## Error Types

- `ValidationError`
  - code/message/field + optional parameters
  - nested errors
  - severity/help metadata
  - utility constructors (`required`, `min_length`, `max_length`, `type_mismatch`, ...)
- `ValidationErrors`
  - collection type for accumulating multiple failures

## Context APIs

- `ValidationContext`
  - key-value typed storage for cross-field validation logic
  - parent/child context chain
  - field path tracking helpers
- `ContextualValidator`
  - trait for validators that need external context
- `ContextAdapter`
  - bridge: regular `Validate<T>` into contextual interface

## Built-in Validator Families

- `validators::length`
  - `min_length`, `max_length`, `exact_length`, `length_range`, `not_empty`
- `validators::pattern`
  - `contains`, `starts_with`, `ends_with`, `alphanumeric`, etc.
- `validators::content`
  - `email`, `url`, `matches_regex`
- `validators::range`
  - `min`, `max`, `in_range`, `exclusive_range`, `greater_than`, `less_than`
- `validators::size`
  - `min_size`, `max_size`, `exact_size`, `size_range`, `not_empty_collection`
- `validators::boolean`
  - `is_true`, `is_false`
- `validators::nullable`
  - `required`, `not_null`
- `validators::network`
  - `ip_addr`, `ipv4`, `ipv6`, `hostname`
- `validators::temporal`
  - `date`, `time`, `date_time`, `uuid`

## Combinators

- logical: `and`, `or`, `not`
- conditional: `when`, `unless`
- optional: `optional`
- collection/object helpers: `each`, `field`, `json_field`, `nested`
- error/message shaping: `with_code`, `with_message`
- performance: `cached`, `lazy`
- factories: `all_of`, `any_of`

## Macros

- `validator!`
  - generates struct + impl + constructors/factories
  - supports unit/field/generic/phantom/fallible constructor variants
- `compose!`
  - AND-chain helper
- `any_of!`
  - OR-chain helper

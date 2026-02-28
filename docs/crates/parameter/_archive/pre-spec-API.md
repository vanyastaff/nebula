# API Reference (Human-Oriented)

## Schema Types

- `ParameterDef`
  - tagged enum with variants for all parameter types
  - shared accessors: `key`, `name`, `kind`, `metadata`, `display`, `validation_rules`, `children`
- `ParameterKind`
  - kind taxonomy (`Text`, `Number`, `Select`, `Object`, `List`, `Mode`, ...)
  - capability helpers (`has_value`, `is_editable`, `is_container`, `supports_expressions`, ...)
- `ParameterMetadata`
  - key/name/description/required/placeholder/hint/sensitive

## Runtime Values

- `ParameterValues`
  - flat key->`serde_json::Value`
  - typed getters (`get_string`, `get_f64`, `get_bool`)
  - `snapshot`/`restore`
  - `diff` producing `added`/`removed`/`changed`

## Schema Collections

- `ParameterCollection`
  - add/get/remove/contains/iter APIs
  - `validate(&ParameterValues) -> Result<(), Vec<ParameterError>>`

Validation behavior:
- skips display-only kinds (`notice`, `group`)
- enforces required/missing semantics
- checks JSON type compatibility
- applies `ValidationRule`s
- recursively validates nested `object` and `list` values

## Validation Rules

- `ValidationRule` variants:
  - `MinLength`, `MaxLength`, `Pattern`
  - `Min`, `Max`
  - `OneOf`
  - `MinItems`, `MaxItems`
  - `Custom` (external evaluation path)
- builder helpers:
  - `min_length`, `max_length`, `pattern`, `min`, `max`, `range`, `min_items`, `max_items`
  - `.with_message(...)`

## Display Rules

- `ParameterDisplay`
- `DisplayRuleSet`
- `DisplayCondition`
- `DisplayContext`

Used to compute parameter visibility from current sibling values and validation context.

## Errors

- `ParameterError`:
  - lookup/existence errors
  - type/value/required violations
  - validation errors
  - serialization/deserialization errors
- helpers:
  - `category()`
  - `code()`
  - `is_retryable()` (always false)

## Options and Select Support

- `SelectOption`
- `OptionsSource`

Supports static options and dynamic option loader references.

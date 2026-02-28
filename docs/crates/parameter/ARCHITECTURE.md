# Architecture

## Positioning

`nebula-parameter` is a schema-definition crate, not a UI crate and not an execution crate.

Dependency direction:
- action/plugin/runtime/UI adapters -> `nebula-parameter`
- `nebula-parameter` stays independent from engine orchestration internals

## Internal Structure

- `kind.rs`
  - `ParameterKind` + capability model + expected JSON value type
- `def.rs`
  - `ParameterDef` tagged enum with delegation helpers
- `types/`
  - concrete parameter structs per kind (`text`, `number`, `object`, `list`, `mode`, ...)
- `metadata.rs`
  - human-facing parameter metadata
- `validation.rs`
  - declarative validation rule schema
- `display.rs`
  - conditional visibility model (`show_when`/`hide_when`)
- `collection.rs`
  - ordered schema collection + runtime validation pipeline
- `values.rs`
  - runtime value store + snapshot/diff helpers
- `error.rs`
  - `ParameterError` classification and error codes

## Core Design Constraints

- schema must serialize cleanly to JSON for transport/persistence
- parameter types remain strongly modeled in Rust while values are runtime JSON
- validation should be deterministic and aggregate errors
- container parameters (`object`, `list`, `mode`, `group`, `expirable`) support nested recursion

## Runtime Validation Flow

`ParameterCollection::validate(values)`:
1. required/null checks
2. JSON type check against `ParameterKind::value_type()`
3. rule evaluation via `nebula-validator`
4. recursive validation of nested containers
5. aggregate all `ParameterError` values

## Known Constraints

- custom validation expressions are declared but evaluated outside this crate
- value typing is JSON-based, so compile-time guarantees end at schema boundary

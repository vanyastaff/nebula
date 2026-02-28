# Interactions

## Ecosystem Map (Current + Planned)

`nebula-parameter` is a schema-definition crate. Dependency direction: action/credential/engine/macros/sdk → parameter; parameter → validator.

## Existing Crates

- **core:** Shared IDs; parameter does not depend on core
- **action:** Attaches `ParameterCollection` to action metadata; re-exports `ParameterCollection`, `ParameterDef`
- **credential:** Uses `ParameterCollection` for credential protocol schemas; `ParameterValues` for credential input; `ParameterError` in credential errors
- **engine:** Uses parameter for node config validation (planned/partial)
- **macros:** Generates `ParameterDef` and `ParameterCollection` from derive attributes; uses `ParameterValues` as action input type
- **sdk:** Re-exports parameter prelude for plugin authors
- **validator:** Parameter delegates rule evaluation to validator; validator has no dependency on parameter
- **expression:** Resolves expressions in values before validation; Custom `ValidationRule` evaluated by expression engine
- **log, config, storage, resource, system:** No direct dependency on parameter

## Planned Crates

- **workflow / runtime / worker:** Will consume parameter for workflow/node config validation
- **api / cli / ui:** Will use parameter schema for form rendering and error mapping

## Downstream Consumers

- **action:** Expects `parameters() -> ParameterCollection`; `Input = ParameterValues`
- **credential:** Expects `ParameterCollection` for protocol schemas; `ParameterValues` for credential resolution input
- **macros:** Expects stable `ParameterDef` variants and constructor patterns
- **sdk:** Expects prelude and stable API for plugin authors

## Upstream Dependencies

- **nebula-validator:** Rule evaluation in `ParameterCollection::validate`; hard contract on `ValidationRule` → validator rule mapping
- **serde, serde_json:** Serialization; fallback: N/A (required)
- **thiserror:** Error derivation; fallback: N/A

## Interaction Matrix

| This crate <-> Other | Direction | Contract | Sync/Async | Failure handling | Notes |
|---------------------|-----------|----------|------------|------------------|-------|
| parameter -> validator | out | ValidationRule evaluation | sync | return Vec<ParameterError> | validator evaluates rules |
| action -> parameter | in | ParameterCollection, ParameterDef | sync | N/A | action owns schema |
| credential -> parameter | in | ParameterCollection, ParameterValues | sync | ParameterError in credential errors | credential resolves values |
| macros -> parameter | in | ParameterDef constructors, ParameterValues | sync | N/A | codegen |
| sdk -> parameter | in | prelude, public API | sync | N/A | re-export |
| expression -> parameter | out | Custom rule expression | sync | N/A | expression engine evaluates Custom |

## Runtime Sequence

1. Action/credential defines schema via `ParameterCollection`
2. User/engine populates `ParameterValues` (possibly with unresolved expressions)
3. Expression engine resolves expressions → concrete `serde_json::Value`
4. `ParameterCollection::validate(&values)` runs; errors aggregated
5. On success, values passed to action/credential execution

## Cross-Crate Ownership

- **parameter owns:** Schema types, validation semantics, error codes, display rule evaluation
- **action/credential own:** When to validate, how to present errors to user
- **expression owns:** Expression resolution; Custom rule evaluation
- **validator owns:** Rule execution logic (min, max, pattern, etc.)

## Failure Propagation

- Validation failures bubble up as `Vec<ParameterError>`; no retries (deterministic)
- Credential may wrap `ParameterError` in credential-specific error type
- API layer maps `ParameterError::code()` to HTTP 400/422

## Versioning and Compatibility

- **Compatibility promise:** Patch/minor preserve `ParameterDef`, `ValidationRule`, `ParameterError` variants and codes
- **Breaking-change protocol:** Declare in MIGRATION.md; major version bump; migration path
- **Deprecation window:** Minimum 6 months

## Contract Tests Needed

- action/parameter: schema round-trip, validation error mapping
- credential/parameter: protocol schema validation, ParameterError propagation
- macros/parameter: generated code compiles and validates correctly

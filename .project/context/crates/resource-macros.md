# nebula-resource-macros

Proc-macro crate providing `#[derive(ClassifyError)]` for nebula-resource error classification.

## Invariants

- `#![forbid(unsafe_code)]`, `#![warn(missing_docs)]`
- Generates `From<UserError> for nebula_resource::Error` with correct `ErrorKind` per variant
- Supported attributes: `#[classify(transient)]`, `#[classify(permanent)]`, `#[classify(exhausted, retry_after = "30s")]`
- Duration parsing: `"30s"`, `"5m"`, `"1h"` formats via manual parser
- Works on enums only — structs are not supported
- Named fields, unnamed fields, and unit variants all supported

## Traps

- `retry_after` is only valid with `exhausted` — compile error otherwise
- Every variant MUST have a `#[classify(...)]` attribute — no default fallback
- The generated `From` impl calls `.to_string()` on the source error for the message

## Relations

- Depended on by: nebula-resource (re-exports `ClassifyError` derive)
- No other dependencies beyond proc-macro2, quote, syn

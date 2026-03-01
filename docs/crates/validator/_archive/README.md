# nebula-validator — Archive

Pre-SPEC and topic-level docs moved here during the docs deep pass (see root `docs/SPEC.md` and `docs/docs-deep-pass-checklist.md`). **Mine for insights; do not delete.**

**Depends on:** nothing from Nebula (standalone)  
**Used by:** `nebula-config`, `nebula-parameter`, `nebula-macros`, `nebula-sdk`, and any crate that validates user input.

---

## Files in this archive

- **Topic docs:** [traits.md](traits.md), [error.md](error.md), [validators.md](validators.md), [combinators.md](combinators.md), [macros.md](macros.md), [context.md](context.md)
- **Pre-SPEC:** pre-spec-README.md, pre-spec-ARCHITECTURE.md, pre-spec-API.md, pre-spec-DECISIONS.md, pre-spec-ROADMAP.md, pre-spec-PROPOSALS.md
- **Archive notes:** archive-ideas.md, archive-phase-1-core.md, archive-crates-dependencies.md

---

## Quick Start (reference)

```rust
use nebula_validator::prelude::*;

// Extension method style (left-to-right reading)
"alice".validate_with(&min_length(3))?;
42.validate_with(&in_range(18, 100))?;

// Compose with .and() / .or() / .not()
let username = min_length(3).and(max_length(20)).and(alphanumeric());
assert!(username.validate("alice").is_ok());

// Macros
let email_val = compose![not_empty(), email(), max_length(255)];
let port_val  = any_of![in_range(80, 80), in_range(443, 443), in_range(1024, 65535)];
```

---

## Module Map

| Module | What it provides |
|---|---|
| `foundation` | `Validate<T>`, `Validatable`, `ValidateExt`, `ValidationError`, `ValidationErrors`, `ValidationContext` |
| `validators` | 30+ built-in validators (string, numeric, collection, boolean, network, temporal) |
| `combinators` | Composition types: `Cached`, `Optional`, `Each`, `Field`, `JsonField`, `When`, `Unless`, `Lazy`, … |
| `macros` | `validator!`, `compose!`, `any_of!` |

---

## Topic Files

- [traits.md](traits.md) — `Validate<T>`, `Validatable`, `ValidateExt`, combinator types
- [error.md](error.md) — `ValidationError` (memory layout, builder), `ValidationErrors`, `ErrorSeverity`
- [validators.md](validators.md) — all 30+ built-in validators by category
- [combinators.md](combinators.md) — composition combinators
- [macros.md](macros.md) — `validator!`, `compose!`, `any_of!`
- [context.md](context.md) — `ValidationContext`, `ContextualValidator`
- [archive-ideas.md](archive-ideas.md) — unimplemented design ideas from early docs (`#[validate(...)]` attribute macro, etc.)
- `pre-spec-README.md`, `pre-spec-ARCHITECTURE.md`, `pre-spec-API.md`, `pre-spec-DECISIONS.md`, `pre-spec-ROADMAP.md`, `pre-spec-PROPOSALS.md` — first docs refresh before root `SPEC.md` template migration

---

## Prelude

```rust
use nebula_validator::prelude::*;
// Validate, Validatable, ValidateExt, ValidationError, ValidationErrors,
// And, Or, Not, When, AnyValidator, AsValidatable, ErrorSeverity,
// Cached, JsonField, and, cached, json_field, json_field_optional, not, or,
// + all built-in validators (min_length, email, in_range, …)
```

---

## Integration with nebula-parameter

`nebula-parameter` attaches `ValidationRule` descriptors to `ParameterDef` structs.
Before action execution the engine converts those descriptors into actual validator
calls against resolved `serde_json::Value` inputs:

```
ValidationRule (data) → nebula-validator (logic) → Result<(), ValidationError>
```

The `validate_any` method on `Validate<T>` bridges typed validators with
`serde_json::Value` via `AsValidatable` conversions:

```rust
// Validate a JSON value through a typed validator
let v = min_length(3);
assert!(v.validate_any(&json!("hello")).is_ok());
assert!(v.validate_any(&json!("hi")).is_err());
```

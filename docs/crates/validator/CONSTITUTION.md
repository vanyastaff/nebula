# nebula-validator Constitution

> **Version**: 1.0.0 | **Created**: 2026-03-01

---

## Platform Role

Config, API, action parameters, and workflow definitions all need validation: types, ranges, patterns, and custom rules. A single composable validation framework with structured errors and error codes allows consistent behavior and clear user feedback across the platform.

**nebula-validator is the composable validation framework for Nebula crates and runtime boundaries.**

It answers: *How do crates validate input (config, API payloads, parameters) with typed validators, combinators, and structured errors that support i18n and compatibility?*

```
Caller has value T (config, JSON, parameter set)
    ↓
Validate<T> or validate_any with AnyValidator
    ↓
ValidationError / ValidationErrors with category, field path, code
    ↓
Caller maps to API response or config load error; optional i18n by code
```

This is the validator contract: type-safe Validate<T>, composable combinators, structured errors with category and path, and compatibility policy for error codes.

---

## User Stories

### Story 1 — Config Crate Validates Loaded Config (P1)

Config loads from file and env; before accepting config it runs a validator (e.g. ConfigValidator bridging nebula-validator). Invalid config produces ValidationErrors; load returns Err with structured errors so that UI or CLI can show field-level messages.

**Acceptance**:
- Validator trait (Validate<T> or bridge) returns ValidationError(s)
- Category and field path in each error; stable error codes where applicable
- Config crate uses validator for load and reload gate; no duplicate validation logic

### Story 2 — API Validates Request Payloads (P1)

API receives JSON body; it validates shape and business rules via validators. 400 response includes structured errors (field path, code, message). Error codes are stable for client handling and i18n.

**Acceptance**:
- validate_any or typed Validate<T> for request body
- ValidationErrors serialize to API error format
- Error code registry or fixture for compatibility

### Story 3 — Action/Parameter Uses Same Error Shape (P2)

Parameter validation and action input validation produce the same error shape (category, path, code) so that UI and API can present them consistently. Validator crate is the source of truth for error structure.

**Acceptance**:
- ValidationError is the shared type; nested errors for composite validation
- Parameter and config use same category/path conventions where applicable
- Contract fixtures lock error shape for minor compatibility

### Story 4 — Performance Budget for Hot Paths (P2)

Validation runs on every config load and many API requests. Hot paths have benchmark and optional performance budget so that new combinators do not regress latency.

**Acceptance**:
- Benchmarks for common validator chains
- Document fast-path vs full validation; no unbounded recursion in default validators
- Large generic types documented; optional simplification proposals

---

## Core Principles

### I. Type-Safe Validate<T> Model

**Validation is typed: Validate<T> validates T. Combinators compose validators without losing type information where possible.**

**Rationale**: Type safety prevents wrong value being validated and improves ergonomics. Composability allows reuse (min, max, pattern, one_of) across config, API, and parameters.

**Rules**:
- Core trait Validate<T> or equivalent; validate(value: &T) -> Result<(), ValidationError>
- ValidateExt or similar for composition (and_then, map_err)
- AnyValidator and validate_any for dynamic/downcast use cases

### II. Structured Errors with Category and Path

**ValidationError carries category, field path, and optional code. Nested errors (ValidationErrors) for composite validation.**

**Rationale**: API and UI need to show "field X failed rule Y". Category and path enable consistent formatting and i18n by code.

**Rules**:
- ValidationError has category and path; code is stable where documented
- ValidationErrors aggregates multiple errors; nested for nested structures
- Compatibility: minor = additive error codes only; major = migration for behavior changes

### III. No Transport or Orchestration in Validator

**Validator validates values. It does not format HTTP responses, retry, or orchestrate workflow.**

**Rationale**: API and config own transport and retry. Validator is a library used at boundaries.

**Rules**:
- No HTTP or transport types in validator crate
- No workflow or execution types; only value validation
- Formatting for API is in api crate; validator returns structured data

### IV. Compatibility Policy for Error Codes and Paths

**Error codes and field path semantics are versioned. Minor = additive only; major = migration guide.**

**Rationale**: Clients and i18n depend on error codes. Breaking code or path semantics breaks consumers.

**Rules**:
- Error registry or fixture (e.g. error_registry_v1.json) for compatibility tests
- Minor: new codes and paths OK; no removal or meaning change
- Major: document old→new in MIGRATION.md

### V. Composable Combinators and Macros

**Common patterns (and_then, any_of, compose, named_field) are combinators or macros. No monolithic validator per domain.**

**Rationale**: Reuse and testability. Config, API, and parameter share min, max, pattern, one_of.

**Rules**:
- Combinators are in validator crate; domain crates use them
- Macro ergonomics (validator!, compose!, any_of!) documented; compile-time cost acknowledged
- Large generic types documented; consider type alias or builder where debuggability suffers

---

## Production Vision

### The validator in an n8n-class fleet

In production, every boundary that accepts input uses validator: config load, API request body, parameter submission, workflow definition. Validators are composed from built-ins (min_length, max_length, in_range, required, email, etc.) and combinators (.and(), .or(), .not(), .when(), optional(), json_field()). Errors are ValidationError (code, message, field, params, nested) and ValidationErrors; API and config map them to responses and load errors. Error code registry and compatibility fixtures (error_registry_v1.json, minor_contract_v1.json) are used for compatibility.

```
foundation: Validate<T>, ValidateExt, Validatable, ValidationError, ValidationErrors,
            ValidationContext, ContextualValidator, AnyValidator, AsValidatable
validators: length, pattern, content, range, size, boolean, nullable, network, temporal
combinators: And, Or, Not, When, Optional, Field, JsonField, Each, Cached, all_of, any_of
macros: validator!, compose!, any_of!
```

Config integration: config crate consumes validator via trait bridge; category naming pinned by config fixture (validator_contract_v1.json). Performance: criterion benches in benches/; contract tests in tests/contract/.

### From the archives: combinators and config bridge

The archive (`docs/crates/validator/_archive/`: combinators.md, error.md, validators.md, traits.md, context.md, macros.md, pre-spec-*.md, archive-*.md) and README describe composable validators and config integration. Production vision: stable public contract (Validate<T>, ValidateExt, ValidationError, ValidationErrors); compatibility fixtures (error_registry_v1.json, minor_contract_v1.json) and contract tests in tests/contract/; config and parameter consume via trait bridge; no transport or orchestration in validator.

### Key gaps from current state to prod

| Gap | Priority | Notes |
|-----|----------|-------|
| Error code governance and registry automation | High | Formalize compatibility checks in CI |
| Cross-crate category/path fixture alignment | High | Config, API, parameter share fixtures |
| Performance budget and fast PR profile | Medium | Bench policy: fast in PR, full on release |
| Large generic type ergonomics | Low | Type aliases or builder to reduce compile time and debug noise |
| i18n mapping document | Low | Document code → message key for frontends |

---

## Key Decisions

### D-001: Validator Crate Owns Error Structure

**Decision**: ValidationError and ValidationErrors are defined in validator crate; config, API, parameter use them.

**Rationale**: Single source of truth for error shape. Consistency across boundaries.

**Rejected**: Each crate defining its own error shape — would fragment and duplicate.

### D-002: Config Integration via Trait Bridge

**Decision**: Config crate uses a validator trait (ConfigValidator or bridge to Validate) for load/reload validation.

**Rationale**: Config does not duplicate validation logic; validator remains generic.

**Rejected**: Config defining its own validation types only — would duplicate combinator logic.

### D-003: Compatibility Fixtures for Minor Releases

**Decision**: Error registry and category/path fixtures are versioned; minor release = additive-only changes to fixtures.

**Rationale**: Prevents accidental breaking of error codes or shape. CI can enforce.

**Rejected**: No fixtures — would allow accidental breakage.

### D-004: AnyValidator for Dynamic Validation

**Decision**: When type is not known at compile time (e.g. JSON from API), validate_any and AnyValidator allow dynamic dispatch.

**Rationale**: API and generic middleware need to validate without generic over every body type.

**Rejected**: Only typed Validate<T> — would block dynamic validation use cases.

---

## Open Proposals

### P-001: Error Code Registry CI Check

**Problem**: Manual check for additive-only error codes is error-prone.

**Proposal**: CI job that diffs error registry and fails if minor release removes or changes existing code.

**Impact**: Additive; improves governance.

### P-002: Fast vs Full Benchmark Profile

**Problem**: Full validator bench suite can slow PRs.

**Proposal**: Split: fast subset on every PR; full suite on release or nightly.

**Impact**: Non-breaking; document in TEST_STRATEGY or ROADMAP.

### P-003: Simplify Large Combinator Types

**Problem**: Deeply nested generics hurt compile time and debuggability.

**Proposal**: Type aliases, builder, or opaque type for common chains to hide complexity.

**Impact**: API surface change if public type names change; could be additive with aliases.

---

## Non-Negotiables

1. **Validate<T> is the core abstraction** — typed validation with composable combinators.
2. **Structured errors** — ValidationError with category, path, code; ValidationErrors for aggregation.
3. **No transport or orchestration** — validator is a library; API/config own usage.
4. **Error code compatibility** — minor = additive only; fixture and registry for enforcement.
5. **Config and API consume via trait/bridge** — no duplicate validation logic in consumers.
6. **Breaking error shape or code semantics = major + MIGRATION.md** — clients and i18n depend on it.

---

## Governance

- **PATCH**: Bug fixes, docs. No change to ValidationError shape or validator semantics.
- **MINOR**: Additive only (new validators, new error codes). No removal or meaning change of existing codes.
- **MAJOR**: Breaking changes to error type or validator contract. Requires MIGRATION.md.

Every PR must verify: compatibility fixtures pass; no removal of error codes in minor; performance regression check if applicable.

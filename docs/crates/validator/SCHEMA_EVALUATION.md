# Schema / Policy Layer Evaluation

**Task**: VAL-T019 — Evaluate optional schema/policy layer and document findings.
**Status**: Evaluated. Recommendation: **defer** — not needed at current stage.

---

## Context

Proposal P004 suggests adding a declarative schema bridge over the existing typed
validators to support plugin and UI ecosystems that may need schema exchange
(e.g., JSON Schema-style field metadata, auto-generated forms).

This document evaluates whether `nebula-validator` should add such a layer now.

---

## Current Validation Architecture

```
                 ┌─────────────────────────────────┐
                 │      Typed Rust Validators       │  ← Compile-time safety
                 │  Validate<T>, ValidateExt<T>     │
                 ├─────────────────────────────────┤
                 │  validator! macro + combinators  │  ← Declarative composition
                 ├─────────────────────────────────┤
                 │  json_field() + validate_any()   │  ← Runtime JSON bridge
                 └─────────────────────────────────┘
```

The crate already provides two layers of dynamism:
1. **`validate_any()`** — typed validators applied to `serde_json::Value`
2. **`json_field()` / `json_field_optional()`** — RFC 6901 path-based field extraction

These cover the primary use case: validating JSON configs and API payloads
against Rust-defined rules.

---

## What a Schema/Policy Layer Would Add

A schema layer would provide:

| Feature | Current Support | Schema Layer Adds |
|---------|----------------|-------------------|
| Validate a JSON value | ✅ `validate_any()` | — |
| Extract and validate fields | ✅ `json_field()` | — |
| Introspect validator rules | ❌ | Metadata: min/max/pattern/required |
| Generate UI forms from rules | ❌ | Field descriptions, types, constraints |
| Serialize/exchange validation rules | ❌ | JSON Schema-compatible output |
| Cross-language validator sharing | ❌ | Language-independent schema format |

---

## Evaluation Criteria

### 1. Does the project need it now?

**No.** The current consumers are:
- `nebula-api` — uses `Validate<T>` for Axum extraction (Rust-only)
- `nebula-config` — applies validators from schema metadata (already has its own schema)
- `nebula-sdk` — re-exports `Validate<T>` for plugin authors (Rust-only)
- `nebula-parameter` — conditional validation for workflow params

All consumers are **Rust crates** within the same workspace. No external plugin
ecosystem or UI form generator exists yet.

### 2. Risk of premature abstraction

**High.** A schema layer requires:
- A stable schema format (JSON Schema subset? Custom?)
- Bidirectional mapping: schema ↔ validator
- Versioning and compatibility guarantees for the schema format itself
- Testing that schema and validator stay in sync

This is significant surface area with no current consumer. Building it now risks:
- Designing for hypothetical requirements
- Dual source of truth (schema vs typed validators)
- Maintenance burden without payoff

### 3. Is `nebula-config` not already covering this?

`nebula-config` already has a `schema.rs` module that maps config field definitions
to validators. This is effectively a domain-specific schema layer for config
validation. A generic schema layer in `nebula-validator` would partially
duplicate this.

### 4. What would trigger the need?

The schema layer becomes justified when:
- A **desktop/web UI** needs to auto-generate forms from validation rules
- An **external plugin SDK** (non-Rust) needs validator definitions
- A **cross-service validation** protocol is needed (e.g., validating inputs
  before sending to a remote workflow engine)

---

## Recommendation

**Defer to Phase 5 or later.** The typed validator core is expressive enough for
current consumers.

### What to do now

1. Keep P004 in `Draft` status
2. Ensure the typed core remains **schema-friendly** by maintaining:
   - Stable error codes (enforced by error registry)
   - Stable field path format (RFC 6901)
   - Serializable `ValidationError` (already `Serialize`/`Deserialize`)
3. If a UI form generator is built, let `nebula-config`'s existing schema model
   drive it rather than adding a new abstraction in `nebula-validator`

### Prerequisites before revisiting

- [ ] External plugin SDK exists (non-Rust consumers)
- [ ] Desktop UI form generation is designed
- [ ] At least 2 distinct consumers need validator introspection

---

## Alternatives Evaluated

| Approach | Verdict | Reason |
|----------|---------|--------|
| Full JSON Schema bridge in `nebula-validator` | ❌ Premature | No consumer, high maintenance |
| Schema trait (`ValidatorSchema`) with describe methods | ❌ Premature | Same concerns, less useful |
| Schema at `nebula-config` level only | ✅ Current | Already exists, domain-appropriate |
| Schema when desktop UI is designed | ✅ Future | Natural trigger point |

---

## References

- [P004: Schema bridge layer](PROPOSALS.md#p004-schema-bridge-layer-for-plugin-ecosystem)
- [D001: Type-bound validation as primary API](DECISIONS.md#d001-type-bound-validation-as-primary-api)
- [ARCHITECTURE.md — Design Reasoning](ARCHITECTURE.md)

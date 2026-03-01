# Architecture

## Problem Statement

- **Business problem:** Workflow nodes need a canonical way to declare inputs (type, validation, visibility) that UI, engine, and plugins can share without duplication.
- **Technical problem:** Schema must be JSON-serializable for transport/persistence, while Rust provides strong typing at the schema boundary; runtime values are dynamic.

## Current Architecture

- **Module map:**
  - `kind.rs` — `ParameterKind` (19 variants, Copy), `ParameterCapability` (9 flags), value type mapping, convenience predicates
  - `def.rs` — `ParameterDef` tagged enum (`#[serde(tag = "type")]`); macro-generated delegation helpers for metadata/display/validation_rules; `children()` for container traversal
  - `types/` — 19 concrete parameter structs, each owning `metadata: ParameterMetadata`, `validation: Vec<ValidationRule>`, `display: Option<ParameterDisplay>`, type-specific `options` and `default`
  - `metadata.rs` — `ParameterMetadata` (key, name, description, required, placeholder, hint, sensitive); `#[serde(flatten)]` into parent struct
  - `validation.rs` — `ValidationRule` declarative schema (9 variants, `#[serde(tag = "rule")]`); `Custom` variant stored but not evaluated here
  - `display.rs` — `ParameterDisplay` (show_when/hide_when), `DisplayRuleSet` (Single/All/Any/Not), `DisplayCondition` (16 variants), `DisplayContext` (values + validation state)
  - `collection.rs` — `ParameterCollection` backed by `Vec<ParameterDef>`; `validate()` pipeline
  - `values.rs` — `ParameterValues` backed by `HashMap<String, Value>` (`#[serde(flatten)]`); `ParameterSnapshot`/`ParameterDiff`
  - `option.rs` — `SelectOption` (key/name/value/description/disabled), `OptionsSource` (Static/Dynamic)
  - `error.rs` — `ParameterError` (9 variants, `code()`, `category()`, `is_retryable()`)
- **Data/control flow:** Schema defined in Rust → serialized to JSON → consumed by UI/engine; values flow as `serde_json::Value`; validation runs at boundaries before execution.
- **Validation pipeline internals** (`collection.rs::validate_param`):
  1. Skip `"none"` value_type parameters (Notice, Group)
  2. Missing/null check for required parameters → `MissingValue`
  3. JSON type check (`value_matches_type`) → `InvalidType` (stops rule evaluation)
  4. Rule evaluation via `nebula-validator` functions; `Custom` rules explicitly skipped with `return`
  5. Recursive descent for `Object` (path: `"parent.field"`) and `List` (path: `"list[0]"`)
- **Known bottlenecks:** Deep nested object/list validation allocates per-path strings; large collections with many errors may allocate heavily.

## Target Architecture

- **Target module map:** Same structure; add optional `lint` module for schema preflight.
- **Public contract boundaries:** `ParameterCollection`, `ParameterValues`, `ParameterDef`, `ParameterError` are the stable surface.
- **Internal invariants:** Keys unique within scope; display rules reference sibling keys only; validation rules match kind capabilities.

## Design Reasoning

- **Schema-first, value-later:** Enables transport/storage compatibility and dynamic workflow execution; type guarantees end at schema boundary.
- **Declarative validation rules:** Serializable rules evaluated in validation layer; enables persistence, tooling, cross-layer portability. `Custom` rules stored in schema but delegated to the expression engine — keeps this crate independent of expression evaluation.
- **Capability-based kind semantics:** `ParameterCapability` flags instead of ad-hoc checks; clearer downstream logic for UI/render/engine.
- **Error aggregation (not fail-fast):** All validation failures collected into `Vec<ParameterError>` so UI can show every error at once rather than one at a time.
- **`#[serde(flatten)]` on metadata:** All `ParameterMetadata` fields appear at the top level of the parameter JSON object alongside type-specific fields, matching the n8n-style schema format consumed by UI.
- **Rejected alternatives:** Hardcoded per-kind validation logic (chose declarative rules); fail-fast validation (chose error aggregation for better UX).

## Comparative Analysis

Sources: n8n, Node-RED, Activepieces/Activeflow.

- **Adopt:** n8n-style parameter types (text, number, select, object, list); Node-RED typed inputs; declarative validation.
- **Reject:** n8n's mixed UI/schema concerns (we separate); Activepieces' per-node ad-hoc schemas (we use shared `ParameterDef`).
- **Defer:** Expression DSL in schema (Custom rule delegates to engine); dynamic options loader (OptionsSource.Dynamic is declared, resolved elsewhere).

## Breaking Changes (if any)

- Typed value layer (P-001) would add `ParameterRuntimeValue`; migration path in PROPOSALS.
- `ParameterKey` newtype (P-003) would change lookup signatures; migration in PROPOSALS.

## Open Questions

- Q1: Should `ParameterCollection::lint()` run at build time or first validate?
- Q2: Formal dependency graph extraction from display rules — when to detect cycles?

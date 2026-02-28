# Architecture

## Problem Statement

- **Business problem:** Workflow nodes need a canonical way to declare inputs (type, validation, visibility) that UI, engine, and plugins can share without duplication.
- **Technical problem:** Schema must be JSON-serializable for transport/persistence, while Rust provides strong typing at the schema boundary; runtime values are dynamic.

## Current Architecture

- **Module map:**
  - `kind.rs` — `ParameterKind`, `ParameterCapability`, value type mapping
  - `def.rs` — `ParameterDef` tagged enum with delegation helpers
  - `types/` — concrete parameter structs per kind (text, number, object, list, mode, etc.)
  - `metadata.rs` — `ParameterMetadata` (key, name, hints, required, sensitive)
  - `validation.rs` — `ValidationRule` declarative schema
  - `display.rs` — `ParameterDisplay`, `DisplayRuleSet`, `DisplayCondition`, `DisplayContext`
  - `collection.rs` — ordered schema collection + validation pipeline
  - `values.rs` — runtime key→value map, snapshot/diff
  - `option.rs` — `SelectOption`, `OptionsSource`
  - `error.rs` — `ParameterError` classification
- **Data/control flow:** Schema defined in Rust → serialized to JSON → consumed by UI/engine; values flow as `serde_json::Value`; validation runs at boundaries before execution.
- **Known bottlenecks:** Deep nested object/list validation allocates per-path; large collections with many errors may allocate heavily.

## Target Architecture

- **Target module map:** Same structure; add optional `lint` module for schema preflight.
- **Public contract boundaries:** `ParameterCollection`, `ParameterValues`, `ParameterDef`, `ParameterError` are the stable surface.
- **Internal invariants:** Keys unique within scope; display rules reference sibling keys only; validation rules match kind capabilities.

## Design Reasoning

- **Schema-first, value-later:** Enables transport/storage compatibility and dynamic workflow execution; type guarantees end at schema boundary.
- **Declarative validation rules:** Serializable rules evaluated in validation layer; enables persistence, tooling, cross-layer portability.
- **Capability-based kind semantics:** `ParameterCapability` flags instead of ad-hoc checks; clearer downstream logic for UI/render/engine.
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

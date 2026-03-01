# nebula-parameter Constitution

> **Version**: 1.0.0 | **Created**: 2026-03-01

---

## Platform Role

Workflow nodes need configurable inputs: text, number, select, object, list, credentials, and more. The UI must render forms, validate user input, and support conditional visibility. The runtime must resolve values (including expressions) and pass them to actions. A single schema layer for parameter kinds, validation rules, and value containers keeps UI, engine, and actions in sync.

**nebula-parameter is the node input schema and value layer for the Nebula workflow platform.**

It answers: *What parameter kinds exist, what validation and display rules do they have, and how are runtime values stored and diffed?*

```
Action (or node) declares parameters: ParameterDef / ParameterCollection (kind, key, validation, show_when)
    ↓
UI renders form from schema; user or API submits values
    ↓
ParameterValues container holds runtime values; ParameterCollection::validate() runs (nebula-validator)
    ↓
Engine/expression resolve expressions in values; result passed to action
```

This is the parameter contract: schema is declarative and JSON-serializable; validation is composable; values are separate from schema so runtime can snapshot and diff.

---

## User Stories

### Story 1 — Action Author Declares Parameters (P1)

An action author defines parameters: "channel" (Select), "message" (Text), "attachments" (List of Object). Each has key, name, validation (required, pattern, min/max), and optional show_when. Schema is serializable for UI and API.

**Acceptance**:
- Parameter kinds (19+ variants) cover Text, Number, Select, Object, List, Mode, Credential, etc.
- Metadata: key, name, hints, required, sensitive
- Validation rules (min, max, pattern, OneOf) declarative
- Conditional display (show_when, hide_when) without UI logic in crate

### Story 2 — UI Renders Form and Validates Input (P1)

The desktop or web UI loads parameter schema and renders form. On submit, validation runs; errors are aggregated and shown by field. No widget layout in parameter crate — only schema and validation.

**Acceptance**:
- Schema is JSON-serializable; UI consumes it
- Validation pipeline returns ValidationErrors with field path and message
- Sensitive parameters are marked; UI redacts in forms and logs

### Story 3 — Runtime Holds Values and Supports Snapshot/Diff (P2)

Engine or runtime holds ParameterValues for the current node. It can snapshot (for checkpoint) and diff (for change detection). Expression resolution is done by expression crate using context that includes parameter values.

**Acceptance**:
- ParameterValues is the runtime value container
- Snapshot/diff utilities for state and debugging
- No expression evaluation in parameter crate — values are raw or resolved by caller

### Story 4 — Validation Order and Error Codes Are Stable (P2)

Validation runs in deterministic order; error codes and field paths are stable so that API and UI can show consistent errors and migration can rely on compatibility.

**Acceptance**:
- Validation order documented; error aggregation deterministic
- Stable error codes; compatibility policy in minor/major
- Contract fixtures for error shape (see validator/config integration)

---

## Core Principles

### I. Schema Is Declarative and Serializable

**Parameter definitions are data: kinds, keys, validation rules, display conditions. No executable logic in schema.**

**Rationale**: UI and API must consume schema without depending on Rust. Declarative schema allows multiple UIs and versioning.

**Rules**:
- All definition types Serialize/Deserialize
- No closures or function pointers in schema
- Conditional logic is data (e.g. show_when expression string or structure)

### II. Validation Is Composable and Crate-Owned

**Validation rules (min, max, pattern, OneOf, etc.) are defined in parameter crate; optional integration with nebula-validator for combinators.**

**Rationale**: Parameter validation is a core responsibility. Reuse validator crate where it fits; parameter owns the parameter-specific rules and aggregation.

**Rules**:
- Validation pipeline produces structured errors (field path, code, message)
- Deterministic order; no dependency on UI or engine for validation logic
- Optional bridge to nebula-validator for consistency with config/API

### III. Values Are Separate From Schema

**Runtime values live in ParameterValues; schema lives in definitions. Same schema can have many value instances.**

**Rationale**: Multiple executions or nodes share schema but have different values. Snapshot and diff operate on values.

**Rules**:
- ParameterValues is the value container
- No schema embedded in values; reference by key or definition set
- Expression resolution is out of scope — caller (engine/expression) resolves

### IV. No UI Layout or Widget Types

**Parameter crate defines kinds and validation; it does not define widgets, layout, or styling.**

**Rationale**: UI belongs to desktop/web apps. Parameter is the contract both can use.

**Rules**:
- No button, layout, or style types in parameter crate
- Hints and display rules are data only (e.g. placeholder string, show_when)

### V. Credential and Expression Are Consumers

**Credential resolution and expression evaluation are handled by credential and expression crates. Parameter only marks "this is a credential ref" or "this value may be an expression".**

**Rationale**: Single responsibility. Parameter declares; others resolve.

**Rules**:
- CredentialRef in schema; credential crate supplies value at runtime
- Expression syntax in values is resolved by expression crate; parameter holds raw or resolved value per contract

---

## Production Vision

### The parameter layer in an n8n-class fleet

In production, every action and trigger declares parameters via this schema. UI loads schema and renders forms; API accepts and validates submissions. Engine loads workflow with parameter definitions and current values; at execution time it resolves expressions and passes resolved values to actions. Validation errors are stable and machine-readable for API and migration.

```
ParameterDef (serde tag "type") — Text, Textarea, Code, Secret, Number, Checkbox, Select, MultiSelect,
    Color, DateTime, Date, Time, Hidden, Notice, Object, List, Mode, Group, Expirable
    ├── metadata: ParameterMetadata (key, name, description, required, placeholder, hint, sensitive)
    ├── display: Option<ParameterDisplay> (show_when, hide_when → DisplayRuleSet / DisplayCondition)
    ├── validation: Vec<ValidationRule> (MinLength, MaxLength, Pattern, Min, Max, OneOf, MinItems, MaxItems, Custom)
    └── type-specific: default, options, fields, item_template, variants, inner, etc.

ParameterCollection — Vec<ParameterDef>; get/get_by_key, validate(&ParameterValues) → Result<(), Vec<ParameterError>>
ParameterValues — HashMap<String, Value> (serde flatten); get/set, snapshot(), diff(); validate via collection
ParameterKind / ParameterCapability — kind.capabilities(), value_type(), is_editable(), etc.
ParameterError — InvalidKeyFormat, NotFound, AlreadyExists, InvalidType, InvalidValue, MissingValue, ValidationError, DeserializationError, SerializationError; category(), code()
```

Validation: `ParameterCollection::validate(&values)` runs required check, type check, then rule evaluation via nebula-validator; Custom rules skipped (expression engine). Error aggregation (no fail-fast). Schema collection and validation pipeline produce a single validation result; error codes and field paths are stable across versions per compatibility policy.

### From the archives: parameter kinds and validation

The archive (`docs/crates/parameter/_archive/`: pre-spec-ARCHITECTURE.md, pre-spec-API.md, pre-spec-DECISIONS.md, pre-spec-PROPOSALS.md, pre-spec-README.md, pre-spec-ROADMAP.md; archive-ideas.md, archive-node-development.md, archive-node-execution.md, archive-crates-dependencies.md, archive-nebula-parameters*.md) describes parameter kinds, validation rules, and value containers. Production vision: keep schema declarative, validation deterministic, values separate; optional typed value layer and schema lint pass as gaps.

### Key gaps from current state to prod

| Gap | Priority | Notes |
|-----|----------|-------|
| Stable error codes and compatibility policy | High | Document and fixture; minor = additive only |
| Deterministic validation order | High | Document and test |
| Schema lint pass (cycles in show_when, invalid refs) | Medium | Preflight before runtime |
| Typed value layer (optional) | Low | Reduce raw JSON type mismatches at runtime |
| Display rule cycle detection | Medium | show_when dependency cycle could hang UI |

---

## Key Decisions

### D-001: 19+ Parameter Kinds in One Crate

**Decision**: All parameter kinds (Text, Number, Select, Object, List, Mode, Credential, etc.) live in nebula-parameter.

**Rationale**: Single schema vocabulary for UI and engine. Splitting would duplicate shared concepts.

**Rejected**: Per-domain parameter crates — would fragment schema and validation.

### D-002: Validation in Parameter Crate

**Decision**: Validation rules and pipeline are in parameter crate; optional use of nebula-validator for combinators or error shape.

**Rationale**: Parameter-specific rules (OneOf, conditional) belong here; consistency with config/API validation via validator bridge.

**Rejected**: Validation only in validator crate — would require validator to know all parameter kinds.

### D-003: No Expression Resolution in Parameter

**Decision**: Parameter holds raw values or expression strings; expression crate resolves when needed.

**Rationale**: Expression engine owns evaluation; parameter owns schema and value container.

**Rejected**: Parameter evaluating expressions — would create dependency and duplicate logic.

### D-004: JSON-Serializable Schema

**Decision**: All public definition types are Serialize/Deserialize for API and UI consumption.

**Rationale**: Web and desktop UIs consume schema over API or IPC. Rust-only types would block that.

**Rejected**: Internal-only schema — would force every UI to duplicate schema definition.

---

## Open Proposals

### P-001: Typed Value Layer

**Problem**: Raw JSON values push type mismatch detection late.

**Proposal**: Optional typed accessors or TypedParameterValues<T> for known shapes; fallback to raw for dynamic.

**Impact**: Additive; may complicate generic code.

### P-002: Schema Lint and Cycle Detection

**Problem**: show_when cycles or invalid refs can cause hangs or confusing errors.

**Proposal**: Lint pass that checks show_when dependency graph and ref validity; run at schema load or publish.

**Impact**: Additive; new API or CLI.

### P-003: Error Code Registry and Compatibility Fixtures

**Problem**: Error codes and field paths must stay stable across minor versions.

**Proposal**: Formal error registry fixture and contract tests (align with validator/config); document additive-only for minor.

**Impact**: Non-breaking; improves stability.

---

## Non-Negotiables

1. **Schema is declarative and serializable** — no executable logic in schema.
2. **Validation is deterministic and stable** — order and error codes documented and fixture-locked.
3. **Values separate from schema** — ParameterValues is the runtime container.
4. **No UI layout in parameter crate** — only kinds, validation, and display rules as data.
5. **No expression or credential resolution in parameter** — only declaration; resolution in expression/credential.
6. **Breaking schema or validation contract = major + MIGRATION.md** — UI and engine depend on it.

---

## Governance

- **PATCH**: Bug fixes, docs. No change to schema or validation semantics.
- **MINOR**: Additive only (new kinds, new optional rules). No removal or behavior change of existing kinds/rules.
- **MAJOR**: Breaking changes to schema or validation. Requires MIGRATION.md.

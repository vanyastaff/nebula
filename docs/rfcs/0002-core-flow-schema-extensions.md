# RFC 0002 - Parameter Core Extensions

Status: Draft

## Summary

This RFC defines minimal parameter-schema additions for reusable form building
in nebula (used by credentials, actions, and other schema-driven UIs).

Design goal: keep nebula v2 JSON-first model intact and add missing
parameter-level semantics without introducing node/runtime behavior.

## Relation to paramdef routing.rs

`paramdef` has a dedicated `Routing` container type with options:
- `connection_label`
- `connection_required`
- `max_connections`

In nebula v2 we do not introduce a new top-level `Field::Routing` type.
Instead, we add presets and validation rules in the existing JSON wire format.

Mapping used in this RFC is parameter-only:
- selection labels and required flags are represented with existing FieldMeta
- topology/runtime constraints are out of scope

## Problem Statement

Current v2 schema is expressive but misses several form-level capabilities:

1. Branch targets are plain text strings.
2. `switch` and `router` case uniqueness requires custom ad-hoc validation.
3. Expression fields are represented as generic `text + expression`.
4. Repeated core fragments (branch target, retry policy, timeout) are duplicated.
5. Dynamic provider contracts are underspecified.

## Proposed Additions

### 1) Branch Target Preset (Type-Driven)

Add a dedicated preset for branch target selection.

```rust
// builder API
impl Field {
    pub fn branch_target(id: &str) -> FieldBuilder;
}
```

Semantics:
- `branch_target` produces `type: "select"` with dynamic provider `workflow.branches`.
- Value payload remains a scalar branch key string.
- Serialized form:

```json
{
  "id": "fallback_branch",
    "type": "select",
    "source": "dynamic",
    "provider": "workflow.branches"
}
```

Validation:
- selected branch key must be validated against the available domain of branch keys
    when such domain is provided by the host product.
- Error code: `unknown_branch_key`.

### 2) List-Level Uniqueness Rule

Add a list/object-aware rule for unique keys in repeated items.

```rust
pub enum Rule {
    // existing...
    UniqueBy { path: String, message: Option<String> },
}
```

Example:

```rust
Field::list("cases", Field::object("_case").fields(vec![/* ... */]).build())
    .rule(Rule::unique_by("pattern", None));
```

Error path behavior:
- Report duplicate items at exact item path, e.g. `cases.3.pattern`.
- Error code: `duplicate_value`.

### 3) Expression Presets (No New Wire Type)

Keep `type: "text"` + `expression: true` and avoid additional wire markers.

Builder shortcuts:

```rust
impl Field {
    pub fn expression_bool(id: &str) -> FieldBuilder;
    pub fn expression_scalar(id: &str) -> FieldBuilder;
    pub fn expression_list(id: &str) -> FieldBuilder;
}
```

Wire example:

```json
{
  "id": "when",
  "type": "text",
    "expression": true
}
```

### 4) Core Field Presets Module

Add reusable builders to reduce duplication across core actions.

```rust
pub mod core_fields {
    pub fn branch_target(id: &str) -> FieldBuilder;      // select + dynamic("workflow.branches")
    pub fn signal_channel(id: &str) -> FieldBuilder;     // select + dynamic("eventbus.channels")
    pub fn retry_policy(id: &str) -> FieldBuilder;       // object with attempts/backoff
    pub fn timeout_ms(id: &str) -> FieldBuilder;         // integer number with min/max
}
```

### 5) Typed Dynamic Source Example (`eventbus.channels`)

Define a canonical dynamic provider example for channel selection.

Provider id:
- `eventbus.channels`

Recommended field shape:

```json
{
    "id": "channel",
    "type": "select",
    "source": "dynamic",
    "provider": "eventbus.channels",
    "searchable": true
}
```

Provider response contract (logical):
- item `value`: stable channel key (e.g. `orders.approved`)
- item `label`: display name
- optional `description`: human-readable hint

Behavior:
- if provider is unavailable, keep previously selected value but mark field invalid on save.
- unknown selected value returns `unknown_channel`.

### 6) Typed Dynamic Source Example (`workflow.branches`)

Define a canonical dynamic provider example for branch references.

Provider id:
- `workflow.branches`

Recommended field shape:

```json
{
    "id": "branch_key",
    "type": "select",
    "source": "dynamic",
    "provider": "workflow.branches",
    "searchable": true
}
```

Provider response contract (logical):
- item `value`: stable branch key/edge id
- item `label`: branch display name (e.g. `priority`, `default`, `on_error`)
- optional `description`: route hint

Behavior:
- if a previously selected value disappears from provider output, the value remains
    visible but fails validation.
- unresolved selected value returns `unknown_branch_key`.

## Non-Goals

- Do not add `Field::Routing` container in this RFC.
- Do not change `Mode` value shape.
- Do not define runtime/node execution behavior.

## Backward Compatibility

- Fully backward compatible.
- New features are additive.
- Existing schemas using plain text branch fields continue to work.

## Examples

### Branch reference field

```rust
Field::branch_target("fallback_branch")
    .label("Fallback Branch")
    .required();

// Typed variant with autocomplete
Field::branch_target("fallback_branch")
    .required();
```

### Switch cases with uniqueness

```rust
Field::list("cases",
    Field::object("_case")
        .fields(vec![
            Field::text("pattern").required().build(),
            Field::branch_target("branch_key").required().build(),
        ])
        .build()
)
.rule(Rule::unique_by("pattern", None))
.rule(Rule::unique_by("branch_key", None));
```

### If condition boolean expression

```rust
Field::expression_bool("when")
    .label("Condition")
    .required()
    .placeholder("{{ $json.total > 1000 }}");
```

## Validation Contract

New error codes:
- `unknown_branch_key`
- `duplicate_value`
- `unknown_channel`

## Open Questions

1. Should `branch_target` allow manual text fallback when providers are unavailable?
2. Should `unique_by` support deep paths (`config.key`) only, or array wildcards later?
3. Should expression presets enforce runtime type checks strictly or best-effort?
4. Should dynamic providers support standard scoped filtering via `depends_on`?
5. Should we define a strict provider response schema versioning strategy?

## Rollout Plan

1. Implement `branch_target` preset using dynamic provider contract.
2. Implement `Rule::unique_by` for list/object validation.
3. Add expression preset builder shortcuts.
4. Define and implement canonical dynamic provider contracts.
5. Add contract tests for provider payload shape.
6. Add `core_fields` helper module (including `signal_channel` and `branch_target`).
7. Migrate core action schemas to presets incrementally.

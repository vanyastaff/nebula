# RFC 0004 — New Field Types: DynamicRecord and Predicate

**Status:** Draft  
**Created:** 2026-03-08  
**Target:** `nebula-parameter` v1.x

---

## Summary

Two new `Field` variants discovered through analysis of core flow-control nodes
(IF, Switch, Router, Filter) and resource-mapper nodes (Google Sheets, Airtable,
any DB INSERT node).

Both types are impossible to express adequately through existing primitives
without losing either semantic meaning or UI fidelity.

For Nebula, acceptance is not only about expressiveness. Any new field type
must satisfy three constraints:

1. Universality: reusable across many node families, not provider-specific hacks.
2. Maintainability: deterministic serialization and stable validation contracts.
3. Implementability: realistic MVP within current crate boundaries.

Decision in this RFC:

- `Predicate`: adopt as a first-class type.
- `DynamicRecord`: adopt with strict provider-contract constraints.

---

## 1. `Field::DynamicRecord` — Runtime-Defined Field Set

### Problem

Some nodes map user-provided values to a target schema that is only known at
runtime. Examples:

- **Google Sheets "Add Row"** — columns depend on the selected spreadsheet and sheet
- **Airtable "Create Record"** — fields depend on the selected base and table  
- **PostgreSQL "Insert Row"** — columns depend on the connected DB and selected table
- **Notion "Create Page"** — properties depend on the selected database

The target schema arrives from an async provider (same mechanism as `OptionSource::Dynamic`),
but instead of a list of *options* it returns a list of *fields*. The user fills
values for each returned field.

**Why `List<Object>` does not work here:**

`List<Object { field_name: Select(dynamic), value: Text(expression) }>` — this is
the Pairs pattern (RFC 0002) and it works as a generic fallback, but it loses:

1. Field-specific types — a number column should render a number input, a date
   column a date picker, a boolean a toggle.
2. Required/optional per-column semantics from the target schema.
3. Field descriptions and metadata from the target system.
4. The provider must return a full `Field` list, not just option labels.

### Proposed Type

```rust
/// A field whose schema is provided by a runtime provider.
///
/// The provider returns a list of `DynamicFieldSpec` definitions at runtime.
/// The user fills values for each returned field.
/// Value shape: `{ "field_id": <value>, "field_id2": <value>, ... }`.
DynamicRecord {
    #[serde(flatten)]
    meta: FieldMeta,
    /// Provider key. Returns `Vec<DynamicFieldSpec>`.
    provider: String,
    /// Re-fetch fields when these sibling fields change.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    depends_on: Vec<String>,
    /// Show only a subset of returned fields initially.
    /// "required_only" | "all" (default: "all")
    #[serde(default = "DynamicRecordMode::all")]
    mode: DynamicRecordMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DynamicRecordMode {
    #[default]
    All,
    /// Show only fields the provider marks as required; let user add optional ones.
    RequiredOnly,
}
```

### Provider Contract

The provider returns `DynamicRecordPage`:

```rust
pub struct DynamicRecordPage {
    /// Dynamic field specs to render.
    /// This is intentionally NOT the full `Field` enum.
    pub fields: Vec<DynamicFieldSpec>,
    /// Pagination cursor (None = all fields returned).
    pub next_cursor: Option<String>,
    /// Provider schema snapshot/version for deterministic persistence.
    pub schema_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DynamicFieldSpec {
    Text {
        id: String,
        label: String,
        required: bool,
        rules: Vec<Rule>,
    },
    Number {
        id: String,
        label: String,
        required: bool,
        integer: bool,
        rules: Vec<Rule>,
    },
    Boolean {
        id: String,
        label: String,
        required: bool,
    },
    Select {
        id: String,
        label: String,
        required: bool,
        options: Vec<SelectOption>,
        multiple: bool,
    },
}
```

The provider has the same interface as `OptionProvider` but resolves `DynamicRecordPage`
instead of `OptionPage`. Registered under the same provider registry, differentiated
by return type at the call site.

Constraints for maintainability:

- `dynamic_record` cannot recursively return `dynamic_record`.
- No provider-returned executable expressions.
- Provider field order is canonical and must be preserved.

### JSON Schema

```json
{
  "id": "row_data",
  "type": "dynamic_record",
  "label": "Row Data",
  "provider": "sheets.columns",
  "depends_on": ["spreadsheet_id", "sheet_id"]
}
```

### Value Shape

```json
{
  "row_data": {
    "Name": "Alice",
    "Age": 30,
    "Active": true,
    "Joined": "2024-01-15"
  }
}
```

Keys are the `id`s returned by the provider in its `Field` list.

### Stale / Unavailable Behavior

- Provider unavailable → keep previously saved values, render as plain text
  with warning notice. Do not clear.
- Provider returns new schema → fields not present in saved values render as
  empty (default value from provider field definition if available).
- Fields removed from schema → keep in saved values but hide from UI.
  Validate on save: unknown fields are handled by explicit policy.

Unknown-field policy:

- `warn_keep` (default): keep unknown values, emit warning.
- `strip`: remove unknown values on save.
- `error`: fail validation with a structured error.

### Canonical Providers

| Provider key       | Returns columns for              |
|--------------------|----------------------------------|
| `sheets.columns`   | Google Sheets sheet columns      |
| `airtable.fields`  | Airtable table fields            |
| `db.columns`       | SQL table columns                |
| `notion.properties`| Notion database properties       |

### Builder Example

```rust
// Google Sheets — Add Row
Field::dynamic_record("row_data")
    .label("Row Data")
    .provider("sheets.columns")
    .depends_on(&["spreadsheet_id", "sheet_id"])
    .mode(DynamicRecordMode::RequiredOnly)
```

---

## 2. `Field::Predicate` — Runtime Data Condition Builder

### Problem

Flow-control nodes (IF, Filter, Switch) need users to define conditions that are
evaluated against the workflow's data payload at execution time:

- **IF node** — "continue on branch A if `order.total > 100`"
- **Filter node** — "keep items where `status == 'active'`"
- **Switch node** — each case has a condition that routes to a branch

This is categorically different from the schema's existing `Condition` type,
which controls *form field visibility*. `Predicate` operates on *runtime data*,
not on form state.

**Why `Text(expression: true)` is insufficient for community developers:**

Expression strings (`{{ $json.total > 100 }}`) require knowing the expression
language. A visual condition builder is essential for non-technical users and
is the standard across every workflow system (n8n Filter, Make Filter,
Node-RED Switch, Power Automate Condition).

**Why `List<Object>` is insufficient:**

A flat list of condition objects works for simple AND/all cases. It cannot
express nested AND/OR groups, which are required for real filtering logic:

```
(status == "active" OR vip == true) AND total > 100
```

### Proposed Type

```rust
/// A structured condition builder operating on runtime workflow data.
///
/// Renders as a visual rule-builder UI. Value is a structured tree
/// evaluated by the runtime against the data payload.
Predicate {
    #[serde(flatten)]
    meta: FieldMeta,
    /// Subset of operators to expose. None = all operators shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    operators: Option<Vec<PredicateOp>>,
    /// Allow nested AND/OR groups. Default: true.
    #[serde(default = "default_true")]
    allow_groups: bool,
    /// Maximum nesting depth for groups. Default: 3.
    #[serde(default = "default_depth")]
    max_depth: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PredicateOp {
    // Equality
    Eq, Ne,
    // Numeric
    Gt, Gte, Lt, Lte,
    // String
    Contains, NotContains, StartsWith, EndsWith,
    // Presence
    IsSet, IsEmpty,
    // Type checks
    IsTrue, IsFalse,
    // Array
    InList, NotInList,
    // Regex
    Matches,
}
```

### Value Shape

The value is a recursive tree. Two node types: `rule` and `group`.

```typescript
type PredicateExpr = PredicateGroup;

interface PredicateGroup {
    type: "group";
    combine: "and" | "or";
    rules: Array<PredicateRule | PredicateGroup>;
}

interface PredicateRule {
    type: "rule";
    field: string;   // canonical dot-path with optional indices: "order.total", "items[0].name"
    op: PredicateOp;
    value?: unknown; // absent for IsSet, IsEmpty, IsTrue, IsFalse
}
```

**Simple AND (most common):**

```json
{
  "type": "group",
  "combine": "and",
  "rules": [
    { "type": "rule", "field": "status", "op": "eq", "value": "active" },
    { "type": "rule", "field": "total", "op": "gt", "value": 100 }
  ]
}
```

**Nested AND/OR:**

```json
{
  "type": "group",
  "combine": "and",
  "rules": [
    { "type": "rule", "field": "total", "op": "gte", "value": 100 },
    {
      "type": "group",
      "combine": "or",
      "rules": [
        { "type": "rule", "field": "status", "op": "eq", "value": "active" },
        { "type": "rule", "field": "vip", "op": "is_true" }
      ]
    }
  ]
}
```

### JSON Schema

```json
{
  "id": "condition",
  "type": "predicate",
  "label": "Condition",
  "required": true
}
```

Restricted to numeric comparisons only (for a "price filter" node):

```json
{
  "id": "price_filter",
  "type": "predicate",
  "label": "Price Filter",
  "operators": ["eq", "ne", "gt", "gte", "lt", "lte"],
  "allow_groups": false
}
```

### Usage in Core Nodes

**IF node:**

```rust
Field::predicate("condition")
    .label("Condition")
    .required()
```

Value routes to branch `true` or `false` depending on evaluation.

**Filter node (filters items in a list):**

```rust
Field::predicate("filter")
    .label("Keep items where")
    .required()
```

Applied per-item. Items where condition is false are dropped.

**Switch node cases:**

Switch cases are `List<Object>` where each case contains a `Predicate`
restricted to a single rule (no groups), plus a `branch_target`:

```rust
Field::list("cases",
    Field::object("_case")
        .fields(vec![
            Field::predicate("condition")
                .label("When")
                .allow_groups(false)
                .required()
                .build(),
            Field::branch_target("branch_key")   // RFC 0002
                .label("Go to Branch")
                .required()
                .build(),
        ])
        .build()
)
.rule(Rule::unique_by("branch_key", None))
```

For Switch, `allow_groups: false` and `max_depth: 1` reduces each case to a
single condition — readable and sufficient for routing logic.

### Validation Errors

```json
[
  {
    "path": "condition.rules.0.field",
    "code": "required",
    "message": "Field path is required"
  },
  {
    "path": "condition.rules.1.value",
    "code": "required",
    "message": "Comparison value is required for operator 'eq'"
  }
]
```

### Runtime Evaluation

The runtime evaluates `PredicateExpr` against a `serde_json::Value` payload.
In v1, field paths use canonical dot-path syntax with indices. JSONPath can be
added later as a compatibility adapter if needed.
Recommended: implement as a simple recursive evaluator in `nebula-core` — no
external crate needed for the rule/group tree structure.

```rust
pub fn evaluate(expr: &PredicateExpr, data: &serde_json::Value) -> bool;
```

Determinism and complexity limits:

- Preserve `rules` array order as canonical order.
- Canonicalize group trees before hashing/diffing.
- Validate bounded complexity: `max_depth`, `max_rules`, and max field-path length.

---

## Impact on Existing Schema Types

No breaking changes. Both types are additive to the `Field` enum.

`OptionProvider` and `DynamicRecordProvider` share the same registration
mechanism — differentiated by the type they resolve. Consider a unified
`Provider` trait with associated response type, or two separate trait objects.

---

## Builder Summary

```rust
// DynamicRecord
Field::dynamic_record("row_data")
    .label("Row Data")
    .provider("sheets.columns")
    .depends_on(&["spreadsheet_id", "sheet_id"])

// Predicate — full
Field::predicate("condition")
    .label("Condition")
    .required()

// Predicate — single rule only (Switch case condition)
Field::predicate("when")
    .label("When")
    .allow_groups(false)
    .required()

// Predicate — operators restricted
Field::predicate("price_filter")
    .label("Price Filter")
    .operators(&[PredicateOp::Gt, PredicateOp::Lt, PredicateOp::Gte, PredicateOp::Lte])
```

---

## Open Questions

1. Should expression-based field selectors be added later as an opt-in adapter,
  while keeping dot-path as the canonical stored form?

2. Should `DynamicRecord` allow the user to override field types returned
   by the provider (e.g., force a column to be treated as text)?

3. Should `PredicateOp` be extensible by community nodes, or fixed enum?
   Fixed enum is safer for cross-language serialization.

4. For `DynamicRecord` in `RequiredOnly` mode: should optional fields be
   accessible via an "Add field" button, or hidden entirely?

---

## Rollout Plan

1. Add `Field::Predicate` variant and `PredicateExpr` tree types.
2. Implement `evaluate(expr, data)` in `nebula-core`.
3. Implement IF and Filter core nodes using `Predicate`.
4. Implement Switch core node using `List<Object { Predicate(allow_groups:false), branch_target }>`.
5. Add `Field::DynamicRecord` variant and `DynamicRecordProvider` trait.
6. Implement `sheets.columns` canonical provider as first reference implementation.
7. Ship `DynamicRecord` behind a feature flag (`dynamic-record`) until at least
  two reference providers prove compatibility (`sheets.columns`, `airtable.fields`).
8. Document provider contract for community node authors.


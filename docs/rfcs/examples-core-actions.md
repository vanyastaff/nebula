# Core Actions - Field Schema Examples (Parameter Schema v2)

Examples of core workflow actions built with the v2 parameter schema.
Goal: validate control-flow UX (`if`, `switch`, `router`, etc.) and reveal
what may still be missing in the core schema.

This document follows the nebula atomic action model:
one action = one schema = one operation.

Related proposal: `docs/rfcs/0002-core-flow-schema-extensions.md`
(maps useful `paramdef` routing ideas into nebula v2 JSON-first schema).

Cross-platform analysis: `docs/rfcs/0003-cross-platform-core-nodes-gap-analysis.md`
(n8n/Node-RED/Make/Power Automate patterns mapped to nebula v2).

---

## Table of Contents

1. [If Action](#1-if-action)
2. [Switch Action](#2-switch-action)
3. [Router Action](#3-router-action)
4. [ForEach Action](#4-foreach-action)
5. [Merge Action](#5-merge-action)
6. [Wait Action](#6-wait-action)
7. [Gaps Found in Core](#7-gaps-found-in-core)

---

## 1. If Action

`If` evaluates a predicate and sends execution to `true` or `false` branch.

### JSON Schema

```json
{
  "fields": [
    {
      "id": "condition",
      "type": "mode",
      "label": "Condition",
      "required": true,
      "default_variant": "expression",
      "variants": [
        {
          "key": "expression",
          "label": "Expression",
          "content": {
            "id": "expr",
            "type": "text",
            "label": "Expression",
            "expression": true,
            "placeholder": "{{ $json.total > 1000 }}",
            "required": true
          }
        },
        {
          "key": "field_compare",
          "label": "Field Compare",
          "content": {
            "id": "config",
            "type": "object",
            "fields": [
              {
                "id": "left",
                "type": "text",
                "label": "Left",
                "expression": true,
                "required": true,
                "placeholder": "{{ $json.status }}"
              },
              {
                "id": "op",
                "type": "select",
                "label": "Operator",
                "required": true,
                "source": "static",
                "default": "eq",
                "options": [
                  { "value": "eq", "label": "=" },
                  { "value": "ne", "label": "!=" },
                  { "value": "gt", "label": ">" },
                  { "value": "gte", "label": ">=" },
                  { "value": "lt", "label": "<" },
                  { "value": "lte", "label": "<=" },
                  { "value": "contains", "label": "contains" }
                ]
              },
              {
                "id": "right",
                "type": "text",
                "label": "Right",
                "expression": true,
                "required": true,
                "placeholder": "completed"
              },
              {
                "id": "ignore_case",
                "type": "boolean",
                "label": "Ignore Case",
                "default": false,
                "visible_when": { "op": "eq", "field": "op", "value": "contains" }
              }
            ]
          }
        }
      ]
    }
  ],
  "groups": [
    { "label": "Logic", "fields": ["condition"] }
  ]
}
```

### Value (expression mode)

```json
{
  "condition": {
    "mode": "expression",
    "value": "{{ $json.total > 1000 && $json.country == 'US' }}"
  }
}
```

---

## 2. Switch Action

`Switch` maps a computed value to one of N named branches.

### JSON Schema

```json
{
  "fields": [
    {
      "id": "input",
      "type": "text",
      "label": "Switch Input",
      "expression": true,
      "required": true,
      "placeholder": "{{ $json.event_type }}"
    },
    {
      "id": "match_mode",
      "type": "select",
      "label": "Match Mode",
      "required": true,
      "default": "exact",
      "source": "static",
      "options": [
        { "value": "exact", "label": "Exact" },
        { "value": "regex", "label": "Regex" },
        { "value": "prefix", "label": "Prefix" }
      ]
    },
    {
      "id": "cases",
      "type": "list",
      "label": "Cases",
      "min_items": 1,
      "item": {
        "id": "_case",
        "type": "object",
        "label": "Case",
        "fields": [
          {
            "id": "pattern",
            "type": "text",
            "label": "Pattern",
            "required": true,
            "placeholder": "user.created"
          },
          {
            "id": "branch_key",
            "type": "select",
            "label": "Branch Key",
            "required": true,
            "source": "dynamic",
            "provider": "workflow.branches",
            "searchable": true
          },
          {
            "id": "stop_after_match",
            "type": "boolean",
            "label": "Stop After Match",
            "default": true
          }
        ]
      }
    },
    {
      "id": "default_branch",
      "type": "select",
      "label": "Default Branch",
      "source": "dynamic",
      "provider": "workflow.branches",
      "searchable": true
    }
  ],
  "groups": [
    { "label": "Switch", "fields": ["input", "match_mode", "cases", "default_branch"] }
  ]
}
```

### Value

```json
{
  "input": "{{ $json.event_type }}",
  "match_mode": "exact",
  "cases": [
    { "pattern": "user.created", "branch_key": "route_user_created", "stop_after_match": true },
    { "pattern": "invoice.paid", "branch_key": "route_invoice_paid", "stop_after_match": true }
  ],
  "default_branch": "route_default"
}
```

---

## 3. Router Action

`Router` evaluates ordered rules and dispatches to one or many branches.

### JSON Schema

```json
{
  "fields": [
    {
      "id": "strategy",
      "type": "select",
      "label": "Routing Strategy",
      "required": true,
      "default": "first_match",
      "source": "static",
      "options": [
        { "value": "first_match", "label": "First Match" },
        { "value": "all_matches", "label": "All Matches" }
      ]
    },
    {
      "id": "rules",
      "type": "list",
      "label": "Routes",
      "min_items": 1,
      "item": {
        "id": "_rule",
        "type": "object",
        "fields": [
          {
            "id": "name",
            "type": "text",
            "label": "Route Name",
            "required": true,
            "placeholder": "high_value_us"
          },
          {
            "id": "when",
            "type": "text",
            "label": "Condition",
            "required": true,
            "expression": true,
            "placeholder": "{{ $json.total > 1000 && $json.country == 'US' }}"
          },
          {
            "id": "branch_key",
            "type": "select",
            "label": "Branch Key",
            "required": true,
            "source": "dynamic",
            "provider": "workflow.branches",
            "searchable": true
          },
          {
            "id": "priority",
            "type": "number",
            "label": "Priority",
            "integer": true,
            "default": 100
          },
          {
            "id": "enabled",
            "type": "boolean",
            "label": "Enabled",
            "default": true
          }
        ]
      }
    },
    {
      "id": "fallback_branch",
      "type": "select",
      "label": "Fallback Branch",
      "source": "dynamic",
      "provider": "workflow.branches",
      "searchable": true
    }
  ],
  "groups": [
    { "label": "Router", "fields": ["strategy", "rules", "fallback_branch"] }
  ]
}
```

### Value

```json
{
  "strategy": "first_match",
  "rules": [
    {
      "name": "high_value_us",
      "when": "{{ $json.total > 1000 && $json.country == 'US' }}",
      "branch_key": "route_priority",
      "priority": 10,
      "enabled": true
    },
    {
      "name": "eu_orders",
      "when": "{{ ['DE', 'FR', 'IT'].includes($json.country) }}",
      "branch_key": "route_eu",
      "priority": 20,
      "enabled": true
    }
  ],
  "fallback_branch": "route_default"
}
```

---

## 4. ForEach Action

`ForEach` iterates an array and executes a child branch per item.

### JSON Schema

```json
{
  "fields": [
    {
      "id": "items_expr",
      "type": "text",
      "label": "Items Expression",
      "required": true,
      "expression": true,
      "placeholder": "{{ $json.items }}"
    },
    {
      "id": "item_alias",
      "type": "text",
      "label": "Item Alias",
      "default": "item",
      "required": true
    },
    {
      "id": "index_alias",
      "type": "text",
      "label": "Index Alias",
      "default": "index",
      "required": true
    },
    {
      "id": "concurrency",
      "type": "number",
      "label": "Concurrency",
      "integer": true,
      "default": 1,
      "min": 1,
      "max": 64
    },
    {
      "id": "continue_on_error",
      "type": "boolean",
      "label": "Continue on Item Error",
      "default": false
    }
  ],
  "groups": [
    { "label": "Loop", "fields": ["items_expr", "item_alias", "index_alias", "concurrency", "continue_on_error"] }
  ]
}
```

### Value

```json
{
  "items_expr": "{{ $json.line_items }}",
  "item_alias": "line",
  "index_alias": "i",
  "concurrency": 4,
  "continue_on_error": true
}
```

---

## 5. Merge Action

`Merge` joins multiple upstream inputs.

### JSON Schema

```json
{
  "fields": [
    {
      "id": "mode",
      "type": "select",
      "label": "Merge Mode",
      "required": true,
      "default": "append",
      "source": "static",
      "options": [
        { "value": "append", "label": "Append" },
        { "value": "by_key", "label": "By Key" },
        { "value": "zip", "label": "Zip" }
      ]
    },
    {
      "id": "left_key",
      "type": "text",
      "label": "Left Key",
      "visible_when": { "op": "eq", "field": "mode", "value": "by_key" },
      "required_when": { "op": "eq", "field": "mode", "value": "by_key" },
      "placeholder": "id"
    },
    {
      "id": "right_key",
      "type": "text",
      "label": "Right Key",
      "visible_when": { "op": "eq", "field": "mode", "value": "by_key" },
      "required_when": { "op": "eq", "field": "mode", "value": "by_key" },
      "placeholder": "user_id"
    },
    {
      "id": "prefer",
      "type": "select",
      "label": "Conflict Preference",
      "default": "right",
      "source": "static",
      "options": [
        { "value": "left", "label": "Left" },
        { "value": "right", "label": "Right" }
      ],
      "visible_when": { "op": "eq", "field": "mode", "value": "by_key" }
    }
  ],
  "groups": [
    { "label": "Merge", "fields": ["mode", "left_key", "right_key", "prefer"] }
  ]
}
```

### Value

```json
{
  "mode": "by_key",
  "left_key": "id",
  "right_key": "user_id",
  "prefer": "right"
}
```

---

## 6. Wait Action

`Wait` pauses execution until timeout/date/signal.

### JSON Schema

```json
{
  "fields": [
    {
      "id": "wait",
      "type": "mode",
      "label": "Wait Type",
      "required": true,
      "default_variant": "duration",
      "variants": [
        {
          "key": "duration",
          "label": "Duration",
          "content": {
            "id": "duration_ms",
            "type": "number",
            "label": "Duration (ms)",
            "integer": true,
            "required": true,
            "min": 1
          }
        },
        {
          "key": "until_datetime",
          "label": "Until DateTime",
          "content": {
            "id": "at",
            "type": "date_time",
            "label": "Resume At",
            "required": true
          }
        },
        {
          "key": "signal",
          "label": "Wait For Signal",
          "content": {
            "id": "config",
            "type": "object",
            "fields": [
              {
                "id": "channel",
                "type": "select",
                "label": "Channel",
                "required": true,
                "source": "dynamic",
                "provider": "eventbus.channels",
                "depends_on": ["correlation_id"],
                "searchable": true
              },
              {
                "id": "correlation_id",
                "type": "text",
                "label": "Correlation ID",
                "expression": true,
                "placeholder": "{{ $json.order_id }}"
              },
              {
                "id": "timeout_ms",
                "type": "number",
                "label": "Timeout (ms)",
                "integer": true,
                "min": 1
              }
            ]
          }
        }
      ]
    }
  ],
  "groups": [
    { "label": "Wait", "fields": ["wait"] }
  ]
}
```

### Value (signal mode)

```json
{
  "wait": {
    "mode": "signal",
    "value": {
      "channel": "orders.approved",
      "correlation_id": "{{ $json.order_id }}",
      "timeout_ms": 300000
    }
  }
}
```

---

## 7. Gaps Found in Core

These examples expose concrete missing pieces or API pain points.

1. Branch references are plain strings.
   Problem: `branch_key`, `fallback_branch`, `default_branch` have no schema-level linkage.
  Proposal: standardize `Field::branch_target(id)` preset as dynamic select backed by `workflow.branches`.

2. Expression-bearing text fields are too generic.
   Problem: `condition.when`, `items_expr`, `input` are all `text + expression` without semantic typing.
  Proposal: keep `expression: true`, but define context-specific validation expectations in docs.

3. `Switch` cases need uniqueness constraints.
   Problem: no built-in validation to enforce unique `pattern` or unique `branch_key`.
   Proposal: add list-level rules (`unique_by`) for object lists.

4. Router and Switch need explicit output contract docs.
   Problem: schema does not describe runtime behavior for no-match, multi-match, and ordering tie-breaks.
   Proposal: add action-level execution contract section in core action specs (outside field schema, but mandatory).

5. Router/Switch branch catalogs need a typed provider contract.
  Problem: without a standard provider, branch selectors degrade to free-text.
  Proposal: standardize dynamic provider `workflow.branches` for branch target fields.

6. ForEach loop context is implicit.
   Problem: aliases (`item_alias`, `index_alias`) are free text; collisions with existing context keys are possible.
   Proposal: add reserved-name validation rule and optional `readonly` system aliases.

7. Wait signal mode suggests need for typed channels/events.
  Problem: channel catalog contract is not defined yet (shape, paging, filters).
  Proposal: standardize dynamic provider `eventbus.channels` and response shape for typed channel selection.

8. Reusable field fragments are needed.
   Problem: recurring field sets (branch target, retry behavior, timeout) will duplicate across core actions.
   Proposal: builder-level helpers in core crate (`CoreFields::branch_target()`, `CoreFields::retry_policy()`).

---

## Suggested Next Actions

1. Add a `Core Field Presets` section to `docs/rfcs/0001-parameter-schema-v2.md` (or a follow-up RFC).
2. Define canonical branch key format and runtime routing semantics in `nebula-engine` docs.
3. Add list-level uniqueness validation rule (`unique_by`) in the parameter schema API.
4. Define expression validation expectations for boolean/scalar/list contexts.

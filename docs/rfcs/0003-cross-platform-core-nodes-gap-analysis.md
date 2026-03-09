# RFC 0003: Cross-Platform Form Parameter Gap Analysis

**Type:** Informational RFC
**Status:** Draft
**Created:** 2026-03-08
**Updated:** 2026-03-08
**Depends on:** None
**Supersedes:** None

## Scope

This RFC is strictly about parameter schema for form construction.
It does not define workflow node behavior, engine execution semantics, or graph contracts.

Target usage:
- credential forms
- action parameter forms
- other schema-driven UI forms in nebula

## Sources Reviewed

Cross-platform references were used only to extract form/UX patterns:
- n8n (If/Switch/Merge/Wait configuration UIs)
- Node-RED (Change/Switch/Template configuration UIs)
- Make (Iterator/Aggregator/Repeater configuration UIs)
- Power Automate Desktop (Flow-control action property panes)

## Reusable Form Patterns

1. Rule lists with ordered evaluation (`list<object>`) and stop/continue toggles.
2. Discriminated configuration modes (`mode`) with variant-specific payload shape.
3. Dynamic option catalogs (`select + provider`) with searchable UX.
4. Expression-enabled fields with expected value categories (bool/scalar/list).
5. Structured error policy forms (retry config, fallback selection).
6. Mapping tables (`list<object>`) for argument and property transform configuration.

## Parameter-Only Schema Examples

### A) Transform Config (declarative set/move/rename/delete)

```json
{
  "fields": [
    {
      "id": "ops",
      "type": "list",
      "label": "Operations",
      "min_items": 1,
      "item": {
        "id": "_op",
        "type": "object",
        "fields": [
          {
            "id": "kind",
            "type": "select",
            "required": true,
            "source": "static",
            "options": [
              { "value": "set", "label": "Set" },
              { "value": "move", "label": "Move" },
              { "value": "rename", "label": "Rename" },
              { "value": "delete", "label": "Delete" }
            ]
          },
          {
            "id": "path",
            "type": "text",
            "rules": [{ "rule": "pattern", "pattern": "^\\$?(\\.[a-zA-Z_][a-zA-Z0-9_]*|\\[[0-9]+\\])+$" }],
            "required": true
          },
          {
            "id": "value_expr",
            "type": "text",
            "expression": true,
            "visible_when": { "op": "eq", "field": "kind", "value": "set" }
          },
          {
            "id": "from_path",
            "type": "text",
            "rules": [{ "rule": "pattern", "pattern": "^\\$?(\\.[a-zA-Z_][a-zA-Z0-9_]*|\\[[0-9]+\\])+$" }],
            "visible_when": { "op": "any_of", "field": "kind", "values": ["move", "rename"] }
          }
        ]
      }
    }
  ]
}
```

### B) Error Policy Form (configuration only)

```json
{
  "fields": [
    {
      "id": "retry_policy",
      "type": "mode",
      "default_variant": "none",
      "variants": [
        { "key": "none", "label": "No Retry", "content": { "id": "_", "type": "hidden" } },
        {
          "key": "fixed",
          "label": "Fixed",
          "content": {
            "id": "cfg",
            "type": "object",
            "fields": [
              { "id": "max_attempts", "type": "number", "integer": true, "min": 1, "default": 3 },
              { "id": "delay_ms", "type": "number", "integer": true, "min": 1, "default": 1000 }
            ]
          }
        }
      ]
    },
    {
      "id": "fallback_target",
      "type": "select",
      "source": "dynamic",
      "provider": "workflow.branches"
    }
  ]
}
```

### C) Subflow Selector Form (parameter-level)

```json
{
  "fields": [
    {
      "id": "target_workflow",
      "type": "select",
      "source": "dynamic",
      "provider": "workflow.catalog",
      "searchable": true,
      "required": true
    },
    {
      "id": "args",
      "type": "list",
      "item": {
        "id": "_arg",
        "type": "object",
        "fields": [
          { "id": "name", "type": "text", "required": true },
          { "id": "value", "type": "text", "expression": true, "required": true }
        ]
      }
    }
  ]
}
```

## Parameter Core Gaps Found

1. Missing canonical path validation preset.
   Proposal: standardize a reusable JSONPath-like rule preset with validator + editor hints.

2. Missing canonical dynamic provider contracts.
   Proposal: define payload contracts for:
   - `workflow.branches`
   - `eventbus.channels`
   - `workflow.catalog`

3. Missing list-level uniqueness for object lists.
   Proposal: add `Rule::unique_by(path)` with precise duplicate error paths.

4. Missing formal expression validation contract.
  Proposal: define expected runtime type checks for expression-enabled fields by context.

5. Missing reusable parameter presets.
   Proposal: add `core_fields` helpers for repeated form fragments:
   - `branch_target`
   - `signal_channel`
   - `timeout_ms`
   - `retry_policy`

6. Missing provider contract test matrix.
   Proposal: add schema-level tests for provider value/label/description integrity and stale value handling.

## Out of Scope

- node execution order
- routing semantics (first match vs all matches)
- runtime retry/backoff behavior
- graph topology validation in engine

## Next Steps (Parameter Core Only)

1. Add canonical path-validation preset to v2 schema docs.
2. Add `Rule::unique_by` to validation docs.
3. Add expression validation contract section to v2 docs.
4. Add canonical dynamic provider contract appendix.
5. Add `core_fields` preset API to builder docs.

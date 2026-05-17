# ADR-0063: JSON Schema 2020-12 lossless interop

**Status:** Proposed (2026-05-14)
**Tags:** schema, interop, openapi, jsonschema

## Context

Charter F11: *"Build on JSON Schema 2020-12, do not parallel it.
Lossless export via `x-nebula-*` annotations."*

`nebula-schema` already exports to JSON Schema via `schemars` feature
(see `crates/schema/src/json_schema.rs`). This ADR extends that
direction with:

1. Documenting the **`x-nebula-*` annotation namespace** for
   Nebula-specific extensions.
2. Adding **import direction** (JSON Schema → `ValidSchema`) for
   round-trip use cases.
3. Defining the **separate optional crate** `nebula-schema-jsonschema`
   for the bidirectional bridge.

Henry Andrews (JSON Schema spec author) endorsed this approach at
Day 6 morning conference: "Be a good citizen — extend, don't parallel.
JSON Schema community will appreciate."

## Decision

### `x-nebula-*` annotation namespace

JSON Schema 2020-12 reserves `x-*` for vendor extensions. Nebula uses
`x-nebula-*`:

```jsonc
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "properties": {
    "email": {
      "type": "string",
      "format": "email",                          // standard
      "title": "Email Address",                   // standard
      "description": "Your account email",        // standard
      "x-nebula-widget": "Text",                  // closed Widget enum
      "x-nebula-secret": false,
      "x-nebula-section": "Authentication",
      "x-nebula-when": { "field": "auth", "op": "eq", "value": "password" }
    },
    "api_key": {
      "type": "string",
      "format": "password",
      "x-nebula-widget": "Password",
      "x-nebula-secret": true,
      "x-nebula-when": { "field": "auth", "op": "eq", "value": "api_key" }
    }
  },
  "required": ["email"]
}
```

**Standard JSON Schema fields used wherever possible**, only
`x-nebula-*` for things JSON Schema doesn't express.

### Lossless export

Export pipeline:

```
ValidSchema --[nebula-schema-jsonschema::export]--> serde_json::Value (JSON Schema 2020-12)
```

Round-trip safety: `import(export(s)) == s` (modulo internal repr
choices like ordering).

### Lossless import

```
serde_json::Value --[nebula-schema-jsonschema::import]--> Result<ValidSchema, ImportError>
```

Imports recognize `x-nebula-*` annotations (rebuilds `Widget`
references, conditional fields, etc.). Imports **without**
`x-nebula-*` annotations (third-party JSON Schema files) succeed but
lose Nebula-specific information — fields default to `Field::String` /
`Widget::Text` / no conditional logic.

### Crate split

```
nebula-schema (default-feature: schemars-export gated)
   ↓ optional dependency
nebula-schema-jsonschema (separate crate)
   - bidirectional bridge
   - schemars 1.x integration
   - import error types
   - round-trip property tests
```

`nebula-schema-jsonschema` is **opt-in** — backend-only consumers
don't pay for the bridge crate.

### OpenAPI 3.1 export — automatic

OpenAPI 3.1 is JSON Schema 2020-12 superset for schema definitions.
`nebula-schema-openapi` (separate optional crate) exports
`ValidSchema` → OpenAPI schema component. Reuses
`nebula-schema-jsonschema` internally.

ADR-0047 (`crates/api/` OpenAPI generation) consumes this for action
parameter schemas in API spec.

## Consequences

### Positive

- **First-class JSON Schema citizen.** Other tools (form generators,
  IDE plugins, OpenAPI tooling) that read JSON Schema work with
  Nebula schemas natively.
- Round-trip safety enables external editing (e.g. operator edits
  exported JSON Schema, imports back).
- `x-nebula-*` namespace clean — never collides with future JSON
  Schema spec evolutions.
- Henry Andrews offered to bring `Mode` field design to JSON Schema
  community — potential standardization upstream.

### Negative

- Maintenance: each new `Field` variant or `#[field]` attribute
  requires `x-nebula-*` mapping update. Tracked via property tests
  (round-trip must hold for all variants).
- Import lossy when source JSON Schema lacks `x-nebula-*`
  annotations — author may need to add Nebula-specific metadata
  manually.

### Neutral

- `schemars` dep version coupling — `nebula-schema-jsonschema = "1.x"`
  pins to `schemars = "1.x"`. Schemars 2.0 future requires Nebula
  major bump in bridge crate (not in core).

## Test discipline

Property test in CI: for every `Field` variant + reasonable attribute
combinations, verify `import(export(v)) == v` for randomly generated
fixtures. Catches regressions at PR time.

## References

- Conference Day 6 morning (CONFERENCE-NOTES.md) — Henry Andrews
  endorsement.
- ADR-0047 (OpenAPI 3.1 generator) — downstream consumer.
- JSON Schema 2020-12 spec — https://json-schema.org/draft/2020-12.
- Existing `crates/schema/src/json_schema.rs` (`schemars` feature
  bridge).

## Out of scope

- Other schema languages (Protobuf, Avro, GraphQL). Each
  bidirectional bridge is separate effort with its own ADR.
- JSON Schema older drafts (Draft 7, 2019-09) — only 2020-12
  supported. Older formats would require explicit upgrade tooling.

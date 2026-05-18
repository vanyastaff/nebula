---
id: 0080
title: schema-validation-platform
status: accepted
date: 2026-05-18
supersedes:
  - 0052-schema-validator-condition-seam
  - 0058-schema-field-vocabulary
  - 0059-cross-foundation-dependency-graph
  - 0060-symmetric-foundation-api
  - 0061-nebula-schema-core-ratification
  - 0062-nebula-schema-stdlib-newtype-zoo
  - 0063-json-schema-2020-12-interop
  - 0064-ui-form-composition
superseded_by: []
tags: [schema, validator, expression, ui, json-schema, m11, contract]
related:
  - docs/INTEGRATION_MODEL.md
  - docs/PRODUCT_CANON.md
---

# 0080. Schema & validation platform (contract ADR)

## Context

The M11 schema / validator / dependency-redesign cascade produced eight
accepted or proposed feature ADRs (**0052**, **0058–0064**) that agents had to
open individually. Mechanics (pipelines, ports, crate boundaries) belong in
[`docs/INTEGRATION_MODEL.md`](../INTEGRATION_MODEL.md) and crate READMEs; this
contract ADR records **decisions only** so agents can answer “how does Nebula
model schema, validation, and form UX?” from one file.

## Decision

### Condition evaluation seam (absorbs 0052)

Visibility/required `When(Rule)` evaluation moves into `nebula-validator`
(`policy` module, `PredicateContext`, `resolve_field_policies`). `nebula-schema`
keeps serde-stable `VisibilityMode` / `RequiredMode` and maps at validate time.
`Rule::evaluate` / `RuleContext` are removed (no shim). `Field::Secret` is
excluded from predicate construction at this boundary. `ValidSchema` /
`ValidValues` proof-token custody stays schema-owned; seam tests lock the
invariant.

### Field UI vocabulary (absorbs 0058)

`#[field(...)]` uses a **closed** vocabulary on `nebula-schema` derives so typos
fail at compile/lint time and form renderers share one hint surface (label,
placeholder, secret/multiline/readonly/advanced, etc.).

### Registration-time dependency graph (absorbs 0059)

Foundation registrations (action, resource, credential) form a typed dependency
graph (action → resource/credential, resource → resource/credential, credential →
credential chains). **Cycles are rejected at registration**, not at runtime.

### Symmetric foundation handles (absorbs 0060)

Authors use one pattern — `#[require("key")] field: Handle<T>` — across action,
resource, and credential surfaces instead of divergent guard types and attribute
names. Acquisition/resolution traits align so SDK and engine wiring stay uniform.

### Core schema ratification (absorbs 0061)

`HasSchema`, three-tier proof tokens (`Schema` → `ValidSchema` → `ValidValues` →
`ResolvedValues`), closed `Field` variants, and the schema/validator/expression
sibling split are **ratified as shipped design**, not a greenfield redesign.

### Stdlib newtype zoo (absorbs 0062)

`nebula-schema::stdlib` ships opinionated newtypes (email, url, port, etc.) as the
**default** author path: validation + UI hints without bare `String` fields.

### JSON Schema 2020-12 interop (absorbs 0063)

Export/import round-trip targets JSON Schema 2020-12 with `x-nebula-*` extensions
for Nebula-only semantics; optional `nebula-schema-jsonschema` crate holds the
bidirectional bridge when enabled.

### UI form composition (absorbs 0064)

Workflow editor forms use **two panels**: schema-driven action inputs vs
slot-binding pickers for resources/credentials. Schema is not overloaded to
express slot roles (contrast with single-schema integration tools).

## Consequences

- Agents treating “schema platform” questions should start here, then IM §schema
  / validation for mechanics, then at most one stub if a legacy link targets an
  old number.
- Source ADRs **0052**, **0058–0064** are redirect stubs; do not treat stub bodies
  as normative.
- Breaking API removals (e.g. `Rule::evaluate`) remain governed by the original
  acceptance records in git history pre-stub.

## Supersession

| Source ADR | Role |
|------------|------|
| [0052-schema-validator-condition-seam](./0052-schema-validator-condition-seam.md) | Stub → 0080 |
| [0058-schema-field-vocabulary](./0058-schema-field-vocabulary.md) | Stub → 0080 |
| [0059-cross-foundation-dependency-graph](./0059-cross-foundation-dependency-graph.md) | Stub → 0080 |
| [0060-symmetric-foundation-api](./0060-symmetric-foundation-api.md) | Stub → 0080 |
| [0061-nebula-schema-core-ratification](./0061-nebula-schema-core-ratification.md) | Stub → 0080 |
| [0062-nebula-schema-stdlib-newtype-zoo](./0062-nebula-schema-stdlib-newtype-zoo.md) | Stub → 0080 |
| [0063-json-schema-2020-12-interop](./0063-json-schema-2020-12-interop.md) | Stub → 0080 |
| [0064-ui-form-composition](./0064-ui-form-composition.md) | Stub → 0080 |

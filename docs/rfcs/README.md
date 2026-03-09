# RFC Index And Conventions

This directory mixes standards-track RFCs, informational analysis, and working
papers. This index defines which documents are normative and how to resolve
overlaps between them.

---

## Canonical RFCs

| ID | Document | Type | Status | Notes |
|---|---|---|---|---|
| RFC 0001 | `0001-parameter-schema-v2.md` | Standards Track | Draft | Canonical v2 JSON wire contract and schema shape |
| RFC 0002 | `0002-core-flow-schema-extensions.md` | Standards Track | Draft | Extends RFC 0001 with reusable core-field semantics |
| RFC 0003 | `0003-cross-platform-core-nodes-gap-analysis.md` | Informational RFC | Draft | Research and gap analysis, not a wire contract |
| RFC 0004 | `0004-new-field-types.md` | Standards Track | Draft | Adds `Predicate` and `DynamicRecord`; depends on RFC 0001 and RFC 0002 |

## Supporting Documents

| Document | Type | Status | Notes |
|---|---|---|---|
| `0001-parameter-api-v2.md` | Working Paper | Draft | Internal architecture exploration; non-normative for JSON shape |
| `0001-v2-universality-playground.md` | Exploratory Playground | Draft | Design sandbox; non-normative |
| `examples-core-actions.md` | Informational Examples | Draft | Example action schemas built on RFC 0001 and RFC 0002 |
| `examples-telegram.md` | Informational Examples | Draft | Stress-test examples for resource/operation schemas |
| `param_systems_deep_dive.md` | Informational Research | Draft | Industry survey and design reference |

## Decision Summary

1. RFC 0001 is the canonical source for the v2 wire contract.
2. Internal Rust models may have additional structure, but production JSON
   contracts must follow RFC 0001.
3. Adapters are allowed only for one-time migration/import tooling. No
   long-lived runtime `legacy` API surface is planned in v1. The canonical v2
   model stays adapter-free.
4. UI-only content is represented as dedicated UI data in the canonical schema.
   The exploratory `Schema { nodes: ... }` shape is not the production wire
   contract.
5. RFC 0004 rollout is gated on RFC 0002 defining a versioned dynamic-provider
   response contract.
6. Naming is split intentionally: `parameter` is the domain and crate name;
   `field` is the canonical schema item in the v2 model.

## Metadata Template

Use this header for new RFC documents:

```md
# RFC 000X: Title

**Type:** Standards Track RFC
**Status:** Draft
**Created:** YYYY-MM-DD
**Updated:** YYYY-MM-DD
**Authors:** Name, Name
**Depends on:** RFC 000Y, RFC 000Z
**Supersedes:** None
**Target:** `crate-or-surface` vX
```

Use `**Type:** Informational RFC` for analysis/reference documents and
`**Type:** Working Paper` for non-normative explorations.
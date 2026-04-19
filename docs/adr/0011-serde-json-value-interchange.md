---
id: 0011
title: serde-json-value-interchange
status: accepted
date: 2026-04-19
supersedes: []
superseded_by: []
tags: [data-model, interchange, schema, engine]
related:
  - docs/PRODUCT_CANON.md#125
  - docs/STYLE.md
  - crates/schema/src/lib.rs
  - crates/engine/src/engine.rs
  - docs/adr/0001-schema-consolidation.md
  - docs/adr/0002-proof-token-pipeline.md
linear:
  - NEB-149
---

# 0011. `serde_json::Value` as the workflow data interchange type

## Context

A workflow engine moves two kinds of data between nodes:

1. **Configuration** — parameters, credentials, resource specs. These are
   described by explicit schemas and live behind the proof-token pipeline
   (`ValidSchema` → `ValidValues` → `ResolvedValues`, see
   [ADR-0002 — proof-token pipeline](./0002-proof-token-pipeline.md)).
2. **Runtime payloads** — the outputs of one node that become inputs to the
   next, plus the data coming in from HTTP/webhook triggers and leaving via
   action responses.

Runtime payloads are **heterogeneous and not statically knowable** at engine
compile time. A `Telegram.sendMessage` node produces a shape that a downstream
`HTTP.request` node is free to pick apart, reshape, and forward. This is the
fundamental n8n-style model Nebula inherits.

We need a single in-memory type that:

- Serializes losslessly to/from JSON (the wire format for checkpoints,
  HTTP, persistence);
- Is cheap to `clone` and `serde`-round-trip;
- Is tree-shaped so the expression engine (`nebula-expression`) can walk it
  with JSONPath / template lookups;
- Imposes **zero** static schema on node-to-node data flow.

`serde_json::Value` fits all four. Alternatives considered below.

Product-canon reference: [`§12.5`](../PRODUCT_CANON.md#125) — *"`serde_json::Value`
is allowed where it is the deliberate interchange type; new stringly protocols
(magic field names without schema validation) require explicit review."*

This ADR records the decision already in force and scopes **what it is and
what it is not**.

## Decision

1. **Workflow runtime data is `serde_json::Value`.** Every `ActionResult`
   payload, every edge data envelope, every checkpoint output column carries
   `serde_json::Value` (or a newtype over it when the engine needs to attach
   metadata).
2. **Configuration is not `Value`.** Parameters, credentials, resource specs
   go through `nebula-schema`'s proof-token pipeline and arrive at nodes as
   typed structs. A node that takes a configuration field as raw `Value` is
   an antipattern unless the field is *explicitly* the generic-JSON escape
   hatch (e.g. `http.body` when content type is `application/json`).
3. **Expression context is `Value`.** The expression engine (`{{ $json.x }}`,
   `{{ $node.<id>.output }}`) walks `serde_json::Value` trees directly. No
   intermediate "internal value" type.
4. **Public APIs at layer boundaries** (HTTP, webhook, plugin-sdk protocol):
   accept and emit JSON. On the wire it is UTF-8 JSON; in process it is
   `serde_json::Value`.
5. **New "stringly" contracts require review.** Magic field names like
   `{"$ref": "...", "$exec": "..."}` invented inside a payload without schema
   validation are a product-canon `§12.5` violation. If a feature needs such
   a shape, it must ship a schema — not a convention.

## Consequences

**Positive**

- One interchange type, one mental model. The engine, expression layer,
  storage layer, and API all speak the same thing.
- Lossless JSON round-trip for checkpoint/resume and for HTTP I/O.
- Plugin authors can produce arbitrary JSON from their actions without
  thinking about `dyn Any` or custom trait objects.

**Negative**

- No compile-time type safety for node-to-node data shape. A downstream
  node cannot assume `input["foo"]["bar"]` exists; it must defensively
  check. This is accepted — it is the same trade-off every
  orchestration engine makes.
- `serde_json::Value` is not the most memory-efficient representation
  (string keys, boxed arrays). If a hot path profiles poorly, a newtype
  with `Arc<str>` keys is a local optimization; the public interchange
  stays `Value`.

**Neutral**

- `serde_json::Value` *in library public APIs* needs justification: use it
  only where the value is deliberately open-ended. Typed input is still
  preferred at function boundaries where the shape is known.

## Alternatives considered

- **Custom `NebulaValue` enum.** Reject. Any custom enum we invent would
  either (a) be isomorphic to `serde_json::Value` and add zero value, or
  (b) diverge from JSON and break wire/storage round-trip.
- **`serde_cbor::Value` / MessagePack.** Reject. Wire format must be
  human-inspectable for debugging workflows; CBOR loses that. We can still
  choose CBOR as a *transport* optimization without changing the in-memory
  type.
- **`dyn Any` per-node typed outputs.** Reject. Kills serialization,
  kills the expression engine, kills cross-plugin composition.
- **`jsonb` (Postgres-native).** Out of scope here — this ADR is about the
  in-process interchange type, not the storage encoding. Postgres can store
  `Value` as `jsonb`; SQLite stores it as TEXT; that is orthogonal.

## Scope and non-goals

- This ADR does **not** authorize new "stringly" protocols at config time;
  those go through `nebula-schema` (see ADR-0001, ADR-0002, ADR-0003).
- This ADR does **not** mandate `serde_json::Value` inside a crate's
  internal functions — only at the public interchange layer (node outputs,
  engine/store/api boundaries).

## Follow-ups

- Any new `serde_json::Value` appearing in a public library function
  signature must be justified in code review against this ADR.
- Expression-engine authors: `$json` semantics are documented against
  `serde_json::Value` shape; do not introduce a parallel value model.

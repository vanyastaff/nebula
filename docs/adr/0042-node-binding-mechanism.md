---
id: 0042
title: node-binding-mechanism
status: accepted
date: 2026-04-29
supersedes: []
superseded_by: []
tags: [action, resource, credential, workflow, binding, slot, m6, m11]
related:
  - .ai-factory/plans/m6-resource-finalization-integration-audit.md
  - docs/adr/0043-dependency-declaration-dx.md
  - docs/adr/0044-supersede-0036-resource-credential-singular.md
---

# 0042. Node → ResourceId / CredentialId binding mechanism

## Context

The dependency-redesign cascade (ADR-0043) introduces typed slot fields:

```rust
struct SendTelegram {
    #[resource(key = "bot")]    bot: ResourceGuard<TelegramBot>,
    #[credential(key = "auth")] token: CredentialGuard<TelegramCredential>,
}
```

At runtime, the framework must answer: **which registered `ResourceId` /
`CredentialId` does the slot named `"bot"` / `"auth"` resolve to for *this*
workflow node?** Multiple resources of the same type may be registered (e.g.
two Postgres pools `"main"` and `"analytics"`); the slot key is a *role*, not
an identifier.

The original `crates/resource/plans/06-action-integration.md:68` sketch
referenced `node_config.resource_id_for::<R>()` but the mapping infrastructure
does not exist in code — the only existing resolution path is
`crates/engine/tests/resource_integration.rs` which uses a literal string key
through the dyn `ctx.resources().acquire_any(&key)` API. Without a documented
binding mechanism, Phase 1 (typed `ResourceContextExt::resource::<R>()`) is
unimplementable.

The same problem applies symmetrically to credential slots — multiple
credentials of the same type (`MyOAuth` "primary" + "secondary") need
independent IDs; slot key is the *role label*, not the *credential identity*.

Three design options surfaced during the v4 design dialogue (sessions
2026-04-29):

- **(a) Compile-time defaults only.** `DeclaresDependencies::resources()`
  returns `Vec<(ResourceKey, ResourceId)>`; action encodes default IDs at
  registration time. Simplest. Fixes IDs into the binary — multi-tenant /
  multi-environment deployments cannot override.
- **(b) Workflow-JSON only.** Workflow `NodeDefinition` carries
  `slot_bindings: HashMap<String, BindingTarget>` mapping each slot key to a
  registered ID. Maximally flexible. Every action node must spell out every
  binding even when only one obvious choice exists.
- **(c) Hybrid.** (a) provides defaults, (b) overrides per node.

## Decision

Adopt **option (c) — hybrid binding**:

1. **Default IDs encoded at action level** via the `key` argument to slot
   attributes:
   ```rust
   #[resource(key = "bot")]                  // → default ResourceId == "bot"
   #[credential(key = "primary_oauth")]      // → default CredentialId == "primary_oauth"
   ```
   At registration time, the macro emits `Dependencies` entries pairing each
   slot field with its declared key as the default ID.
2. **Per-node override** via workflow JSON:
   ```json
   {
     "node_id": "send-msg-to-acme",
     "action_key": "telegram.send",
     "slot_bindings": {
       "bot":  { "resource_id":   "acme-tenant-bot" },
       "auth": { "credential_id": "acme-bot-token-v2" }
     },
     "input": { "chat_id": 12345, "text": "Hello" }
   }
   ```
3. **Resolution order at runtime** (inside `FromWorkflowNode::from_workflow_node`,
   ADR-0043):
   ```
   For each slot field on Self:
     resolved_id = node.slot_bindings.get(slot_key)
                       .unwrap_or(slot_key)            // declared default
     instance    = ctx.acquire_<resource|credential>_by_id(resolved_id).await?
   ```

The binding machinery is **symmetric for resources and credentials** — same
JSON shape, same resolution order, same default-from-attribute pattern.

## Consequences

### Positive

- Single-resource and single-credential actions get **zero-config binding** —
  the slot key IS the default ID. 80%+ of plugin authoring needs no per-node
  overrides.
- Multi-environment deployments override per node without modifying the
  action source — `acme-tenant-bot` vs `globex-tenant-bot` for the same
  `TelegramBot` slot.
- Workflow JSON shape is a single new field (`slot_bindings`) on
  `NodeDefinition`; downstream `nebula-workflow` schema impact is bounded.
- Macro-emitted `Dependencies` populated at registration time — engine can
  validate "all required slots resolvable" before workflow starts.

### Negative

- Workflow JSON authors who want fully-explicit bindings still need a JSON
  field — there is no "this node uses defaults" inference at the JSON layer
  (the inference happens in the resolver). Acceptable; default shape is
  empty `slot_bindings: {}`.
- The slot key shadows the registered `ResourceId` / `CredentialId` namespace
  for the no-override case — naming collision risk if two actions both
  declare `key = "db"` and the registry has a registered resource at
  `ResourceId("db")` of a wrong type. Engine emits `TypeMismatch` error at
  acquire time; no silent corruption.

### Follow-up work

- Phase 6 implements `ctx.acquire_resource_by_id::<R>(id)` and
  `ctx.resolve_credential_by_id::<C>(id)` per ADR-0043.
- `nebula-workflow` schema gains `slot_bindings: HashMap<String, BindingTarget>`
  on `NodeDefinition` (Phase 6).
- Validation pass at workflow registration: every action's declared slot has
  a resolvable ID either via override or via default (registered-resource
  lookup). Ship as part of workflow shift-left validation (ROADMAP §M3.6).

## Alternatives considered

### Option (a) — compile-time defaults only

Rejected: cannot override per node. Multi-tenant / multi-environment use
cases (which the M6 shared-resource pattern explicitly supports — one
TelegramBot for 10 workflows) require *different* IDs per workflow.
Compile-time-only freezes IDs at action build time.

### Option (b) — workflow-JSON only

Rejected: every node spells out every binding even when there is only one
sensible choice. Plugin authoring DX regression — the simplest example
"action with a single Postgres slot" forces JSON authors to type
`"slot_bindings": {"db": {"resource_id": "db"}}` redundantly.

## Seam / verification

- **Macro emission point.** `#[derive(Action)]` (ADR-0043 Phase 3.4) emits the
  `Dependencies` builder pairing slot field name → default `ResourceId` /
  `CredentialId` from the `key = "..."` attribute argument.
- **Runtime resolution.** `FromWorkflowNode::from_workflow_node` per slot:
  consult `node.slot_bindings.get(slot_key)`, fall back to declared default.
  Implementation in `nebula-action` (ADR-0043 Phase 3.3).
- **Test gate.** `crates/engine/tests/resource_integration.rs` extends with
  `cross_workflow_resource_sharing` (M6 Phase 8) — one workflow uses defaults,
  another overrides; both resolve to the same backing instance via shared
  `ResourceId`.

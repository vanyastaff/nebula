---
id: 0081
title: m6-resource-credential-integration
status: accepted
date: 2026-05-18
supersedes:
  - 0042-node-binding-mechanism
  - 0043-dependency-declaration-dx
  - 0044-supersede-0036-resource-credential-singular
  - 0045-eventtrigger-scope-deferral
  - 0051-external-provider-redesign
  - 0066-credential-runtime-crate
  - 0067-engine-owned-rotation-fanout-self-refresh-hook
amends:
  - 0036-resource-credential-adoption-auth-retirement
superseded_by:
  # partial — only the credential-runtime-crate (0066) + engine-owned-rotation-fanout (0067) sections;
  # the slot-binding / dependency-DX / external-provider decisions remain in force.
  - docs/adr/0092-credential-subsystem-consolidation.md
tags: [resource, credential, action, engine, m6, m11, contract]
related:
  - docs/INTEGRATION_MODEL.md
  - docs/PRODUCT_CANON.md
  - docs/adr/HISTORICAL.md  # ADR-0030 engine-owned credential orchestration (historical)
---

# 0081. M6 resource & credential integration (contract ADR)

## Context

The M6 resource finalization and M11 credential-runtime cascade produced seven
feature ADRs (**0042–0045**, **0051**, **0066–0067**) covering slot binding,
dependency DX, credential-on-resource adoption, deferred EventTrigger scope,
external secret providers, the management runtime crate, and engine-owned rotation
fan-out. Agents integrating actions, resources, and credentials had to chase
multiple files for one binding story. This contract ADR unifies **integration
binding decisions**; orchestration mechanics stay in
[`docs/INTEGRATION_MODEL.md`](../INTEGRATION_MODEL.md) and ADR-0030.

## Decision

### Node binding (absorbs 0042)

Workflow nodes bind slot **roles** (`#[resource(key)]`, `#[credential(key)]`) to
registered `ResourceId` / `CredentialId` instances via an explicit per-node map —
not implicit type-only resolution when multiple instances share a type.

### Dependency declaration DX (absorbs 0043)

Actions declare infrastructure needs with typed slot fields and
`FromWorkflowNode` (or successor) wiring so compile-time schemas and runtime
resolution share one vocabulary.

### Singular credential on resources (absorbs 0044)

Supersedes ADR-0036’s plural credential bag: each resource exposes typed credential
slot fields; auth retirement paths converge on slot credentials.

### EventTrigger deferral (absorbs 0045)

EventTrigger as a first-class DX wrapper remains **deferred**; scope and trigger
surface are documented here so agents do not assume shipped EventTrigger ergonomics.

### External provider contract (absorbs 0051)

`ExternalProvider` uses native `impl Future + Send` (no `async_trait`), a
resolution envelope aligned with production secret-manager patterns, and an
error-discriminated provider chain for Vault/cloud/keyring backends.

### Credential management runtime (absorbs 0066)

> **Superseded by [ADR-0092](0092-credential-subsystem-consolidation.md) (2026-06-10).**
> The separate `nebula-credential-runtime` (Exec) crate was **deleted**; the
> management bounded context (registry, validate→encrypt→store pipeline, lifecycle
> dispatch, store/external resolution) was **consolidated into `nebula-credential`**
> as its `service/` module. The text below is the historical 0066/0081 decision.

`nebula-credential-runtime` (Exec tier) owns the management bounded context:
registry, validate→encrypt→store pipeline, lifecycle dispatch, and store/external
resolution — without folding management into `nebula-engine`.

### Engine-owned rotation fan-out (absorbs 0067)

> **Superseded by [ADR-0092](0092-credential-subsystem-consolidation.md) (2026-06-10).**
> The resolver/refresh-coordinator/lease/rotation-**state** machinery moved **out of
> `nebula-engine` into `nebula-credential::runtime`**, and the per-slot rotation
> **fan-out** moved into **`nebula-resource`** (co-located with `Manager`). The
> engine retains only the credential/resource accessor bridges. The `&self`
> refresh-hook shape and `SlotCell` substrate are unchanged. The text below is the
> historical 0067/0081 decision ("engine owns").

Engine owns per-slot rotation fan-out, `&self` refresh hooks, `SlotCell`
substrate, and dispatch from credential/lease events; amends 0044’s hook shape
only. Resource finalization design detail lives in IM + crate READMEs, not
execution-plan paths.

## Consequences

- Integration-binding questions → **0081** first, then IM §resource/credential,
  then ADR-0030 for engine mechanism boundaries.
- Stubs **0042–0045**, **0051**, **0066–0067** redirect here; inbound links keep
  working.
- Code behavior unchanged in Wave B (documentation-only merge).

## Supersession

| Source ADR | Role |
|------------|------|
| ADR-0042 — node-binding-mechanism | absorbed (git history) |
| ADR-0043 — dependency-declaration-dx | absorbed (git history) |
| ADR-0044 — supersede-0036-resource-credential-singular | absorbed (git history) |
| ADR-0045 — eventtrigger-scope-deferral | absorbed (git history) |
| ADR-0051 — external-provider-redesign | absorbed (git history) |
| ADR-0066 — credential-runtime-crate | absorbed (git history) |
| ADR-0067 — engine-owned-rotation-fanout-self-refresh-hook | absorbed (git history) |

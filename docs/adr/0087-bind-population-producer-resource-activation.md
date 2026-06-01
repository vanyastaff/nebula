---
id: 0087
title: bind-population-producer-resource-activation
status: proposed
date: 2026-05-31
supersedes: []
amends: []
superseded_by: []
tags: [resource, credential, engine, m6, m12, bind-population, contract-delivery]
related:
  - docs/adr/0081-m6-resource-credential-integration.md
  - docs/adr/0084-pre-expiry-credential-refresh-deferred.md
  - docs/INTEGRATION_MODEL.md
  - docs/PRODUCT_CANON.md
  - docs/COMPETITIVE_ANALYSIS.md
  - docs/ROADMAP.md  # M12.4
---

# 0087. Bind-population producer — resolve & populate credential slots at resource activation

## Context

ADR-0081 (the M6 contract ADR, absorbing 0042–0045 / 0051 / 0066–0067) fixed the
resource↔credential integration **binding contract**: node-level `#[resource(key)]`
/ `#[credential(key)]` slot roles bound to registered ids via an explicit per-node
map; typed credential slot fields; engine-owned rotation fan-out + `&self` refresh
hooks over the `SlotCell` substrate. That contract **assumes a producer** that, at
resource activation, resolves each declared credential slot to a live
`CredentialGuard`, populates the slot **before** `Resource::create`, and records a
reverse-index bind so later rotation/revoke events fan out. No such producer ships
in production — it is the deferred half of ROADMAP §M12.4 and the **sole reason
`nebula-resource` is still `frontier`**.

A file:line codebase gap-analysis (2026-05-31) established that the moat is a
*wiring* job, not a green-field build — the two halves exist and are tested, but
nothing in production joins them:

- **Declaration half (exists):** `#[credential(key)]` → `SlotCell<CredentialGuard<C>>`
  field + `slot_bindings: HashMap<String, CredentialKey>` on `RegisterRequest`
  (`engine/src/resource/registrar.rs`).
- **Resolver half (exists, type-level):** `CredentialService::resolve_for_slot::<C>`
  re-checks the binding's tenant fingerprint and resolves through
  `Audit(Cache(Encryption(raw)))` into a typed `CredentialGuard`
  (`credential-runtime/src/service.rs`); `ValidatedCredentialBinding` has a
  `pub(crate)` constructor minted only by `validate_credential_binding`
  (`binding.rs`) — this is what closed the ADR-0052 `slot_bindings`
  confused-deputy residual.
- **Disconnected (the gap):** `resolve_for_slot` callers are tests only;
  `Manager::register_resolved` validates binding *names* but never resolves nor
  calls `slot.store()`; the resource derive emits a `todo!()` `create` body
  (`resource/macros/src/resource.rs`); `ResourceFanoutDriver::new` has **zero call
  sites**; `register_and_bind` / `EngineConfig::register_resource_and_bind` have
  **zero production callers**.

A produced-but-unconsumed seam (`register_and_bind` with zero callers) is, by
PRODUCT_CANON §12.7, an integrity bug — honest only while an ADR records the
deferral with a re-open trigger. This ADR is that re-open and the delivery
decision.

## Decision

Build the production **bind-population producer** in `nebula-engine` (Exec tier),
keeping `nebula-resource` a pure consumer. At resource activation:

1. Compute `scope: TenantScope` and the per-node `slot_bindings`
   (`slot_name → CredentialId`) from the workflow node.
2. For each binding: `CredentialService::validate_credential_binding(scope, id)`
   → `ValidatedCredentialBinding` (tenant-fingerprint sealed, unforgeable).
3. `CredentialService::resolve_for_slot::<C>(scope, &binding, cancel)` →
   `CredentialGuard<C::Scheme>`. Resolve-with-refresh runs here, so **reactive
   OAuth refresh is obtained on this path for free** (engine resolver, L1+L2
   coalesced per ADR-0041).
4. `slot.store(Arc::new(guard))` **before** `Resource::create(&self, …)` runs
   (canon: injection precedes create). The resource derive emits a real
   constructor that accepts **already-resolved guards** — slot population is the
   engine's job, replacing the `todo!()` create body.
5. `register_and_bind` records the reverse index
   `resolved-credential-identity → (ResourceKey, scope, slot_name, slot_identity)`.
6. Instantiate `ResourceFanoutDriver` in the engine compose path so real
   rotation/revoke events reach bound rows → `Manager::refresh_slot` / `revoke_slot`
   → `SlotCell::store(new guard)` → `on_credential_refresh(&self, slot, runtime)`
   fires (blue-green rebuild of pooled clients).

### Layering (binding)

The producer lives in `nebula-engine` (Exec) calling `nebula-credential-runtime`
(shared infra) and populating `nebula-resource` (Business) slots: Exec
orchestrates, shared-infra resolves, Business reacts. The resolver call **must
not** sit inside `nebula-resource` (it would invert the dependency and re-erode
the crate boundary). The resource constructor accepts resolved guards only.

### Concurrency / quiesce contract (binding)

Per canon §13.2 (no silent strand): an in-flight execution holding valid material
survives a concurrent rotation; the engine must not mid-call swap to a fresher
value the action did not consent to. The `SlotCell` generation stamp +
`credential_slot_epoch()` **order-sensitive fold (not `max`)** + blue-green
RAII-drain satisfy this. The `register_and_bind` **pre-publish window** (between
staging the reverse-index bind and publishing the discoverable `Manager` row) is a
hazard: a rotation in that window can be counted as a miss and not replayed,
leaving a row bound to stale material. The producer **must** honor the quiesce
contract (quiesce fan-out for those credentials across activation) or adopt the
heavier atomic register-then-publish surface.

### Reactive-only boundary

1.0 credential refresh is reactive by accepted decision (ADR-0084 defers proactive
pre-expiry refresh to 1.1). This producer wires the reactive path only; it adds no
background scheduler.

### External source

`StateSource::External` stays fail-closed (`ExternalSourceNotWired`) until the
first-party provider chain (ADR-0081 external-provider contract / ADR-0085
operator-secret lineage) is real. The producer routes `External` only once that
chain is wired — keep the fail-closed guard until then.

## Alternatives considered

- **Resolver call inside `nebula-resource`** — rejected: inverts the dependency
  (Business → shared-infra resolution) and re-introduces boundary erosion.
- **Lazy slot population on first use inside `create`** — rejected: breaks the
  "injection precedes create" canon and complicates the `&self` hook contract.
- **Keep deferred** — rejected: the zero-caller `register_and_bind` is a §12.7
  integrity bug, and `nebula-resource` cannot flip `frontier → stable` without it.

## Scope / non-goals

- **Prerequisite, tracked separately:** concrete credential types
  (`nebula-credential-builtin` OAuth2 + API-key with real `impl Refreshable`,
  ROADMAP M12.3) — without them there is nothing to rotate.
- **Out (ADR-0084):** proactive pre-expiry refresh.
- **Out (ADR-0081 / 0085 lineage):** first-party external Vault provider backend.
- **Adjacent (separate change):** routing `nebula-api` through the typed
  `CredentialService` facade instead of the raw store handle.

## Consequences

- `nebula-resource` flips `frontier → stable` once the producer + fan-out ship;
  ROADMAP §M12.4 bind-population closed.
- The §12.7 orphan (`register_and_bind` zero callers) is resolved; until this ADR
  is accepted and the producer lands, that seam remains a **documented** gap, never
  silent.
- The latent lifecycle (rotation fan-out, `&self` hooks, `ReauthRequired`,
  `ValidatedCredentialBinding`) becomes live end-to-end.
- Delivers a demonstrable, reactive **active credential lifecycle** engine
  primitive — the empty-niche differentiator recorded in
  [`docs/COMPETITIVE_ANALYSIS.md`](../COMPETITIVE_ANALYSIS.md) (no code-first /
  durable-execution incumbent ships one).
- Build order: concrete credential types (M12.3) → producer → fan-out driver →
  API-through-facade.

## References

- ADR-0081 — M6 resource & credential integration (the contract this delivers);
  historical bind-population deferral detail in ADR-0067 lives in git history
  (`git show <rev>:docs/adr/0067-*`).
- ADR-0084 — pre-expiry refresh deferred to 1.1 (the reactive-only boundary).
- ADR-0041 — durable refresh-claim coordinator (the L2 path resolve-with-refresh uses).
- PRODUCT_CANON §12.7 (no orphan modules), §13.2 (no silent strand), §12.5
  (no local persist of externally-resolved secrets).
- ROADMAP §M12.4.

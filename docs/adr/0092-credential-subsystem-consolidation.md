---
# budget-justified: ADR prose document — one contiguous decision record (credential-subsystem consolidation via relocation + port-inversion), not decomposable code
id: 0092
title: credential-subsystem-consolidation
status: accepted
date: 2026-06-10
supersedes: []
amends:
  - docs/adr/0088-credential-subsystem-rewrite.md
  - docs/adr/0066-credential-management-runtime.md
  - docs/adr/0081-credential-lifecycle.md
superseded_by: []
tags: [credential, crate-boundaries, layering, ports, dependency-inversion, crypto, boundary-erosion, breaking]
related:
  - docs/adr/0088-credential-subsystem-rewrite.md  # D4 (generic erasure) finalized; D7 layering superseded
  - docs/adr/0084-reactive-credential-refresh.md    # the reactive-only bet this rests on
  - docs/adr/0052-credential-slot-binding.md         # confused-deputy closure preserved
  - CLAUDE.md  # Layered Dependency Map (credential subsystem collapses)
---

# 0092. Consolidate the credential subsystem into one crate — the engine cycle was an artifact of misplaced machinery

## Status

**Accepted** (2026-06-10, owner-directed). Breaking changes + architectural
correction explicitly authorized.

## Context

ADR-0088 split the credential subsystem across four crates and placed the
lifecycle machinery where it is invoked rather than where it belongs:

- `nebula-credential` (Core) — the contract.
- `nebula-credential-builtin` (Business) — three reference credential types, one
  consumer.
- `nebula-credential-runtime` (Exec) — the `CredentialService` facade, which
  depends **upward** on `nebula-engine` (resolver / refresh-coordinator / lease)
  and `nebula-storage` (encryption / cache / audit decorators).
- `nebula-crypto` (cross-cutting leaf) — AES-256-GCM + Argon2id.

A prior analysis concluded the facade **must** remain a separate Exec-tier crate
because folding it into Core `nebula-credential` forms a hard cargo cycle:
`engine → credential` (engine consumes the contract) and
`credential-runtime → engine` (the facade calls the engine-resident resolver),
so `credential → engine → credential`.

**That cycle is an artifact of where the machinery currently lives, not a
fundamental constraint.** The credential **lifecycle** logic — resolve, refresh,
rotate, lease — is a credential-domain concern. It squats in `nebula-engine`
(14-dep, DAG-execution) and `nebula-storage` (sqlx adapter) for historical
wiring reasons, contradicting both crates' own self-descriptions
("engine = orchestration", "storage = sole adapter") — a boundary erosion.

### Import evidence (the claim that defeats "Exec orchestrates")

The doctrinal rule this ADR overturns is ADR-0088 §D7 (layering table + binding
rules): *"Exec orchestrates, shared-infra resolves"*, which deliberately assigns
`RefreshCoordinator` / lease-scheduling / reclaim / sentinel to `nebula-engine`.
The decisive counter-evidence is a dependency fact, file:line-verified:

- **`crates/engine/src/credential/**` (27 files) references zero Exec-only
  types** — no `WorkflowEngine`, no `nebula_execution` / `nebula_workflow`, no
  `ExecutionStore` / `TransitionBatch`, no `crate::engine` / `crate::runtime`.
  It imports only Core (`nebula_credential`, `nebula_core`), cross-cutting
  (`nebula_eventbus` / `nebula_metrics` / `nebula_resilience`), and Core
  storage-port re-exports. Refresh/lease are credential-**domain**, not
  workflow-**execution**.
- The three storage decorators (`Encryption` / `Cache` / `Audit`) are generic
  `impl<S: CredentialStore>` over the **Core port**; `SqliteCredentialStore`
  appears only under `#[cfg(test)]`. Zero coupling to the sqlx adapter.
- The **moat survives**: the engine invokes resolution through an injected
  type-erased closure `Arc<dyn Fn(&str) -> Pin<Box<dyn Future>>>`
  (`engine.rs:3298`) bridged to the Core `CredentialAccessor` seam; it never
  names `CredentialResolver` as a type in the execution loop. Relocating the
  resolver out of the engine crate leaves refresh-during-execution intact — the
  composition root builds the same closure from the relocated resolver.

The obstacle was never a hard cycle; it was an ADR rule whose premise the import
graph disproves.

## Decision

**Consolidate the credential subsystem into a single crate, `nebula-credential`,
by relocating the misplaced machinery downward and inverting its heavy I/O to
injected ports.** The owner chose one crate over a two-crate (contract /
runtime) compile-firewall split, accepting the LoC growth.

### Relocation

- **Out of `nebula-engine` → `nebula-credential`:** `resolver`, `refresh`
  (coordinator / reclaim / sentinel / L1 / metrics / audit), `lease`,
  `dispatchers`, `executor`, `scoped_accessor`, and the rotation state machines
  (`blue_green` / `grace_period` / `scheduler` / `transaction`, plus the
  token-refresh **state logic**). Engine loses its `reqwest` dependency and keeps
  only the `credential_accessor` / `resource_accessor` bridges, which now consume
  the facade from `nebula-credential`.
- **The `Encryption` / `Cache` / `Audit` decorators + `KeyProvider` / `AuditSink`
  STAY in `nebula-storage`** (amended 2026-06-11). Although they are generic over
  the Core `CredentialStore` port and *could* compile in Core, they are
  **storage-coupled for testing**: a decorator can only be exercised against a
  concrete `CredentialStore`, and the credentials are durable-only — the durable
  stores (`SqliteCredentialStore` / `PgCredentialStore`) deliberately live in
  `nebula-storage` (#789), and the in-memory credential store was deliberately
  **deleted** (#790). Relocating the decorators into Core forced reintroducing an
  in-memory `CredentialStore` double to test them — reversing #790 and barred by
  [the durable-only rule](#). So the decorators stay in storage and are tested
  there over `SqliteCredentialStore`. Only the `Cipher` / `Kdf` generalization
  (below) applies — wiring `EncryptionLayer` onto the `Cipher` port is a storage
  follow-up. The credential subsystem links no in-memory store double.
- **Out of `nebula-engine` → `nebula-resource`:** the resource-fanout pair
  (`fanout_driver` + `resource_fanout`) — the only genuinely cross-Business
  piece. It co-locates with `Manager` (where it already reaches) and stays an
  eventbus subscriber to `CredentialEvent` / `LeaseEvent`.
- **`nebula-credential-runtime` and `nebula-credential-builtin` are deleted**;
  their contents fold into `nebula-credential`.

### Port inversion (the heavy I/O the consolidated crate must not link)

- **`RefreshTransport` (new, in `nebula-credential`)** — inverts the OAuth2 IdP
  token POST so `reqwest` + `rustls` is linked only by the composition root
  (`nebula-api`). **The seam is drawn narrow on purpose:** it carries the bare
  HTTP POST (`url` + form → response bytes). SSRF host/IP validation (SEC-10),
  bounded-response reading (SEC-01), secret-borrow scoping, and `OAuth2State`
  mutation **stay inside `nebula-credential`** (they touch only Core types). A
  wide `&mut OAuth2State` seam would export the SSRF defense to the composition
  root, where a second root (CLI, test harness) could inject a permissive
  transport and bypass it. The concrete `ReqwestRefreshTransport::hardened()`
  lives in `nebula-api`. **Note:** the SSRF validation + reqwest client are
  currently **duplicated** in `api/src/transport/oauth/` and
  `engine/src/credential/rotation/`; the relocation unifies them, and the
  DNS-rebind TOCTOU is closed by enforcing host/IP validation at the transport's
  connect layer (custom resolver) in addition to the pre-call check.
- **`Cipher` / `Kdf` (new, in `nebula-crypto`)** — the credential-crypto
  generalization. `EncryptionLayer` becomes generic over `Arc<dyn Cipher>`; the
  default `AesGcmCipher` / `Argon2Kdf` impls delegate to the existing free
  functions (zero behavior change). The AAD-mandatory invariant (SEC-11) is
  preserved — the trait exposes no no-AAD encrypt method. This adds one vtable
  dispatch on the warm encrypt/decrypt path (owner-accepted) and buys
  algorithm-agility (ChaCha20-Poly1305, HSM-backed) + fake-cipher testability.
- **store port** (`DynCredentialStore` / `RefreshClaimStore`) already lives in
  `nebula-storage-port` (Core); the `nebula-storage` re-export of it is the
  artifact and is dropped — consumers import from the port crate.

### Resulting graph (acyclic, every credential edge points down)

```
api → {engine, storage, credential, tenancy, resource}
engine → {credential, storage, resource, workflow, execution, storage-port}
storage → {credential, storage-port}
credential → {storage-port, crypto, core, schema, metadata, error, eventbus, resilience, metrics}
```

No back-edge from `credential` to any Exec crate. `nebula-credential` links zero
`reqwest`, zero `sqlx`; the ~6 contract-only consumers (action, resource,
plugin, tenancy, storage, sdk) stay free of the heavy deps **without
feature-gates** — heavy impls are dyn-injected at the one composition root, the
codebase's existing isolation pattern.

## Consequences

**Positive:** the cycle dissolves by relocation (not a shim); boundary erosion
is fixed (credential logic leaves engine + storage); the literal one-crate goal
is met; `nebula-crypto` gains the wanted `Cipher` / `Kdf` generalization; heavy
deps are provably isolated from contract-only consumers.

**Negative / accepted:** `nebula-credential` grows to ~15–20k LoC (the largest
non-engine crate) and **loses the crate-level compile firewall** between
contract and runtime that ADR-0088 §D4 cited — a touch to the contract
recompiles the facade + lifecycle. The owner accepts this for the single-crate
authoring surface.

**Invariants preserved** (relocation, not rewrite): ADR-0088 D7 one-write-path;
single `Scope::credential_owner_id` derivation; ADR-0052 confused-deputy closure
(`ValidatedCredentialBinding` sole `pub(crate)` constructor — its real caller is
`validate_credential_binding`, not `resolve_for_slot`; keep ctor + validator +
the cross-tenant test + the `raw_store_without_layers` trybuild E0624 fixture
co-resident); object-safe async seams (`Pin<Box<dyn Future + Send>>`, no
async-trait); reactive-only refresh (ADR-0084); the `CredentialObserver` seam;
one registry / capability-as-sub-trait-membership (ADR-0088 D3).

**Open risk:** if proactive refresh (1.1, ADR-0084) requires the refresh
coordinator to coordinate with the engine scheduler, the "credential-domain, not
orchestration" premise weakens and the machinery could climb back toward Exec.
This ADR bets that reactive refresh (1.0) is free of scheduler coupling.

## Migration (expand-contract, each step whole-workspace-green)

0. **This ADR.** 1. Declare ports (`Cipher`/`Kdf` in crypto; `RefreshTransport`
in credential — additive). 2. `storage-port` `RefreshClaimRepo` pass-through.
3. ~~Move decorators storage→credential~~ **(reverted)** — decorators stay in
`nebula-storage` (tested durably there); wiring `EncryptionLayer` onto the
`Cipher` port is a storage-local follow-up. 4. Move engine lifecycle subtree → credential; route the IdP POST through
`RefreshTransport`; engine loses `reqwest`. 5. Carve the resource-fanout pair →
`nebula-resource`. 6. Fold the facade + builtins into `nebula-credential`.
7. `nebula-api` composition root injects `ReqwestRefreshTransport::hardened()` +
`RustCryptoCipher`. 8. Delete `nebula-credential-runtime` + `-builtin`, drop the
drain re-export shims, update `deny.toml` wrappers. 9. Verify-loop: `cargo tree`
proves no `reqwest`/`sqlx` in action/plugin/sdk and no `credential→engine`/
`credential→storage` edge.

The temporary drain re-export shims (steps 3–4, inside the crates being drained)
are deleted in step 8 — the no-shims rule is honored at completion.

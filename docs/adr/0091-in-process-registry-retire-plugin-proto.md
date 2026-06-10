---
# budget-justified: ADR prose document — one contiguous decision record (direction for the credential/schema/action/resource/plugin authoring axis + Plugin-Proto retirement), not decomposable code
id: 0091
title: in-process-registry-retire-plugin-proto
status: accepted
date: 2026-06-09
supersedes: []
amends:
  - docs/adr/0088-credential-subsystem-rewrite.md
  - docs/adr/0055-nebula-sdk-facade.md
superseded_by: []
tags: [plugin, sandbox, in-process, registry, credential, crate-boundaries, dx, layering, retire, authoring]
related:
  - docs/adr/0006-sandbox-phase1-broker.md  # HISTORICAL — retired by this ADR
  - docs/adr/0025-sandbox-broker-rpc-surface.md  # HISTORICAL — retired by this ADR
  - docs/adr/0054-typed-capability-system.md
  - docs/adr/0088-credential-subsystem-rewrite.md
  - docs/PRODUCT_CANON.md  # §9 north star, §12.6 in-process honesty, WASM non-goal
  - docs/INTEGRATION_MODEL.md  # Plugin = in-process registry
  - CLAUDE.md  # Layered Dependency Map (Plugin-Proto tier removed)
---

# 0091. In-process registry is the plugin north star — retire Plugin-Proto; strengthen the five authoring crates

## Status

**Accepted** (2026-06-09, owner-directed).

The product owner set the direction: **abandon the out-of-process plugin path
(`nebula-sandbox` + `nebula-plugin-sdk` + WASM) for now** and make the
**in-process authoring story strong** across the five crates a human touches to
ship integrations — `nebula-credential`, `nebula-schema`, `nebula-action`,
`nebula-resource`, `nebula-plugin` — with `nebula-plugin` as the **in-process
registry** where actions / credentials / resources / nodes are registered in
Rust code.

This ADR is the **direction record** for that axis. It amends ADR-0088 (the
credential subsystem rewrite continues, minus the object-safety pressure that
out-of-process authoring would have imposed) and ADR-0055 (the SDK facade's
`sandbox-process` feature is dropped). It retires the Plugin-Proto tier
introduced by the historical ADR-0006 / ADR-0025.

## Context

Recon (2026-06-09, file:line-verified) established four facts that make this
direction low-risk and canon-aligned, not a strategic reversal.

### F1 — Out-of-process is NOT a stated moat; it contradicts the north star less than it serves it

`PRODUCT_CANON` §12.6 names **WASM/WASI an explicit non-goal** for plugin
isolation and states the in-process sandbox is "**capability/correctness, not
attacker-grade isolation**." §9's north star is "a Rust developer ships a
working, tested node for a new service in a focused day" — an **in-process Rust
crate-linking** model. `INTEGRATION_MODEL` §"Plugin is the registry" makes the
in-process Cargo-dependency closure primary. The out-of-process path is
positioned in `ROADMAP` as decomposed-for-honesty **unfinished scaffolding**
(capability-discovery enforcement still open, M4), gated behind the
`out-of-process-plugins` feature (**OFF by default**). Removing it contradicts
no binding commitment.

### F2 — `nebula-plugin` is ALREADY the in-process registry

`Plugin` trait → `ResolvedPlugin::from()` (eager-resolves + namespace-validates
+ dedups) → `PluginRegistry::register()` (keyed `HashMap`, O(1) lookup). It
returns trait objects (`ActionFactory` / `AnyCredential` / `AnyResource`) indexed
by key. The out-of-process surface (`discovery` / `sandbox_bridge` /
`remote_action` / `handler` / `discovered_plugin`) is an **orthogonal
feature-gated layer on top**; deleting it leaves the in-process registry
contract intact.

### F3 — Engine coupling is surgical

Action execution branches on `IsolationLevel`: `None` → direct in-process
handler call; `CapabilityGated`/`Isolated` → `SandboxRunner`. Credentials and
resources already resolve fully in-process (`EngineCredentialAccessor` /
`EngineResourceAccessor`, deny-by-default allowlist). Dropping out-of-process =
delete `out_of_process.rs` + `plugin_supervisor.rs` (feature-gated) + the dead
`plugin_pool.rs` + the `ProcessSandbox` `SandboxRunner` impl, and collapse the
`IsolationLevel` match to `None`. `ActionRuntime` dispatch logic is unchanged.

### F4 — The five-crate authoring story is coherent but fragmented; the credential subsystem carries structural debt

- **Crate sprawl + incoherence:** first-party credential types are **split** —
  `api_key`/`basic_auth`/`oauth2` in `nebula-credential/src/credentials/`,
  `bearer`/`shared`/`signing` in `nebula-credential-builtin`. `nebula-credential`
  is a **14,850-LOC / 64-module God-crate** (contract + schemes + secrets +
  provider chain + lifecycle + store trait + concrete types).
- **InMemoryStore is the only production backend** (one wiring site in
  `credential_service_factory.rs`); the durable seam is clean (`impl
  CredentialStore` 5 methods + `DynPendingStateStore`; Encryption/Cache/Audit
  decorators are generic). Dead `CredentialRow`/`CredentialRepo` + migrations
  0008/0017 still present with zero impls (ADR-0088 D7d).
- **DX fragmentation:** associated-type names differ for the same role
  (Action `Input/Output`, Resource `Config/Runtime/Lease`, Credential
  `Properties`); field-slot attribute grammar differs (Action `#[credential]`
  on `CredentialGuard<C>` eager vs Resource on `SlotCell<CredentialGuard<C>>`
  rotated); a legacy `#[derive(Credential)]` still ships beside the canonical
  `#[credential]` attribute macro (ADR-0088 D1). Schema-derive is already
  uniform.

## Decision

Four work-streams, executed in order. Each is whole-workspace-green per commit
(expand-contract). Each ships as its own PR.

### D1 — Retire Plugin-Proto (FIRST; this branch)

Delete `nebula-sandbox` and `nebula-plugin-sdk` (~4,636 LOC) and the
out-of-process scaffolding they feed:

- **engine:** delete `runtime/out_of_process.rs`, `runtime/plugin_supervisor.rs`,
  `runtime/plugin_pool.rs`; remove the `ProcessSandbox` `SandboxRunner` impl;
  collapse the `IsolationLevel::{CapabilityGated,Isolated}` dispatch arms to the
  in-process path. `SandboxRunner` collapses to its single in-process executor
  (inline if only one impl remains). Drop the `out-of-process-plugins` feature.
- **plugin:** delete `discovery.rs`, `discovered_plugin.rs`, `remote_action.rs`,
  `handler.rs`, `sandbox_bridge.rs`; keep `Plugin` / `ResolvedPlugin` /
  `PluginRegistry` (the in-process registry) untouched.
- **cli / examples / api(dev):** drop the sandbox/discovery call sites and the
  out-of-process integration tests.
- **deny.toml:** remove the `nebula-sandbox` / `nebula-plugin-sdk` wrapper
  entries; remove both crates from `[workspace.members]`.
- **docs:** delete the **Plugin-Proto** tier from the root `CLAUDE.md` Layered
  Dependency Map and `README.md`; mark ADR-0006 / ADR-0025 **superseded** in
  `docs/adr/HISTORICAL.md`; note ADR-0055's `sandbox-process` feature removed.
- **canon:** `PRODUCT_CANON` §12.6's "ProcessSandbox (already shipping) → …"
  isolation roadmap is struck — the honest position becomes "in-process only;
  isolation is correctness, not a security boundary; third-party untrusted code
  is out of scope until a future revisit."

`IsolationLevel` as a per-action concept is retained as data (an action may
*declare* a desired isolation) but has exactly one execution meaning today
(in-process); the enum is **not** deleted, so re-introducing isolation later is
additive, not breaking.

### D2 — Credential consolidation (split the God-crate) — OPEN DECISION POINT

`nebula-credential` (Core) keeps **only the contract**: `Credential` trait +
capability sub-traits + `AuthScheme` + schemes + `CredentialState`/`PendingState`
+ secrets + registry + lifecycle policy + provider-chain trait + store trait. All
**concrete first-party types move to one home**.

> **Owner leaning:** put all first-party types **in `nebula-credential`**.
> **Recon-grounded recommendation:** put all first-party types in
> **`nebula-credential-builtin`** (Business), leaving Core a thin contract. The
> owner's own goal — a contract plugin authors adopt — is better served by a
> Core crate a newcomer can read as *just the contract*, with `-builtin` as the
> worked reference set. Keeping concrete types in a 64-module Core crate is the
> God-crate smell the owner flagged. **This conflict is resolved before D2
> starts; D1 does not depend on it.**

Either way the split deletes the `credentials/` ↔ `-builtin` incoherence: one
home for first-party types, contract-only Core.

### D3 — Durable credential backend (remove InMemoryStore-only)

Implement `CredentialStore` + `DynPendingStateStore` durable adapters
(**SQLite + Postgres**, owner-chosen "both at once") in `nebula-storage` against
the live `StoredCredential` + `EncryptionLayer` path; wire as the production
backend behind the existing decorator stack (unchanged). Then **delete the dead
`CredentialRow` / `CredentialRepo` / migrations 0008+0017** (ADR-0088 D7d). The
in-memory store survives as a test double only.

### D4 — Authoring-DX unification across the five crates

- One associated-type vocabulary for the "schema-bearing input": align on a
  single name (`Input` is the most-used; Credential `Properties` and Resource
  `Config` are aliased or renamed — decided at D4 start).
- One field-slot attribute grammar: `#[credential(key = …)]` / `#[resource(key =
  …)]` read identically on Action and Resource; the eager (`Guard`) vs rotated
  (`SlotCell<Guard>`) distinction is the *field type*, not a different attribute.
- Delete the legacy `#[derive(Credential)]` path; `#[credential]` attribute macro
  (ADR-0088 D1) is the only credential authoring surface.
- Keep schema-derive as-is (already uniform).

## Sequence

1. **D1 retire Plugin-Proto** — independent, owner-greenlit, surgical. (this branch)
2. **D2 credential consolidation** — after the D2 home decision.
3. **D3 durable backend** — needs the consolidated contract crate; seam already clean.
4. **D4 DX unification** — last; cosmetic contract alignment after structure settles.

## Consequences

- The workspace loses two crates (`nebula-sandbox`, `nebula-plugin-sdk`) and ~4.6k
  LOC; the **Plugin-Proto tier disappears** from the layer map. Linux
  Landlock/rlimit hardening (sandbox-only) is dropped — acceptable, it only
  guarded out-of-process children that no longer exist.
- The credential subsystem moves from 6 crates with split first-party types to a
  thin contract Core + one first-party home + Exec facade + optional Vault
  backend + crypto + testutil; the God-crate shrinks.
- Credentials gain durable persistence; InMemoryStore is demoted to a test double.
- A plugin author sees one coherent authoring grammar across action / resource /
  credential / schema, registered through one in-process `Plugin` impl.
- This is a **hard breaking change** across plugin / engine / sandbox / sdk / cli
  / api / credential / storage and the derive macros. It is canon-aligned
  (F1–F4), spec-correct, and expand-contract-migratable.

## Alternatives considered

- **Keep out-of-process behind the feature flag, untouched.** Rejected by the
  owner: it carries 4.6k LOC of unfinished scaffolding, a Plugin-Proto tier, and
  a "planned isolation" honesty debt (M4) with no near-term consumer. Dead weight
  on the in-process north star.
- **WASM/WASI sandbox instead of process isolation.** Rejected by canon §12.6
  (explicit non-goal; false-capability + DX-regression).
- **D2: all first-party types stay in `nebula-credential`.** The owner's initial
  leaning; flagged against the adoption goal (God-crate). Carried as an open
  decision point, not silently overridden.

## References

- ADR-0088 — credential subsystem rewrite (amended: object-safety pressure
  removed; D1–D4 of that ADR remain landed).
- ADR-0055 — nebula-sdk facade (amended: `sandbox-process` feature dropped).
- ADR-0006 / ADR-0025 — Plugin-Proto broker (HISTORICAL; retired here).
- PRODUCT_CANON §9 (north star), §12.6 (in-process honesty, WASM non-goal).
- INTEGRATION_MODEL — Plugin as in-process registry.
- Recon digests: memory `project_inprocess_registry_pivot`,
  `project_credential_api_wiring`, `project_credential_rewrite_plan`.

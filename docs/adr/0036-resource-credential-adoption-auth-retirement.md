---
id: 0036
title: resource-credential-adoption-auth-retirement
status: proposed
date: 2026-04-24
supersedes: []
superseded_by: []
tags: [resource, credential, trait-shape, rotation, revocation, breaking-change, canon-3.5, canon-4.5, canon-12.5]
related:
  - docs/superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md
  - docs/superpowers/specs/2026-04-24-credential-tech-spec.md
  - docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md
  - docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md
  - docs/PRODUCT_CANON.md#35-architectural-vocabulary
  - docs/PRODUCT_CANON.md#45-public-surface-discipline
  - docs/PRODUCT_CANON.md#125-secrets-and-auth
  - crates/resource/src/resource.rs
  - crates/resource/src/manager.rs
  - docs/adr/0028-cross-crate-credential-invariants.md
  - docs/adr/0030-engine-owns-credential-orchestration.md
  - docs/adr/0033-integration-credentials-plane-b.md
  - docs/adr/0037-daemon-eventsource-engine-fold.md
linear: []
---

# 0036. `Resource::Credential` adoption, `Resource::Auth` retirement

## Status

**Proposed** at Phase 5 of the nebula-resource redesign cascade. Acceptance gates on Tech Spec CP1 ratification (Phase 6).

Records the primary architectural decision from the [nebula-resource redesign Strategy](../superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md) §4.1 + §4.2 + §4.3 (frozen 2026-04-24). Cross-references the [credential Tech Spec](../superpowers/specs/2026-04-24-credential-tech-spec.md) §3.6 (lines 928-996) as the ratified downstream contract being adopted verbatim.

**Cross-cascade coordination:** none. The resource-side hooks consume credential primitives that already exist in the credential Tech Spec ([§Credential::revoke line 228](../superpowers/specs/2026-04-24-credential-tech-spec.md), [§4.3 lines 1062-1068](../superpowers/specs/2026-04-24-credential-tech-spec.md)). No credential-side spec extension required.

## Context

### What was wrong

**`Resource::Auth: AuthScheme` is a false capability** ([Strategy §1.1](../superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md), [pain-enumeration §1.1 + 🔴-5](../superpowers/drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md)).

- Trait declaration at [`crates/resource/src/resource.rs:233`](../../crates/resource/src/resource.rs): `type Auth: AuthScheme;`
- Workspace usage: 100% of `type Auth = …;` sites bind `()`. Verified across 9 in-tree test resources (`tests/basic_integration.rs:109,183,619,1536,1620,1698,2149,2506,2581,2966`) and 5 production runtime resources (`runtime/{daemon,pool,resident,service}.rs`). No non-`()` consumer in trunk.
- Result per [`PRODUCT_CANON.md §4.5`](../PRODUCT_CANON.md): "public surface exists iff the engine honors it end-to-end." `Resource::Auth` advertises capability the workspace never exercises — false-capability-rule violation.

**Credential Tech Spec §3.6 prescribes a structurally different shape** ([credential Tech Spec lines 928-996](../superpowers/specs/2026-04-24-credential-tech-spec.md)).

The downstream-ratified contract designs rotation as a per-resource `on_credential_refresh` hook on the `Resource` trait directly with a `type Credential: Credential` associated type. The current `type Auth: AuthScheme` shape cannot host the §3.6 hook without semantic distortion (`AuthScheme` is the runtime material, not the credential identity).

**Rotation dispatchers are `todo!()` panics over an empty reverse-index** ([Strategy §1.1](../superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md), [security-lead Phase 1 finding 🔴-1](../superpowers/drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md)).

- `Manager::on_credential_refreshed` at [`crates/resource/src/manager.rs:1360`](../../crates/resource/src/manager.rs) and `Manager::on_credential_revoked` at [`:1386`](../../crates/resource/src/manager.rs) both terminate in `todo!("Implementation deferred to runtime integration")` ([`:1378` and `:1400`](../../crates/resource/src/manager.rs)).
- Backing `credential_resources: DashMap<CredentialId, Vec<ResourceKey>>` field declared at [`:262`](../../crates/resource/src/manager.rs) and constructed at [`:293`](../../crates/resource/src/manager.rs) — never written. No code path inserts entries.
- Operational consequence: credential refresh/revocation events from the credential plane are silently no-op'd today (the dispatchers find an empty index, return `Ok(())` before reaching the `todo!()`). The first reverse-index write would unmask the panic.

**Earlier rename moved trait in the wrong direction.** Commit `f37cf609 feat(resource)!: rename Resource::Credential to Resource::Auth` renamed the associated type to `Auth`, but the credential Tech Spec §3.6 (ratified downstream) is structured around `Credential`. The rename optimised for surface-level naming consistency with `AuthScheme` and missed the structural mismatch with the Tech Spec.

### What forced the decision

- **Atomic landing required.** Security-lead BLOCKed Option A (deferral) at Phase 2 ([phase-2-security-lead-review.md:25-46](../superpowers/drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md) cited via [scope-decision §1](../superpowers/drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md)). Reverse-index write + dispatcher logic + per-resource hook must land in one PR — the panicking placeholder cannot ship as a partial fix.
- **Spec alignment.** Credential Tech Spec §3.6 is the ratified downstream contract; resource-side trait must reconcile or the credential rotation cascade has no per-resource integration point.
- **Migration cheap.** `nebula-resource` MATURITY = `frontier` per [`docs/MATURITY.md:36`](../MATURITY.md); workspace has 5 in-tree consumers (`nebula-action`, `nebula-sdk`, `nebula-engine`, `nebula-plugin`, `nebula-sandbox`) and zero external adopters. Bundled breaking-change PR is viable.

## Decision

Adopt the credential Tech Spec §3.6 shape verbatim on the `Resource` trait. Conceptual signature ([credential Tech Spec lines 935-955](../superpowers/specs/2026-04-24-credential-tech-spec.md) — full Rust signatures land in Phase 6 Tech Spec §3, not here):

```rust
pub trait Resource {
    type Credential: Credential;          // was: type Auth: AuthScheme
    type Error: Classify + Send + Sync + 'static;

    async fn create(
        ctx: &ResourceContext<'_>,
        scheme: &<Self::Credential as Credential>::Scheme,
    ) -> Result<Self, Self::Error>
    where Self: Sized;

    /// Default no-op. Connection-bound resources override with blue-green
    /// pool swap pattern (credential Tech Spec §3.6 lines 961-993).
    async fn on_credential_refresh(
        &self,
        new_scheme: &<Self::Credential as Credential>::Scheme,
    ) -> Result<(), Self::Error> { Ok(()) }

    /// Default no-op. Override invariant: post-invocation, the resource
    /// emits no further authenticated traffic on the revoked credential.
    async fn on_credential_revoke(
        &self,
        credential_id: &CredentialId,
    ) -> Result<(), Self::Error> { Ok(()) }

    // …other lifecycle methods (key, destroy, metadata) per Phase 6 Tech Spec §3.
}
```

Specifics:

- **`type Credential = NoCredential;` is the idiomatic opt-out** for resources without an authenticated binding ([Strategy §4.1](../superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md), [scope-decision §4.1](../superpowers/drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md)). Replaces the current `type Auth = ();` pattern.
- **NO `AuthenticatedResource: Resource` sub-trait.** Rejected at Phase 2 tech-lead amendment 1 and Phase 3 CP1 E1 ([scope-decision §4.1](../superpowers/drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md)). Spike exit criteria do NOT include sub-trait fallback per [Strategy §2.4](../superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md).
- **Blue-green pool swap is internalised by the resource impl.** `Manager` does not orchestrate the swap; per-resource `on_credential_refresh` owns its `Arc<RwLock<Pool>>` write-lock window ([credential Tech Spec lines 961-993](../superpowers/specs/2026-04-24-credential-tech-spec.md)).
- **Rotation dispatch: parallel `join_all` with per-resource timeout isolation** ([Strategy §4.3](../superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md)). NOT a single global dispatch timeout — a global timeout defeats the isolation invariant (one slow resource would cascade-fail siblings). Each per-resource future has its own error path; one resource's failure does NOT block sibling dispatches.
- **Revocation default-hook invariant** ([Strategy §4.2](../superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md)): post-invocation, the resource emits no further authenticated traffic on the revoked credential. The mechanism (destroy pool / mark-tainted / wait-for-drain / reject-new-acquires) is a Phase 6 Tech Spec §5 decision; this ADR commits to the *invariant*, not the *implementation*.
- **Reverse-index write path lands atomically with the dispatcher.** `register_*` paths populate `credential_resources` when `type Credential != NoCredential`; `Manager::on_credential_refreshed` / `on_credential_revoked` consume the populated index. No `todo!()` survives the PR wave.
- **`warmup_pool` must NOT call `Scheme::default()`** under the new shape ([Strategy §2.4 / §4.9](../superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md), security-lead amendment B-3). Phase 6 Tech Spec §5 specifies the credential-bearing warmup signature.
- **No `clone()` on secret schemes in the dispatcher hot path** ([Strategy §2.4](../superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md), security-lead constraint 7). Dispatcher passes `&Scheme`, not owned `Scheme`. Each clone would be another zeroize obligation per [`PRODUCT_CANON.md §12.5`](../PRODUCT_CANON.md).

## Consequences

### Positive

- **Cross-crate boundary is spec-aligned.** Credential Tech Spec §3.6 and `Resource` trait now share one rotation hook shape across the workspace; no parallel-shape dual surface.
- **Silent revocation drop eliminated.** Reverse-index write path and rotation dispatcher land atomically in the same PR wave; the panicking `todo!()` is replaced by working dispatch in one breaking change, not two.
- **Per-resource blue-green swap is materially safer than manager-orchestrated recreation.** The §3.6 pattern keeps the new scheme inside the resource's `await` window; `Manager` never holds a `&Scheme` across an external boundary, narrowing the attack surface for credential exposure.
- **Dead `Auth` associated type removed.** API surface smaller, more intention-revealing — `type Credential` reads as identity-and-capability, not just material-of-the-moment.
- **`NoCredential` opt-out is idiomatic Rust.** `type Credential = NoCredential;` matches the current `type Auth = ();` ergonomically; no footgun at call site, no `Option`-wrapping required.
- **Rotation observability is DoD, not follow-up** ([Strategy §4.9](../superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md)). Trace span + counter + `ResourceEvent::CredentialRefreshed` / `CredentialRevoked` ship in the same PR as the dispatcher; operators can distinguish silent-success from silent-drop on first deploy.

### Negative

- **Breaking change to public `Resource` trait.** Every in-tree `impl Resource` must be rewritten — 9 test resources + 5 production runtime resources surveyed at [pain-enumeration §1.1](../superpowers/drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md) need `type Auth = ();` → `type Credential = NoCredential;` migration (or a real credential bound where applicable).
  - Mitigation: 5 in-tree consumer crates + MATURITY = `frontier` (no external adopters); per Nebula stability policy ([`docs/MATURITY.md:36`](../MATURITY.md)) breaking changes are expected at this maturity level.
  - No shims, no deprecation window, no parallel-shape compat layer per `feedback_no_shims.md` + `feedback_hard_breaking_changes.md`.
- **Phase 4 spike exit criteria require §3.6 shape to pass.** No sub-trait fallback if ergonomics or perf fail. Spike failure escalates to Phase 2 round 2 per [scope-decision §4.1 / §5](../superpowers/drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md), not a mid-flight shape change. Encoded explicitly in [Strategy §2.4](../superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md) spike-exit-criteria constraint.
- **Adapter documentation surface must be rewritten.** Current `crates/resource/docs/{adapters.md,api-reference.md,README.md}` 3-way contradict on `Credential` vs `Auth` naming ([pain-enumeration §1.1 + 🟠-15](../superpowers/drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md)). Phase 6 Tech Spec §13 lands the rewrite atomically with the trait reshape per [Strategy §4.7](../superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md).

### Neutral

- **Strategy-level decision, not Phase 6 implementation.** This ADR records the contract; Phase 6 Tech Spec encodes the full Rust trait signature, the reverse-index write paths, the dispatcher concurrency primitives, the per-resource timeout configuration surface, and the consumer migration deltas.
- **Cross-crate spec dependency closed.** Credential Tech Spec already provides `Credential::revoke` ([line 228](../superpowers/specs/2026-04-24-credential-tech-spec.md): `async fn revoke(ctx, state) -> Result<(), RevokeError>`) and revocation lifecycle modes ([§4.3 lines 1062-1068](../superpowers/specs/2026-04-24-credential-tech-spec.md): soft/hard/cascade revocation, `state_kind = 'revoked'` semantics). Resource-side `on_credential_revoke` hook is a *consumer* of those existing primitives, not an extension of credential §3.6. No cross-cascade coordination round required.
- **Future trigger for sub-trait revisit.** If a non-trivial fraction of `Resource` impls outside the workspace ever adopt `type Credential = NoCredential;` (i.e., the unauthenticated path becomes statistically dominant), the trait split (`Resource` + `AuthenticatedResource`) may merit reconsideration. Not in scope here; tracked as a future-cascade trigger via [Strategy §5](../superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md).

## Alternatives considered

### Alternative 1 — Keep `Resource::Auth: AuthScheme`; fix rotation at `Manager` only

Manager orchestrates `destroy → recreate` of pool instances when a credential refreshes. Per-resource hook is not introduced. `Resource::Auth` retains current shape (mostly `()` everywhere).

**Rejected** because:

- Leaves the structural mismatch with credential Tech Spec §3.6 unresolved — every future credential-side change re-opens the same misalignment.
- Manager-orchestrated `destroy → recreate` has an inherent consistency window: in-flight `acquire` callers either see the old pool (until destroyed) or block awaiting the new pool. The §3.6 blue-green swap is atomic at the `Arc<RwLock<Pool>>` write — readers never observe a destroyed pool because the swap publishes the new value before the old is dropped.
- Does not remove the dead `type Auth` associated type; canon §4.5 false-capability violation persists.

### Alternative 2 — `AuthenticatedResource: Resource` sub-trait + retain `Resource::Auth = ()` shim

`Resource` keeps `type Auth = ()` as a vestigial shim; `AuthenticatedResource: Resource` adds `type Credential: Credential` for credential-bearing impls.

**Rejected** at Phase 2 tech-lead amendment 1 and Phase 3 CP1 E1 ([scope-decision §4.1](../superpowers/drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md)):

> "§3.6 is the ratified downstream contract, zero in-tree production `impl Resource` sites make migration cheap, and a sub-trait doubles the API learning surface for no benefit."

The sub-trait would force every `register_*` / `acquire_*` convenience method to take a `where R: AuthenticatedResource` branch in parallel with a `where R: Resource` branch — exactly the combinatorial-bound friction surfaced in [pain-enumeration §2.3 (rust-senior 🟡)](../superpowers/drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md). With 5 in-tree consumers and `frontier` maturity, the cost of the breaking change is lower than the cost of permanent dual-shape API.

### Alternative 3 — Defer trait reshape entirely (Option A from Phase 2)

Doc rewrite + drain-abort fix only. No trait change, no rotation hook, no reverse-index write. Credential×resource seam fix deferred to a "follow-up project."

**Rejected** at Phase 2 by security-lead BLOCK ([scope-decision §1, 🔴-1 row](../superpowers/drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md)):

> "🔴-1 silent revocation drop + latent `todo!()` panic on reverse-index write cannot be resolved by deferral."

Tech-lead independently rejected on `feedback_incomplete_work.md` grounds — writing the doc rewrite against an `Auth`-shaped trait known to be superseded by §3.6 is "don't write the docs twice" at the architecture level ([scope-decision §1](../superpowers/drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md)).

### Alternative 4 — Comprehensive redesign also collapsing `Runtime`/`Lease` and wiring `AcquireOptions::intent/.tags` (Option C from Phase 2)

Everything in this ADR plus: collapse `Runtime`/`Lease` distinction (9/9 test resources set `Lease = Runtime` per [pain-enumeration §2.3 🟡](../superpowers/drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md)) via associated-type default; wire or remove `AcquireOptions::intent/.tags` ([crates/resource/src/options.rs:17-64](../../crates/resource/src/options.rs)).

**Rejected** at Phase 2 ([Strategy §3.3](../superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md)):

- `Runtime`/`Lease` collapse — friction is real but orthogonal to the credential driver. Standalone ADR + future cascade.
- `AcquireOptions::intent/.tags` — engine integration ticket #391 drives the shape; resolving here would be guessing without engine-side design.
- Surface added not addressed by 🔴 / 🟠 primary drivers; schedule risk; both deferred items are explicit Strategy §5 open items recorded as future-cascade triggers.

## References

- [nebula-resource redesign Strategy](../superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md) — §1.1 (problem), §2.3 (cross-crate constraint), §2.4 (toolchain + spike-exit), §3.2 (Option B), §4.1 (trait reshape), §4.2 (revocation extension), §4.3 (rotation dispatch mechanics), §4.9 (observability DoD), §5 (open items), §6 (post-validation roadmap).
- [nebula-credential Tech Spec](../superpowers/specs/2026-04-24-credential-tech-spec.md) — §3.6 (lines 928-996, on_credential_refresh + blue-green pattern), §Credential::revoke (line 228), §4.3 (lines 1062-1068, revocation lifecycle modes).
- [Phase 1 pain enumeration](../superpowers/drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md) — §1.1 (convergent finding 🔴-1), §2.2 (security-lead unique), §2.3 (rust-senior unique).
- [Phase 2 scope decision](../superpowers/drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md) — §1 (in-scope), §4.1 (sub-trait rejection rationale), §4.7 (no-shim migration policy), §5 (spike scope + exit criteria), §6 (cascade artefact plan).
- [`PRODUCT_CANON.md §3.5`](../PRODUCT_CANON.md) (resource = pool/SDK client; engine owns lifecycle).
- [`PRODUCT_CANON.md §4.5`](../PRODUCT_CANON.md) (false capability rule — removes unused `Auth` associated type).
- [`PRODUCT_CANON.md §12.5`](../PRODUCT_CANON.md) (secrets invariants on rotation path).
- [ADR-0028 — cross-crate credential invariants](./0028-cross-crate-credential-invariants.md).
- [ADR-0030 — engine owns credential orchestration](./0030-engine-owns-credential-orchestration.md).
- [ADR-0033 — integration credentials Plane B](./0033-integration-credentials-plane-b.md).
- Trait declaration at [`crates/resource/src/resource.rs:233`](../../crates/resource/src/resource.rs); rotation dispatchers at [`crates/resource/src/manager.rs:1360-1401`](../../crates/resource/src/manager.rs); reverse-index field at [`crates/resource/src/manager.rs:262`](../../crates/resource/src/manager.rs).
- Commit `f37cf609 feat(resource)!: rename Resource::Credential to Resource::Auth` — earlier rename now reversed by this ADR.

## Review

Ratified through the Phase 2 + Phase 3 co-decision protocol of the redesign cascade:

- **architect** — drafts this ADR (Phase 5).
- **tech-lead** — Phase 2 priority-call selecting Option B with 2 amendments ([Strategy §3.2](../superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md)); Phase 3 CP1 + CP2 ratification ([Strategy §4.1 / §4.2](../superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md)).
- **security-lead** — Phase 2 ENDORSE Option B with 3 amendments (isolation invariant on parallel dispatch; revocation extension of §3.6; warmup `Scheme::default()` removal); §3.6 blue-green pattern endorsed as security-superior to manager-orchestrated recreation.

Acceptance gate: this ADR moves to `accepted` when Phase 6 Tech Spec CP1 ratifies the full Rust trait signature against the conceptual shape recorded above.

### Amended in place on

(empty on first draft; future amendments listed here per the ADR-0035 amended-in-place pattern.)

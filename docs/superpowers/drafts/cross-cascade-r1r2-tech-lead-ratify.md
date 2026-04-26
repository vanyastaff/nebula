---
name: Cross-cascade R1+R2 + I1-I4 routing — tech-lead ratification
status: ratified
date: 2026-04-26
authors: [tech-lead]
scope: Mechanical ratification of architect-enacted R1 (action §2.2.4 stub Resource trait removal) + R2 (resource §2.1 + ADR-0036 §Decision on_credential_refresh re-pin) + routing call for 4 INCOMPLETE gaps (I1-I4) from cross-cascade consolidated review §7.2
inputs:
  - docs/superpowers/drafts/2026-04-24-cross-cascade-consolidated-review.md (§7.1 R1+R2 routing; §7.2 I1-I4 routing)
  - docs/superpowers/drafts/cross-cascade-r1r2-enactment.md (architect enactment record)
  - docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md §2.2.4 + §15.13 + frontmatter
  - docs/superpowers/specs/2026-04-24-nebula-resource-tech-spec.md §2.1 + §2.1.1 + §2.3 + §11.6 + §15.7 + frontmatter
  - docs/adr/0036-resource-credential-adoption-auth-retirement.md (frontmatter + §Status + §Decision + §Amended in place on)
  - docs/tracking/cascade-queue.md (slot 1 + slot 2 — I3/I4 routing target)
posture: ratification + routing-decision; orchestrator commits
---

# Cross-cascade R1+R2 ratification + I1-I4 routing

## R1+R2 ratification verdict

**RATIFY (commit-ready, ZERO nits).** Both amendments enacted per ADR-0035 amended-in-place precedent; both close the 🔴 STRUCTURAL gaps from cross-cascade review §6.2; mechanical and pattern checks PASS.

## R1 mechanical check (action §2.2.4)

| Check | Verdict | Evidence |
|---|---|---|
| Stub `pub trait Resource: Send + Sync + 'static { type Credential: Credential; }` hard-deleted | PASS | `grep "^pub trait Resource: Send"` returns zero matches in action Tech Spec |
| `use nebula_resource::Resource;` import inserted at §2.2.4 code block | PASS | line 453 |
| `ResourceAction::Resource: Resource` bound preserved | PASS | line 456: `type Resource: Resource;` resolves to imported trait |
| §2.2.4 callout box cites cross-cascade review §2.2.1 + §7.1 path (a) + §15.13 enactment | PASS | 2nd callout at line 448 cites all three anchors + `feedback_no_shims.md` |
| §15.13 enactment record present with cross-ref to R2 + ADR-0036 | PASS | §15.13.5 closure cross-references resource Tech Spec §15.7 + ADR-0036 |
| Frontmatter status table CP4 row appended with R1 qualifier | PASS | line 33 contains `cross-cascade R1 (action §2.2.4 stub Resource trait removal)` |

R1 PASS.

## R2 mechanical check (resource §2.1 + ADR-0036)

| Check | Verdict | Evidence |
|---|---|---|
| `on_credential_refresh<'a>(&self, new_scheme: SchemeGuard<'a, Self::Credential>, ctx: &'a CredentialContext<'a>)` matches credential CP5 §15.7 verbatim | PASS | line 236-240 (trait); cross-ref to credential Tech Spec line 3394-3429 + iter-3 line 3503-3516 |
| §2.1.1 idiomatic impl example uses owned-guard signature + `&*new_scheme` Deref pattern + zeroize-on-Drop comment | PASS | line 330-348 |
| §2.3 invariants — owned-guard invariant replaces borrow invariant; cites probes #6, #7 | PASS | line 454 first bullet |
| §11.6 RealPostgresPool blue-green walkthrough re-pinned to `SchemeGuard<'a, _>` + `&'a CredentialContext<'a>` + `&*new_scheme` | PASS | line 2184-2207 |
| Resource Tech Spec frontmatter status appended with R2 qualifier + §15.7 cross-ref | PASS | line 3 |
| §15.7 enactment record present with §15.7.5 ADR-0036 amendment-in-place sub-record + §15.7.6 closure | PASS | enactment table at line 2673-2676 + ADR enactment at §15.7.5 (line 2716+) |

R2 PASS.

## ADR-0036 amendment check

| Check | Verdict | Evidence |
|---|---|---|
| Frontmatter status: `accepted` → `accepted (amended-in-place 2026-04-26 — cross-cascade R2)` | PASS | line 4 |
| §Status section body — amendment paragraph cites cross-cascade review §3.2.1 + §6.3 Pattern A + §7.1 path (a) | PASS | line 36 contains all 3 anchors + supersession-propagation framing |
| §Decision conceptual signature re-pinned to `SchemeGuard<'a, Self::Credential>` + `&'a CredentialContext<'a>` | PASS | line 91-95 |
| §Decision doc-comment annotated with cross-cascade R2 marker + lifetime-pin cross-ref | PASS | line 84-90 |
| §"Amended in place on" entry added with full 2026-04-26 cross-cascade R2 narrative | PASS | line 213 |
| ADR retains `accepted` status (canonical-form correction per ADR-0035 §Status precedent) | PASS | qualifier in frontmatter is cross-cascade marker, not status transition |

ADR-0036 amendment check PASS.

## Cross-cascade pattern adherence check

R1+R2 follow ADR-0035 amended-in-place precedent — verified against the established cascade pattern:

| Precedent | R1+R2 application | Verdict |
|---|---|---|
| Q1 post-freeze (§15.9) — `*Handler` shape Q1 amendment-in-place per ADR-0024 alignment | R1 mirrors: single-section trait-shape correction citing upstream authority + amendment callout + enactment record | PASS |
| Q6 (§15.10) — TriggerAction lifecycle restoration; production drift closure | R2 mirrors: signature re-pin to upstream cross-source authority (credential CP5 §15.7); production code change deferred to implementation | PASS |
| Q7 wholesale-bundle (§15.11) — 17 amendments enacted in single CP per ADR-0035 amended-in-place precedent | Both R1+R2 land as separate amendments (R1 to action, R2+ADR-0036 to resource) — correct per cascade-spec separation | PASS |
| Q8 research-driven (§15.12) — 5 AMEND items + cascade-queue slots + canon updates | R2 ADR-0036 §Decision re-pin parallels Q8's per-source authority discipline | PASS |
| Q7 §15.5 ADR-0037 amendment-in-place — separate ADR-file Edit alongside Tech Spec amendment | R2 §15.7.5 enacts ADR-0036 amendment-in-place via separate Edit on ADR file | PASS |

5 prior amendments-in-place in action cascade + 1 ADR-0037 in Q7 = pattern firmly established. R1+R2 absorb cleanly. Cross-cascade pattern adherence PASS.

## I1-I4 routing decisions

### I1 — `ResourceAction::configure → Resource::create` lifecycle bridge — **DEFER к slot 2 (engine cascade)**

**Decision:** DEFER к slot 2 (Cluster-mode coordination cascade — engine-side scope).

**Why:** This is an engine-side composition seam between `nebula-action::ResourceAction::configure` body and `<Self::Resource as Resource>::create(config, scheme, ctx)`. The bridge lives in `nebula-engine` (per action Tech Spec §11 adapter section). The action Tech Spec correctly defers via §1.2 N1-extended (cred-cascade dependency). The resource Tech Spec correctly defers via §2.1 trait declaration without engine-side narrative. **Neither cascade is the right home for the bridge narrative** — it belongs to the engine cascade scope. Slot 2 is the next engine-side cascade; the bridge narrative lands there alongside the cluster-mode hooks. Adding to action or resource Tech Spec post-freeze creates the same parallel-paradigm risk that R1 just resolved (a stub-shape predating the canonical site).

**Trade-off:** Implementer reading action §2.2.4 + resource §2.1 must derive the bridge. Mitigation: the spike already validated the configure→create pattern in spike `parallel_dispatch_isolates_per_resource_errors`; implementer has working precedent. Doc gap is real but localized.

**Revisit when:** Slot 2 cascade authoring begins — surface I1 as an in-scope item for the engine Tech Spec.

### I2 — `ResourceHandler` `Box<dyn Any>` ⇄ resource topology mapping — **DEFER к slot 2 (engine cascade)**

**Decision:** DEFER к slot 2 (engine cascade — same home as I1).

**Why:** Same reasoning as I1 — the adapter that downcasts `Box<dyn Any + Send + Sync>` to `<R as Resource>::Runtime` lives in `nebula-engine` per action Tech Spec §11. Resource Tech Spec §2.4 declares the topology sub-traits + acquire paths; action Tech Spec §2.4 declares the dyn-erasure boundary. The mapping between the two is engine-scope. Pre-freezing this in action or resource Tech Spec would duplicate engine-scope decisions in the wrong document — the parallel-paradigm risk again.

**Trade-off:** Implementer reading both Tech Specs sees the dyn-erasure boundary on action's side and the topology runtime on resource's side without explicit bridging narrative. Mitigation: the topology sub-traits (Pooled / Resident / Service / Transport / Exclusive) make the runtime types nameable; downcast site is mechanical at engine-side adapter.

**Revisit when:** Slot 2 cascade authoring begins — bundle with I1 narrative.

### I3 — cascade-queue.md slot 1 surface obligation expansion — **AMEND-NOW (cascade-queue.md edit)**

**Decision:** AMEND-NOW. Slot 1 trait-shape column gains `Resource` trait surface from resource Tech Spec §2.1.

**Why:** Slot 1 is a tracking entry — it has no spec-amendment cycle. Updating the trait-shape column is a cascade-queue.md edit, not a Tech Spec amendment. The R2 amendment makes the resource cascade's `Resource` trait a downstream consumer of credential CP6 SchemeGuard primitives; slot 1 implementer needs to know the Resource trait surface lands as part of credential CP6's cross-cascade obligation. Leaving slot 1's shape column credential-only would silently understate the implementation scope. This is exactly the silent-degradation guard that Strategy §6.6 three-field discipline catches.

**Suggested wording (architect drafts; orchestrator approves):** Append to slot 1 trait-shape column: "Includes `Resource` trait surface per [resource Tech Spec §2.1](../superpowers/specs/2026-04-24-nebula-resource-tech-spec.md) (5 assoc types + 9 lifecycle methods including `on_credential_refresh<'a>` with `SchemeGuard<'a, _>` parameter per cross-cascade R2)."

**Trade-off:** None. Tracking-table edit; no cascade gate.

### I4 — cascade-queue.md slot 2 reconcile cluster-mode + daemon/eventsource — **AMEND-NOW (cascade-queue.md edit)**

**Decision:** AMEND-NOW. Slot 2 trait-shape column gains daemon/eventsource extraction reference.

**Why:** Same reasoning as I3 — tracking-table edit, no Tech Spec amendment. Slot 2 currently lists action's 4× engine trait placeholders; resource cascade's ADR-0037 + resource Tech Spec §12 commit Daemon + EventSource→TriggerAction adapter to the same engine landing site (`crates/engine/src/daemon/`). Without this cross-reference, implementer reading slot 2 sees half the engine-side cluster-mode cascade. Also: action Tech Spec §3.7's `ExternalSubscriptionLedger` placeholder may overlap with resource cascade's EventSource→TriggerAction adapter — slot 2 description should surface the overlap so future engine-cascade authoring resolves it (rather than silently inheriting two parallel surfaces).

**Suggested wording (architect drafts; orchestrator approves):** Append to slot 2 trait-shape column: "Includes engine-side `crates/engine/src/daemon/` extraction per [ADR-0037](../adr/0037-daemon-eventsource-engine-fold.md) + [resource Tech Spec §12](../superpowers/specs/2026-04-24-nebula-resource-tech-spec.md) — Daemon + EventSource→TriggerAction adapter. Surface overlap with action Tech Spec §3.7 `ExternalSubscriptionLedger` placeholder MUST be resolved during engine-cascade authoring."

**Trade-off:** None. Tracking-table edit. Surface-overlap call-out is informational; resolution is in slot 2's scope.

## Required edits if any

**None for R1+R2.** Both enactments commit-ready as-is.

**Two cascade-queue.md edits required** (I3 + I4) — architect drafts the slot 1 + slot 2 trait-shape column extensions per suggested wording above; tech-lead approves wording at draft review; orchestrator commits both edits in same PR.

**No changes to Tech Specs or ADRs.** Per `feedback_no_shims.md` — I1+I2 properly belong in engine cascade, NOT bolted onto action or resource Tech Spec post-freeze.

## Implementation readiness verdict

**READY-AFTER-COMMIT** — R1+R2 commit closes both 🔴 STRUCTURAL gaps from cross-cascade review §6.2. Slot 1 (credential CP6 implementation) + slot 2 (engine cluster-mode) are cleared to begin authoring once committed. I1+I2 narrative gaps are doc-only and properly deferred to their natural home (engine cascade); they do NOT block credential CP6 or resource implementation work.

**One cascade-queue.md edit follow-up** (I3 + I4) lands as separate small PR; architect authors, orchestrator commits. Not a gate — slot 1/slot 2 implementation can begin with current trait-shape column descriptions; I3+I4 expansions reduce future implementer ambiguity.

## Summary

Both architect-enacted amendments are mechanically clean and pattern-conformant. R1 hard-deletes the parallel-shape `Resource` stub at action §2.2.4 (replaces with canonical `use nebula_resource::Resource;`); R2 re-pins resource `on_credential_refresh` and ADR-0036 §Decision to credential CP5 §15.7 `SchemeGuard<'a, _>` shape. Both follow the established ADR-0035 amended-in-place precedent (5 prior in action cascade Q1/Q6/Q7/Q8 + Q7's ADR-0037 ADR-file edit precedent for R2).

I1 + I2 deferred to engine cascade slot 2 (their natural home; per `feedback_no_shims.md` not bolted onto action/resource Tech Spec). I3 + I4 routed to cascade-queue.md edit (architect drafts, orchestrator commits).

R1+R2 commit-ready; ADR-0040 status preserved at `proposed pending user`; credential Tech Spec unmodified per cascade prompt. No new ADRs required; no production code modified (production migration lands at implementation time per Strategy §4.8 atomic single-PR wave).

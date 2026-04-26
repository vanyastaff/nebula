# Tech Spec CP1 — Tech-Lead Ratification

**Date:** 2026-04-25
**Reviewer:** tech-lead (subagent dispatch)
**Document:** `docs/superpowers/specs/2026-04-24-nebula-resource-tech-spec.md`
**Checkpoint:** CP1 (§0 + §1 + §2 + §3)
**Cascade phase:** Phase 6, follow-up to Strategy CP3 freeze + ADR-0036/ADR-0037 proposed.

---

## Ratification verdict (overall)

**RATIFY_WITH_EDITS** — three bounded edits below. CP1 is implementation-ready on the load-bearing axes (trait shape, dispatcher, reverse-index write path, drain-abort fix); the edits are scope-precision on the Q3 default rationale, scope-clarification on the §3.5 event cardinality CP1-vs-CP2 split, and one ADR-0037 acceptance-gate calibration (the gate text and CP1 §2.4 disagree on what CP1 must deliver).

CP1 faithfully encodes [Strategy §4.1-§4.3, §4.6, §4.9](../../specs/2026-04-24-nebula-resource-redesign-strategy.md). §2.5 spike resolutions are tight: Q1 (NoCredential in `nebula-credential`), Q2 (`TypeId` over sealed-trait), Q4 (per-Manager + per-resource hybrid) are correct calls with correct rationale citing correct sources. §3.6 explicitly resolves the load-bearing 🔴-1 + 🔴-4 commitments from [§1.4 success criteria](../../specs/2026-04-24-nebula-resource-tech-spec.md). Confidence: high.

Three architect-flagged decisions confirmed below.

---

## Q3 — `credential_rotation_concurrency = 32` default — CONFIRM

Confirm 32. Math is right: 5 in-tree consumers × ~3 resources each = ~15 expected at steady state ([Strategy §4.3 rationale, line 264](../../specs/2026-04-24-nebula-resource-redesign-strategy.md): "5 in-tree consumers × maybe 2-3 resources each = ~10-15"); doubling for headroom is 32. The number carries forward from my Phase 2 Q3 review ([phase-2-tech-lead-review.md:54](phase-2-tech-lead-review.md): "soft cap of ~32 concurrent hooks is a conservative default"). Conservative without being wasteful — tunable via `ManagerConfig::credential_rotation_concurrency` per §3.3 — and the soak in [Strategy §6.3](../../specs/2026-04-24-nebula-resource-redesign-strategy.md) is the validation gate.

Alternatives rejected:
- **16** (tighter): too close to the expected steady state of 15. One operator registering a 4th resource against a credential triggers fan-out cap behavior on a path that should be unbounded `join_all`. False ceiling.
- **64** (looser): unnecessary headroom. The current implementation is unbounded `join_all` for N ≤ cap (CP1 §3.4 line 893); a higher cap doesn't change behavior at expected scale, only inflates the worst-case `Box::pin` allocation count if someone misconfigures.
- **Scale-with-cardinality** (e.g. `2 × |consumers|`): premature optimization. We have one workspace, 5 consumers, no plug-in registry counting at Manager construction time. A single tunable scalar is the right shape until operational signal demands more.

Edit-trigger only if soak surfaces evidence the default is wrong; that's the §6.3 path, not a CP1 concern. **CP1 commits 32 as the default.**

---

## Q4 — `credential_rotation_timeout = 30s` default — CONFIRM

Confirm 30s. Right value for production:

- **10s is too tight.** Blue-green Postgres pool rebuilds with handshake + TLS + replica-warm-up + initial query bench can legitimately take 5-8 seconds on a cold path. 10s would fire `TimedOut` on legitimate slow rebuilds, generating false-failure events that operators have to triage.
- **60s is too loose.** Hides pathology — a hook taking >30 seconds is operationally a problem. The whole point of the per-resource budget ([Strategy §4.3 amended invariant](../../specs/2026-04-24-nebula-resource-redesign-strategy.md): "each per-resource future is bounded by its own timeout") is to catch slow/hung resources, not give them indefinite runway.
- **30s splits the difference cleanly.** Accommodates legitimate slow rebuilds with a doubling buffer; bounds misbehaving impls. Operators can override per-Manager (`ManagerConfig::credential_rotation_timeout`) or per-resource (`RegisterOptions::credential_rotation_timeout`) when their resource has known non-uniform latency (CP1 §3.3).

Hybrid model (CP1 §3.3 lines 822-885) is correct. `RegisterOptions` already carries per-resource concerns (`scope`, `resilience`, `recovery_gate`); adding `credential_rotation_timeout: Option<Duration>` matches the existing surface (Q4 resolution rationale, [CP1 §2.5 line 561](../../specs/2026-04-24-nebula-resource-tech-spec.md)). **CP1 commits 30s default + per-resource override.**

---

## §3.5 event semantics — ENDORSE WITH ONE EDIT

CP1 commits to the **aggregate** event shape only (`ResourceEvent::CredentialRefreshed { credential_id, resources_affected, outcome }`), with per-resource detail in tracing spans, and `HealthChanged { healthy: false }` per resource where `RevokeOutcome != Ok`. Per-resource event broadcast cardinality flagged as a CP2 §7 open item ([CP1 §3.5 line 935-937, §3.6 open items line 970](../../specs/2026-04-24-nebula-resource-tech-spec.md)).

**This is compatible with security amendment B-2 ([phase-2-security-lead-review.md:67-74](phase-2-security-lead-review.md)).** B-2 required "the revocation dispatch must emit a `HealthChanged { healthy: false }` event per-resource" — CP1 §3.5 line 937 honors this verbatim ("`HealthChanged { healthy: false }` per security amendment B-2 fires for every resource where `RevokeOutcome != Ok`"). The aggregate `CredentialRevoked` is *additional* signal, not a substitute for B-2's per-resource health event.

**Minor concern.** [Strategy §4.9 line 333](../../specs/2026-04-24-nebula-resource-redesign-strategy.md) defines `RotationOutcome` as the aggregate outcome type (`{ resources_affected, outcome: RotationOutcome }`), but CP1 §3.5 type signature defines `RefreshOutcome` and `RevokeOutcome` as **per-resource** result enums (Ok / Failed / TimedOut). Two distinct types with similar names. The aggregate `outcome: RotationOutcome` field on the event is undefined in CP1 — what's its variant set? This is a missing type definition, edit E2 below.

**Edit E2** addresses this. Otherwise endorse the §3.5 semantics.

---

## §2.5 spike resolutions priority-check (5 questions)

| Q | Resolution | Verdict | Notes |
|---|---|---|---|
| Q1 | `NoCredential` in `nebula-credential` | ENDORSE | Layering-honest. Mirrors `NoPendingState` precedent. Re-export at resource crate root. Memory [project_resource_redesign_cp2.md](../../../../.claude/agent-memory-local/tech-lead/project_resource_redesign_cp2.md) noted credential-side already has all primitives — no spec extension needed. |
| Q2 | `TypeId` over sealed-trait marker | ENDORSE | Called once per registration, not in hot path. Sealed-trait would force NoCredential to live with the marker; `TypeId` is host-crate-agnostic. Right call. |
| Q3 | `Box::pin` cost acknowledged via existing latency histogram; no extra metric | ENDORSE | RPITIT-in-trait-objects not stable on 1.95 (verified — see CP1 §2.5 Q3 cite to spike NOTES.md). Box-allocation per-rotation, not per-acquire. The latency histogram statistically dominates; separate metric would be observability noise. CP4 §15 carries the future-cleanup hook. |
| Q4 | Per-Manager default + per-resource override | ENDORSE | Confirmed above. |
| Q5 | Trait probes carry forward; runtime gaps to §3 invariants | ENDORSE | Correct partition. Double-registration + NoCredential-with-Some(id) are runtime configuration mistakes (warn, don't reject) — CP2 §8 test plan picks them up. The 4th compile-fail probe (wrong-signature `on_credential_revoke`) is a sound addition for symmetry with probe 1. |

No red flags. All 5 resolutions are spec-correct, source-cited, and consistent with Phase 2-3 LOCKED scope.

---

## §3 🔴-1 + 🔴-4 resolution check

**🔴-1 (silent revocation drop)** — RESOLVED. Three load-bearing pieces:

1. **Reverse-index write path** lands in `register_inner` (CP1 §3.1 line 619). The `dashmap::DashMap<CredentialId, Vec<ResourceKey>>` field type changes to `Vec<Arc<dyn ResourceDispatcher>>` (CP1 §3.1 line 662) — bare `ResourceKey` cannot drive type-erased dispatch; the dispatcher carries the type info.
2. **`on_credential_refreshed` `todo!()` panic** at [`manager.rs:1378`](../../../../crates/resource/src/manager.rs) replaced by parallel-dispatch implementation (CP1 §3.2 lines 770-811). Symmetric for `on_credential_revoked`.
3. **Dispatcher trampoline** (CP1 §3.2 lines 690-764) is the type-erasure glue. `Send + Sync` invariants honored; `&(dyn Any + Send + Sync)` chosen (not `&dyn Any`) because the future must be `Send` for `join_all` on multi-thread runtime — confirmed by spike NOTES.md cite.

Implementation can begin. Concrete enough that engine team has a buildable contract.

**🔴-4 (drain-abort phase corruption)** — RESOLVED. CP1 §3.6 line 950-960 commits to the `set_phase_all_failed` replacement at [`manager.rs:1493-1510`](../../../../crates/resource/src/manager.rs); the `#[expect(dead_code)]` on `set_failed` ([`runtime/managed.rs:93-102`](../../../../crates/resource/src/runtime/managed.rs)) lifts. Bundles atomically with the `manager.rs` file-split per [Strategy §4.6](../../specs/2026-04-24-nebula-resource-redesign-strategy.md) — same shutdown path, single review context.

Both 🔴 resolutions are concrete enough to enter implementation queue.

---

## §1 goals/non-goals match Phase 1 + Strategy

§1.1 primary goals: 4 items, all match — credential reshape (Strategy §4.1), silent revocation drop fix (Phase 1 🔴-1), per-resource rotation hooks with isolation (Strategy §4.3 + security B-1), atomic landing (Strategy §4.8). §1.2 secondary goals: Daemon/EventSource extraction (Strategy §4.4 + ADR-0037), `manager.rs` file-split (Strategy §4.5), drain-abort fix (Strategy §4.6 + Phase 1 🔴-4), doc rewrite (Strategy §4.7). §1.3 non-goals: AuthenticatedResource sub-trait (rejected Phase 2 + Strategy §2.4), Runtime/Lease collapse (Strategy §5.3), AcquireOptions wiring (Strategy §5.2), Service/Transport merge (Strategy §5 deferred), FuturesUnordered cap (Strategy §4.3 deferred), L2 cross-replica coordination (credential cascade).

§1.4 success criteria are tight and trace 1:1 to Phase 1 🔴 findings. **No mismatch.**

§0 freeze policy — appropriate per-CP discipline. Bug fixes via "docs(spec)" PR with spec-auditor sign-off; semantic amendments via co-review (architect + tech-lead + relevant specialty). Strategy authority supersedes Tech Spec. ADR amendments via amended-in-place pattern. This is the right shape — matches the credential cascade pattern.

---

## Required edits (RATIFY_WITH_EDITS)

**E1 — ADR-0037 acceptance-gate calibration (load-bearing).** ADR-0037 `## Review` line 140 says: *"this ADR moves to `accepted` when Phase 6 Tech Spec CP1 ratifies the engine-side landing site (module layout, primitive name, EventSource→TriggerAction adapter signature) against the target layer recorded above."* But CP1 §2.4 line 519 explicitly defers all three of those — *"Engine-side landing site (module layout, primitive naming, `EventSource → TriggerAction` adapter signature) is **CP3 §13 deliverable**, not CP1."* These are in direct conflict.

Two options. Pick one:

- **(a)** Amend ADR-0037 acceptance gate to read *"this ADR moves to `accepted` when Phase 6 Tech Spec CP1 ratifies the **decision** that Daemon and EventSource leave `nebula-resource` and fold into the engine layer; the engine-side landing site is a CP3 §13 deliverable."* This matches what CP1 actually delivers (the `TopologyRuntime<R>` enum shrink + the contractual commitment to extraction; the *engine-side* shape is CP3).
- **(b)** Hold ADR-0037 at `proposed` until CP3 ratification. CP1 ratification flips only ADR-0036.

**Recommend (a).** ADR-0037's load-bearing decision is *"Daemon and EventSource leave `nebula-resource` and fold into the engine layer"* — that's what Phase 5 actually decided. The engine-side primitive naming was always going to be CP3 work (the Tech Spec CP cadence in [`03-scope-decision.md` §6](03-scope-decision.md) puts §13 in CP3, not CP1). The ADR text is over-specified relative to what CP1 was scoped to deliver. Architect to amend ADR-0037 acceptance gate via amended-in-place; tech-lead ratifies the amendment; ADR-0037 then flips with ADR-0036 on this same CP1 ratification.

**E2 — Define `RotationOutcome` aggregate type or remove the field.** [Strategy §4.9 line 333](../../specs/2026-04-24-nebula-resource-redesign-strategy.md) names `RotationOutcome` as the aggregate-event payload type. CP1 §3.5 (lines 905-924) defines `RefreshOutcome` and `RevokeOutcome` as **per-resource** result enums but does not define the aggregate `RotationOutcome`. CP1 §3.5 line 935 refers to *"`outcome` is `RotationOutcome` summarizing the aggregate"* without giving the type. This is a hole.

Either (a) define `RotationOutcome` in §3.5 as `{ ok: usize, failed: usize, timed_out: usize }` (or similar aggregate counts), or (b) remove the `outcome` field from the event payload and let consumers reconstruct from the per-resource tracing-span data. Recommend (a) — operators reading events want a one-glance summary, not a pivot to spans. Architect picks the exact shape; CP1 must close the type signature gap before freeze.

**E3 — §3.5 line 935 forward-reference clarity.** Line 935 says *"CP2 §7 finalizes the broadcast cardinality (per-resource event vs aggregate event)"* — but the immediately-prior text already commits to aggregate-only with per-resource detail in spans. Reads as if the cardinality is unresolved when CP1 actually commits. Tighten to *"CP1 commits to aggregate-only event broadcast (per-resource detail in tracing spans). CP2 §7 may revisit if operational evidence emerges that per-resource events are required; until then, aggregate-only is the contract."* Removes the latent ambiguity.

---

## ADR-0036 + ADR-0037 transition gate (cleared / blocked)

- **ADR-0036** — **CLEARED by CP1 ratification.** Acceptance gate ([ADR-0036 line 198](../../../adr/0036-resource-credential-adoption-auth-retirement.md)): *"this ADR moves to `accepted` when Phase 6 Tech Spec CP1 ratifies the full Rust trait signature against the conceptual shape recorded above."* CP1 §2.1 delivers the full Rust trait signature; §2.2 delivers the `NoCredential` concrete impl; §2.4 delivers the topology sub-trait propagation; §2.5 resolves the 5 spike open questions. **Flip to `accepted` upon CP1 ratification + E1/E2/E3 edits applied.**

- **ADR-0037** — **BLOCKED on E1.** As written, ADR-0037's acceptance gate names three deliverables (engine-side module layout, primitive name, EventSource→TriggerAction adapter signature) that CP1 §2.4 explicitly defers to CP3 §13. Apply E1 (preferred path: amend ADR-0037 acceptance gate to match the decision-level commitment, not the implementation-level shape). After E1, ADR-0037 **CLEARED**. Without E1, ADR-0037 stays at `proposed` until CP3 ratification — but that creates an asymmetric gating that doesn't match the "same gating posture as ADR-0036" framing in [ADR-0037 line 140](../../../adr/0037-daemon-eventsource-engine-fold.md).

---

## Convergence with parallel reviewers

spec-auditor + rust-senior reviewing in parallel. Predictions:

- **spec-auditor** likely surfaces E2 (the `RotationOutcome` type gap) and E3 (forward-ref ambiguity at line 935) on cross-section consistency check. May also flag the §3.5 line 935 forward-ref to "CP2 §7" as imprecise.
- **rust-senior** likely scrutinizes the §3.2 dispatcher trampoline `Send + Sync` invariants and the `&(dyn Any + Send + Sync)` choice. CP1's rationale chain looks tight; expect endorse-with-minor on idiom polish (e.g., `async fn` vs `impl Future` consistency in the topology sub-traits).

If spec-auditor + rust-senior surface no axis-2 disagreements with the Q3/Q4/§3.5 calls above, CP1 ratifies in round 1.

---

## Artefact references

| Artefact | Path |
|---|---|
| This review | `docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/phase-6-cp1-tech-lead-ratification.md` |
| Tech Spec CP1 (under review) | `docs/superpowers/specs/2026-04-24-nebula-resource-tech-spec.md` §0-§3 |
| Strategy (CP3 frozen) | `docs/superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md` |
| ADR-0036 (proposed-pending-CP1) | `docs/adr/0036-resource-credential-adoption-auth-retirement.md` |
| ADR-0037 (proposed-pending-CP1) | `docs/adr/0037-daemon-eventsource-engine-fold.md` |
| Phase 2 tech-lead review | `docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/phase-2-tech-lead-review.md` |
| Phase 2 security-lead review | `docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/phase-2-security-lead-review.md` |
| Phase 3 CP2 tech-lead ratification | `docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/phase-3-cp2-tech-lead-ratification.md` |

*End of review. Awaiting orchestrator synthesis with spec-auditor + rust-senior parallel reviews.*

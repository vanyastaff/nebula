# Tech Spec CP2 — Tech-Lead Ratification

**Date:** 2026-04-24
**Reviewer:** tech-lead (subagent dispatch, consensus mode — security-lead reviewing in parallel)
**Reviewing:** [`docs/superpowers/specs/2026-04-24-nebula-resource-tech-spec.md`](../../specs/2026-04-24-nebula-resource-tech-spec.md) §4-§8 (CP2 draft, 2026-04-26)
**Inputs:** Phase 2 tech-lead review (`phase-2-tech-lead-review.md`); Phase 2 scope decision (`03-scope-decision.md`); Strategy §4 (frozen CP3); CP1 ratification (`phase-6-cp1-tech-lead-ratification.md`)

---

## Verdict

**RATIFY_WITH_EDITS** — three bounded edits, all load-bearing-but-small. CP2 round 1 lock viable; no round 2 needed unless security-lead disagrees on §5.3 option (b) or §5.2 split.

The §4-§8 draft is materially correct on all four architect-flagged decisions, derives them from Strategy §4.1-§4.9 + Phase 2 amendments without re-litigation, and resolves Phase 1 🔴-1 + 🔴-4 with file:line cross-refs. The edits below are spec-hygiene (variant count, submodule count, the §3.5 forward-ref nit CP1 had already flagged) — not directional disagreements.

---

## §5.3 revocation default-hook (option (b)) — **CONFIRM**

Lock option (b): default body returns `Ok(())`; Manager unconditionally flips per-resource `credential_revoked: AtomicBool` post-dispatch; subsequent `acquire_*` returns `Err(Error::credential_revoked(_))`.

**Why I confirm.**

1. **Honors Strategy §4.2 invariant unconditionally.** Strategy §4.2 (line 252) commits the post-condition: "post-invocation, the resource emits no further authenticated traffic on the revoked credential." Option (c) makes that invariant satisfiable-but-violatable from the trait default body — silent invariant violation is the exact failure mode security-lead Phase 2 BLOCKED Option A on. Option (b) makes the invariant Manager-enforced; the trait default body satisfies the invariant by Manager's flip, not by the override doing the right thing. Override remains additive-only (resource-specific cleanup like synchronous pool destruction), never corrective.

2. **Option (a) breaks per-resource invariant authorship.** A default body that destroys the runtime forces every override to wrap-and-suppress the parent's destruction — that's the inverse of the additive-override pattern Strategy §4.2 establishes. Stateless API clients, multi-tenant pools, NoCredential resources all have valid reasons to defer destruction; option (a) makes those cases require boilerplate-suppression in the override.

3. **Wire is explicit (§5.3 lines 1228-1248).** Manager's `set_revoked_for_dispatcher` flip happens AFTER `dispatch_revoke().await`, regardless of override outcome (Ok/Err/timeout). No window where atomic flips before the override could observe pre-revocation state. `RevokeOutcome::Failed` still surfaces via aggregate `CredentialRevoked` + per-resource `HealthChanged { healthy: false }` (B-2 honored).

**Trade-off accepted.** Multi-tenant pool sharing one resource across N credentials cannot opt out of unconditional taint today. CP2 records this as open item §5.3 (per-`RegisterOptions::tainting_policy` knob, CP3 §11 candidate) with the right trigger: "a real consumer that needs the exception, not synthetic test." That's the correct deferral discipline — zero in-tree consumers fit the multi-tenant exception today.

**If security-lead concurs (high probability):** lock (b). If security-lead pushes for stronger semantics (e.g., immediate guard tainting, not just acquire-blocking): that's a tightening within (b), not a switch to (a)/(c) — would be a CP2 amendment, not a CP3 issue.

---

## §5.1 pool swap (`Arc<RwLock<Pool>>` not `ArcSwap`) — **CONFIRM**

Lock `Arc<tokio::sync::RwLock<Pool>>` for connection-bound pool swap; `ArcSwap` reserved for `ResourceStatus` and `R::Config` reads (§8.5).

**Why I confirm.**

1. **Async-write exclusion is dispositive.** §5.1 line 1135 names the constraint: `build_pool_from_scheme(new_scheme).await?` runs INSIDE the swap critical section. Tokio's `RwLock` write guard is `Send` and held across `.await`; ArcSwap doesn't provide write-side exclusion (it's lock-free CAS). Two parallel refresh dispatches against the same resource (defense-in-depth — §3.5 isolation already protects externally) MUST serialize on a single rebuild; ArcSwap can't enforce that.

2. **Adopts credential Tech Spec §3.6 (lines 981-993) verbatim.** Strategy §4.1 explicitly commits to §3.6 verbatim. The §3.6 example uses `Arc<tokio::sync::RwLock<Pool>>`; CP2 §5.1 quotes lines 981-993 directly. Diverging from the verbatim adoption would be a Strategy supersession, not a CP2 implementation choice.

3. **Read affordance unchanged.** Read-side hot path takes a shared read lock; throughput unchanged from naked `Arc<Pool>`. Old-pool drain is RAII-natural — no Manager-side coordination required. This matches the "Manager NEVER orchestrates pool recreation; Manager NEVER holds Scheme longer than dispatch call window" invariant from security review constraint #2 ([`phase-2-security-lead-review.md` line 105-108](phase-2-security-lead-review.md)).

**ArcSwap retained for the right surfaces.** §8.5 keeps `ResourceStatus`, `R::Config` on ArcSwap (read-frequent, write-on-lifecycle-boundary, no async-cross-await write critical section). The split between RwLock (pool) and ArcSwap (status/config) is principled — not "RwLock everywhere because we've used it before."

---

## §5.2 `warmup_pool` two-method split — **CONFIRM**

Lock the credential-bearing `warmup_pool<R>(&self, credential: &<R::Credential as Credential>::Scheme, ctx)` + dedicated `warmup_pool_no_credential<R>(&self, ctx) where R: Pooled<Credential = NoCredential>`.

**Why I confirm.**

1. **Type-level B-3 enforcement is non-negotiable.** Security amendment B-3 ([`phase-2-security-lead-review.md` line 76-82](phase-2-security-lead-review.md)) is hard-required: `warmup_pool` MUST NOT call `Scheme::default()` under the new shape. A unified `warmup_pool(scheme: Option<&Scheme>)` requires runtime branching with a runtime-checked invariant; the split makes "no credential" expressible only via the dedicated variant whose `Credential = NoCredential` bound is *unfaultable at compile time*. Type-level B-3 vs runtime-checked B-3: the tech-lead call has to favor the unfaultable form when the cost is "two methods instead of one" and the existing 5 in-tree consumers all migrate atomically (Strategy §4.8).

2. **Removes `Default` bound on `Scheme`.** Current `R::Auth: Default` at [`manager.rs:1264`](../../../crates/resource/src/manager.rs) goes away. The `Credential` trait shape ([`credential/src/contract/credential.rs`](../../../crates/credential/src/contract/credential.rs)) does not require `Scheme: Default` — adding it would be a credential-side spec extension we don't want. Two methods avoid the bound.

3. **Trade-off cost is small.** Two methods, one call site each, distinguished by trait bound — not user-visible API churn. `RegisterOptions` already carries `credential_id` metadata, so a future `warmup_pool_by_id` convenience is available if a non-trivial number of consumers want it (CP3 §11 has the placeholder; not blocking CP2).

**Acceptance.** Single-method ergonomics is the trade-off given up. The DX cost is borne once per consumer migration (5 in-tree consumers, atomic wave per Strategy §4.8) and never again — type-level safety is permanent.

---

## §6.1-§6.3 observability identifier locks — **CONFIRM**

All four identifier classes lock at CP2; rename pushback after this lock = amendment, not CP3 rename.

**Span names (§6.1) — confirm.** `resource.credential_refresh` / `.dispatch` / `resource.credential_revoke` / `.dispatch` / `resource.acquire.{topology}` / `resource.shutdown`. The dotted dispatch.child shape satisfies my Phase 2 amendment 2 ask (per-resource child span; [`phase-2-tech-lead-review.md` line 56-58](phase-2-tech-lead-review.md)). Levels are correct (revoke = WARN reflects security severity; refresh = INFO reflects routine cadence). Field redaction enforced — `credential_id` is the typed ID, no scheme bytes per `PRODUCT_CANON.md §12.5`.

**Counter metrics (§6.2) — confirm.** `nebula_resource.credential_rotation_attempts` / `credential_revoke_attempts` / `credential_rotation_dispatch_latency_seconds` (3 NEW); `acquire_total` / `acquire_error_total` (2 PRESERVED). Naming follows existing `nebula_metrics::naming::*` pattern at [`metrics.rs:8-11`](../../../crates/resource/src/metrics.rs). `outcome` label cardinality is bounded (3 values × 5 metrics = 15 series); no per-`resource_key` label per §6.4 cardinality discipline. Histogram bucket boundaries deferred to CP3 §11 — that's the right deferral (bucket boundaries need post-soak signal per Strategy §6.3, not pre-commit).

**`ResourceEvent` variant additions (§6.3) — confirm with edit E1.** Two NEW aggregate variants (`CredentialRefreshed`, `CredentialRevoked`) + reuse of existing `HealthChanged` on per-resource revoke failure (B-2 honored). Aggregate-only refresh + aggregate-plus-per-resource-failure revoke is the asymmetry CP1 §3.5 already locked; CP2 §6.3 correctly reflects it.

**Cardinality (§6.4) — confirm.** 256 broadcast capacity bounds simultaneous N-resource-revoke-failure storm; operational guidance ("if >256 resources per credential, surface `event_tx` capacity via `ManagerConfig`") is an honest deferral to CP3 §11 ergonomics. For 5 in-tree consumers × ~3 resources × 1 credential each = ~15 expected, well under cap.

---

## §4 lifecycle ratification

**Atomic discipline matches Strategy §4.6 — RATIFY.** §4.1 register's atomic 5-step chain (validate → construct → reverse-index write → registry write → broadcast) lands the reverse-index write *before* the registry write per [ADR-0036 line 103](../../adr/0036-resource-credential-adoption-auth-retirement.md). That order resolves Phase 1 🔴-1 — there is no observable half-state where registry has the resource but the reverse-index doesn't. Cancel-safety claim is correct (register is fully synchronous; no `await` points; `event_tx.send` is best-effort per existing pattern).

**§4.2 acquire — RATIFY.** Six-step common chain with topology-specific runtime acquire at step 4 is correct. Drain-tracker increment at step 5 (AFTER the last cancel-aware `await` of the runtime acquire) is the load-bearing cancel-safety claim; the reasoning at line 1049 ("there is no `await` between increment and `ResourceGuard::new`; partial-construction `Drop` recovers the counter") is right. Recovery gate check at step 2 BEFORE resilience wrap at step 3 — that's the right order (gate is a hard-stop; resilience is a retry strategy; you don't retry a hard-stop).

**§4.3 release — RATIFY.** Sync-drop vs async-ReleaseQueue paths separated correctly. `Drop` panic-safety via `catch_unwind` around on-release closure ([`guard.rs:96-99`](../../../crates/resource/src/guard.rs)) preserved. Tainted vs healthy distinction lands in the closure, not the trait — that's right.

**§4.4 drop — RATIFY.** `#[must_use]` annotation on `ResourceGuard` affirmed as load-bearing. 30s rescue-timeout cap is the operator signal; `dropped_count` metric exists today.

**§4.5 drain + §4.6 shutdown — RATIFY.** Phased machinery (SIGNAL → set_phase_all(Draining) → wait_for_drain → DrainTimeoutPolicy dispatch → CLEAR → AWAIT WORKERS) preserves CAS-guarded `shutting_down` from [`manager.rs:1465-1471`](../../../crates/resource/src/manager.rs). DrainTimeoutPolicy::Abort branch correctly delegates to §3.6 / §5.5 fix.

---

## §5.5 drain-abort fix — RATIFY

Wires `set_phase_all_failed(ShutdownError::DrainTimeout { outstanding })` in place of the current `set_phase_all(ResourcePhase::Ready)` corruption at [`manager.rs:1507`](../../../crates/resource/src/manager.rs). `#[expect(dead_code)]` lifts from [`runtime/managed.rs:93-102`](../../../crates/resource/src/runtime/managed.rs) — that lint scope was correct; the "callers will land with the recovery-error work" was always referring to this CP. Test invariant in §5.5 lines 1300-1310 directly asserts `phase == ResourcePhase::Failed` after Abort policy timeout — that's the assertion that closes 🔴-4.

**§5.4 file-split — RATIFY with edit E2.** Submodule cuts are principled (registration.rs holds the reverse-index write site so 🔴-1 fix lands in the right place; shutdown.rs holds set_phase_all_failed so 🔴-4 fix lands in the right place; rotation.rs is the dispatcher seam). `pub(crate)` discipline correct. `__internal::ResourceDispatcher` doc-hidden seam is right for test access without polluting the public surface. **Edit E2: §1.2 line 78 enumerates 5 submodules; §5.4 table enumerates 7. Reconcile.**

---

## §7 testing strategy + §8 storage — RATIFY

**§7 covers 🔴-2 and 🔴-6 by extraction.** §7.1 Per-topology tests enumerate Pool/Resident/Service/Transport/Exclusive — *no Daemon, no EventSource*. That's the contract: post-extraction those topologies don't exist in `nebula-resource`, so they CAN'T be registered through it. The test absence is the proof. §7.5 compile-fail probes (4 total — three carried forward from spike + one NEW for `_wrong_revoke_signature_must_fail`) cover the trait-shape negative space; the new probe correctly closes the spike's three-probe gap per CP1 §2.5 Q5.

**§7.2 + §7.4 cover the rotation-dispatch hot path.** `credential_refresh_drives_per_resource_swap` + `credential_revoke_drives_per_resource_taint_default` + `credential_revoke_failure_emits_health_changed` + `drain_abort_records_failed_phase_not_ready` — all four lock the hot-path invariants from Strategy §4.2/§4.3/§4.6 + security B-1/B-2. Spike's parallel-dispatch isolation tests carry forward; new tests at concurrency cap (32) and above (64) exercise the deferred FuturesUnordered ceiling.

**§8.1 storage — RATIFY.** Manager is in-process only. No persistence. `feedback_active_dev_mode.md` aside: this is correct, not "deferred to fix later." Cross-cascade implication is correctly stated (credential persistence lives in `nebula-credential`; resource state is consumer responsibility).

**§8.2-§8.5 runtime ownership + reverse-index lifetime + generation counter + Cell+ArcSwap usage — RATIFY.** New `credential_revoked: AtomicBool` on `ManagedResource<R>` (§8.2) is the §5.3 wire. Reverse-index lifetime correctly tied to Manager construction + `remove`/`shutdown` (§8.3). Generation counter discipline preserved (no bump on credential refresh — orthogonal to config reload). §8.5 ArcSwap-vs-RwLock split is the principled version of §5.1 + §8.5 cross-ref; reasoning at lines 1527-1532 is correct.

---

## Required edits (3, bounded)

**Edit E1 — §6.3 line 1357 variant count.** "Three variants added" is wrong; the code block shows TWO new variants (`CredentialRefreshed`, `CredentialRevoked`). The third "addition" is reuse of existing `HealthChanged` for revoke-failure semantics — explicitly stated in lines 1357 + 1387 ("existing `HealthChanged` reused"). Fix: change "Three variants added" → "Two new variants added; existing `HealthChanged` reused with revocation-failure semantics per security amendment B-2." This is a structural-audit class issue spec-auditor will catch; cleaner to fix in CP2 than at audit pass.

**Edit E2 — §1.2 line 78 ↔ §5.4 table submodule count.** §1.2 line 78 enumerates 5 submodules (`mod.rs`, `options.rs`, `gate.rs`, `execute.rs`, `rotation.rs`); §5.4 table enumerates 7 (adds `registration.rs` + `shutdown.rs`). Strategy §4.5 lists 5 in the proposal but does not foreclose extension. CP2 §5.4 is correctly more granular (registration.rs holds the reverse-index write site for 🔴-1; shutdown.rs holds set_phase_all_failed for 🔴-4). Fix: update §1.2 line 78 to match the §5.4 7-submodule list, OR add "(extended in §5.4)" parenthetical so the inconsistency is signposted as deliberate. Prefer the former.

**Edit E3 — §6.3 line 1391 forward-ref ambiguity.** The `event()::key()` discussion defers the return-type decision to "CP3 §12" but CP1 §3.5 forward-ref-to-CP2 §7 was the same construction CP1 ratification flagged as ambiguous (CP1 ratification edit E3, [`phase-6-cp1-tech-lead-ratification.md`](phase-6-cp1-tech-lead-ratification.md)). Same shape, same risk: CP3 §12 might land "we deferred CP2's §6.3 question" without recall. Fix: explicitly cite "CP3 §12 picks: (a) `Option<&ResourceKey>` (breaking subscriber change) or (b) orthogonal `credential_id() -> Option<&CredentialId>` accessor (additive); CP2 commits the *variants*; CP3 must close this in the same wave as variant landing." Adds 1 sentence; closes the ambiguity at the exact site.

---

## Convergence

**CP2 round 1 lock — high probability (~85%).**

The four architect-flagged decisions all confirm without amendment: §5.3 (b), §5.1 RwLock, §5.2 split, §6.1-§6.3 observability identifier locks. The three required edits are spec-hygiene (variant count, submodule count, forward-ref clarity) — all bounded, none directional. CP2 §4-§8 is materially correct.

**Round-2 trigger conditions** (narrow):

1. **Security-lead disagrees on §5.3 (b) → tighter semantics needed.** Most plausible: security-lead asks for immediate-guard-tainting (revoke flips `credential_revoked` AND each in-flight `ResourceGuard` flags as tainted, forcing the next operation to re-resolve). That would be a tightening within (b), not a switch — could land as CP2 amendment if security-lead writes a specific spec proposal. Co-decision protocol per Strategy §0; orchestrator escalates if we split.

2. **Security-lead reads §5.1 RwLock-vs-ArcSwap as security-irrelevant and pushes ArcSwap on perf grounds.** Unlikely — ArcSwap can't enforce write-side exclusion across `.await`, which is the dispositive constraint, not perf. If security-lead raises this it's pure tech-lead axis; I hold the call.

3. **Spec-auditor (after we converge) catches a §4-§8 cross-section consistency issue we missed.** Spec-auditor handoff is post-tech-lead+security-lead per §6.3 of the spec's own §"Handoffs requested" — I expect they'll catch the variant-count and submodule-count edits I called out, plus possibly more line-number-citation drift in §4 / §5. That's an audit pass, not a re-litigation.

If round 2 happens it will be single-axis (most likely security-lead on §5.3 tightening), not a scope rewrite. CP2 lock at round 1 expected.

**Handoffs.**

- **security-lead** (parallel co-decision): your gate on §5.2 (B-3 honored?), §5.3 (B-2 + B-1 honored? option (b) acceptable?), §6.3 per-resource `HealthChanged` cardinality. Frame your output as your position with reasoning; if we disagree on §5.3 tightening, orchestrator surfaces tie-break to user.
- **spec-auditor** (post-convergence): structural audit of §4-§8 — verify cross-section consistency (E1 + E2 are likely-already-on-the-list), forward-reference integrity (every "CP3 §X" / "CP4 §Y" lands at a real future site), claim-vs-source (every Strategy §X.Y reference, every spike `lib.rs:N-M` line range, every `manager.rs:N` cite). Pay specific attention to §5.3 — three options enumerated with rejection rationale; verify each rejection derives from the cited source.

**ADR posture.** ADR-0036 + ADR-0037 already at `accepted` per CP1 ratification. CP2 ratification does not change ADR posture — CP2 lock is a Tech Spec internal milestone, not an ADR gate. CP3 §13 carries the engine-side landing site work that ADR-0037's amended-in-place gate text now correctly defers to.

---

## Memory

Updating tech-lead memory after this ratification: CP2 RATIFY_WITH_EDITS (3 bounded edits — variant count, submodule count, forward-ref ambiguity); §5.3 option (b) confirmed; §5.1 RwLock confirmed; §5.2 two-method split confirmed; §6.1-§6.3 identifier locks confirmed. Round-2 trigger: security-lead pushes §5.3 tightening (most plausible path — narrow, recoverable). No new architect-initiative calls in CP2.

*End of ratification. Awaiting orchestrator synthesis with security-lead.*

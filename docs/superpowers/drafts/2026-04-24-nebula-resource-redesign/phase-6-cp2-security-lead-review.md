# Tech Spec CP2 — Security-Gate Review

**Date:** 2026-04-24
**Reviewer:** security-lead (subagent dispatch)
**Reviewing:** `docs/superpowers/specs/2026-04-24-nebula-resource-tech-spec.md` §4-§8 (CP2 draft, lines 998-1567)
**Mode:** co-decider (parallel authority with tech-lead)
**Scope:** security-axis ratification of CP2 — verify B-1/B-2/B-3 honoured, four architect-flagged decisions, §4-§8 surface review.

---

## Verdict

**ENDORSE_WITH_AMENDMENTS** (3 amendments, all bounded edits — no rewrite).

CP2 materially honours all three Phase 2 amendments (B-1, B-2, B-3). The four architect-flagged decisions are sound on security grounds. Three CP3-track tightenings are below; none block CP2 ratification.

CP2 may be ratified now, conditional on tech-lead concurrence on the same axes; the three amendments below are lock-in obligations for CP3 (not CP2 rewrites).

---

## B-1 isolation invariant — honored?

**Yes — fully honoured.** Three independent layers of evidence:

1. **§2.3 lines 431-432** explicitly encodes the *Per-resource isolation invariant* in the trait contract: "A slow or failed refresh on this resource MUST NOT block siblings."
2. **§3.5 lines 960-965** ratifies as Strategy invariant cite + spike validation: "One resource's `Failed` does NOT poison sibling outcomes" with cite to spike `parallel_dispatch_isolates_per_resource_errors` (lib.rs:537-578) and `parallel_dispatch_isolates_per_resource_latency` (lib.rs:483-531).
3. **§3.2 lines 808-822** implements the invariant via `tokio::time::timeout(timeout, d.dispatch_refresh(scheme))` per-future inside `join_all` — sibling futures are wall-clock-independent.
4. **§7.3 line 1443** commits a property test for B-1 at scale: "for any subset S ⊆ registered resources where `dispatch_refresh` returns `Err`, the complement (R \ S) still returns `Ok` outcomes."

This satisfies my Phase 2 B-1 ask verbatim ([phase-2-security-lead-review.md:60-65](phase-2-security-lead-review.md)). Per-resource timeout in §3.3 (with `RegisterOptions::credential_rotation_timeout` per-resource override per §2.5 Q4) closes the operational tunability gap I implicitly relied on in B-1.

---

## B-2 revoke semantics — honored?

**Yes — fully honoured, with the strongest of the three options I floated.** Architect picked **option (b)** (default body no-op + Manager-enforced unconditional taint flip), which is exactly the option I would have chosen had I been authoring the Strategy.

Evidence:

1. **§5.3 lines 1209-1226** records the decision and the security rationale verbatim. Quote line 1217 (rejection of option (c)): "a resource with default `on_credential_revoke` body satisfies the trait but VIOLATES the Strategy §4.2 invariant... The contract is uncatchable by Manager and uncatchable by `tracing::warn!` — silent invariant violation. This is the failure mode security-lead Phase 2 BLOCKED on for Option A."
2. **§2.3 lines 434-439** records the post-invocation invariant from credential Tech Spec Strategy §4.2 ("post-invocation, the resource emits no further authenticated traffic on the revoked credential") and B-2 ask ("`HealthChanged { healthy: false }` per failed dispatch").
3. **§6.3 lines 1387-1389** wires per-resource `HealthChanged { healthy: false }` on `RevokeOutcome != Ok` — verbatim B-2: "Revocation emits 1 aggregate `CredentialRevoked` + per-resource `HealthChanged { healthy: false }` events on the failure path."
4. **§5.3 line 1248** notes the *taint-flip is unconditional* and sequenced *after* `dispatch_revoke.await` — closing the TOCTOU gap I'd have flagged otherwise.

This satisfies B-2 ([phase-2-security-lead-review.md:67-74](phase-2-security-lead-review.md)) and Strategy §4.2 invariant ("no further authenticated traffic on revoked credential") in the strongest available shape.

---

## B-3 warmup_pool — honored?

**Yes — fully honoured at the type level.** Two-method split makes B-3 *unfaultable*, not just enforced.

Evidence:

1. **§5.2 lines 1145-1196** records the decision. Line 1145: "`warmup_pool` takes the credential scheme as an explicit parameter; **NOT via `Scheme::default()`**." Line 1202: "No `Default` trait dependency on `Scheme`. The current `R::Auth: Default` bound at `manager.rs:1264` goes away."
2. **§5.2 line 1190** binds `warmup_pool_no_credential` at the type level: `where R: Pooled<Credential = nebula_credential::NoCredential>` — the opt-out variant cannot be invoked on a credential-bearing resource. Compile-time enforcement, not runtime check.
3. **§5.2 line 1201** explicitly rejects the unified `Option<&Scheme>` shape on the grounds that it would resurrect the runtime fallback path B-3 was attacking.

`Scheme::default()` is **not callable** in either variant. B-3 is fully resolved ([phase-2-security-lead-review.md:76-82](phase-2-security-lead-review.md)).

---

## Architect-flagged decisions security verdict

### §5.3 option (b) revocation — ENDORSE

**Satisfies B-1, B-2, and Strategy §4.2 invariant.**

- **B-1 (isolation):** §3.2 line 814 wraps each dispatch in its own `tokio::time::timeout` — one resource's hook hang does not block sibling taint-flips. Manager's post-dispatch `set_revoked_for_dispatcher` (§5.3 line 1241) runs per-future, not as a global step.
- **B-2 (revoke semantics):** §6.3 lines 1387-1389 emit per-resource `HealthChanged { healthy: false }` on `RevokeOutcome != Ok`. Aggregate `CredentialRevoked` always fires.
- **Strategy §4.2 invariant** ("no further authenticated traffic on revoked credential"): satisfied by §5.3 lines 1241-1248. The `credential_revoked: AtomicBool` flips *unconditionally* after `dispatch_revoke.await` returns, regardless of override result. Subsequent `acquire_*` returns `Err(Error::credential_revoked(_))` per §8.2 line 1505.
- **Defence-in-depth bonus:** even a buggy/malicious override that returns `Ok(())` while continuing to issue authenticated traffic is contained — the *next* `acquire_*` is denied. In-flight handles complete naturally (matching credential Tech Spec §4.3 lines 1062-1068 soft-revocation semantics: existing in-flight resolves continue for grace; new resolves fail).

**One CP3-track follow-up (Amendment SL-1 below):** the trade-off accepted at §5.3 line 1250 (multi-tenant pool exception) is recorded as a CP3 §11 candidate. I want this *gated* on a real consumer surfacing the exception, not on a synthetic argument; the current shape is the secure default and any per-`RegisterOptions::tainting_policy` knob must include a security-review hook.

### §5.2 two-method split — ENDORSE

**B-3 fully resolved.** No `Scheme::default()` callable in either variant; the bound `R: Pooled<Credential = NoCredential>` on `warmup_pool_no_credential` is a compile-time guard, not a runtime check. The CP3 §11 ergonomics deferral (`warmup_pool_by_id` convenience) is acceptable as long as the credential resolution path goes through `CredentialAccessor` (§5.2 line 1203 says it does) — flagging as Amendment SL-2 to ensure the CP3 helper does not bypass.

### §5.1 Arc<RwLock<Pool>> async-hold security implication — ENDORSE_WITH_NOTE

**No critical security concern. One DoS-amplification note for CP3.**

The write-guard-across-await pattern (§5.1 lines 1135-1137) is the canonical credential Tech Spec §3.6 shape (lines 981-993, cited verbatim at §5.1 line 1129). Security analysis:

- **Manager NEVER holds scheme longer than dispatch call.** Confirmed: §3.2 line 814 passes `&scheme` to `d.dispatch_refresh(scheme)`; the resource impl is the only entity that holds the scheme inside its own `RwLock<Pool>` write window. Manager's `&scheme` borrow is dropped at `join_all` completion. Constraint #2 from Phase 2 satisfied.
- **No Scheme clone in dispatcher hot path.** §3.2 line 805 SAFETY comment: "NO clone of `Scheme` per Strategy §4.3 hot-path invariant." Confirmed against §3.2 lines 743-757 (`scheme: &'a (dyn Any + Send + Sync)`).
- **DoS-amplification note (CP3 §11 surface, not CP2 block):** if `build_pool_from_scheme(new_scheme).await` hangs, the resource-side write lock is held indefinitely. Per-resource budget (§3.3 default 30s, §2.5 Q4 per-resource override) bounds Manager's wall clock, but does NOT bound the resource's *own* write-lock hold. New `inner.read().await` calls on the resource pile up behind the indefinite write guard until the timeout drops the future. The future drop releases the write lock (RAII), so the leak is bounded — but read-side throughput is gated on the timeout, not on the actual rebuild latency. **This is a known tokio `RwLock` pattern, not a novel risk.** Recording as Amendment SL-3 — CP3 §11 should specify the resource-side guidance: budget for `build_pool_from_scheme` should be *tighter* than the Manager dispatch budget so the impl cleans up before Manager's timeout fires.

No CRITICAL. The `ArcSwap`-vs-`RwLock` rationale at §5.1 lines 1133-1139 is correct on the security axes (writer exclusion is dispositive). I would have made the same call.

### §6.3 event semantics — ENDORSE (matches B-2 verbatim)

**§6.3 lines 1357-1391 are a verbatim implementation of B-2.**

- Aggregate `CredentialRefreshed` and `CredentialRevoked` variants per §6.3 lines 1371-1384.
- Per-resource `HealthChanged { healthy: false }` on `RevokeOutcome != Ok` per §6.3 line 1387 — exactly the asymmetry I asked for in B-2 ([phase-2-security-lead-review.md:74](phase-2-security-lead-review.md): "the revocation dispatch must emit a `HealthChanged { healthy: false }` event per-resource").
- Cardinality bound at §6.4 line 1399 (broadcast channel cap 256) — operationally bounded; >256 simultaneous revoke failures is a deployment-shape problem, not a security-shape problem.
- §7.2 line 1437 commits an integration test for B-2: `credential_revoke_failure_emits_health_changed`.

**No amendment.** §6.3's revocation health asymmetry (line 1389) is a load-bearing security invariant: "revocation failure means the impl could not enforce the no-further-authenticated-traffic invariant — operators MUST see this per resource." This is the right shape.

The §6.3 line 1391 deferral of `ResourceEvent::key()` return-type to CP3 §12 is acceptable from a security standpoint — no security claim depends on the signature shape; the credential_id propagation through events is preserved either way.

---

## §4-§8 security findings

### §4 lifecycle — no critical concerns

- **§4.1 register** (lines 1004-1020). Reverse-index write *before* registry write per ADR-0036 — atomic landing invariant honoured. Synchronous (`Cancel-safety` line 1018: "register is fully synchronous (no `await` points)") so cancellation cannot interleave. **Good.**
- **§4.2 acquire** (lines 1024-1049). Recovery gate check before any await window (line 1039) — fail-closed default. Drain-tracker increment at step 5 *after* runtime acquire (line 1042) — cancel-safe per line 1049. **Good.**
- **§4.3 release** (lines 1053-1066). `catch_unwind` around on-release closure (line 1066) — closure panic cannot leak semaphore slot. Tainted vs healthy at line 1060 — taint flag inspected on drop, drives destroy-vs-recycle decision. **Good.**
- **§4.4 drop** (lines 1070-1080). `#[must_use]` annotation load-bearing (line 1072). Rescue-timeout 30s upper bound (line 1074) — operationally bounded leak channel. **Good.**
- **§4.5 drain** (lines 1084-1102). `shutting_down` CAS (line 1084) prevents drain double-entry. `set_phase_all_failed` on Abort (line 1092) — closes 🔴-4. **Good.**
- **§4.6 shutdown** (lines 1106-1117). CAS-guarded idempotency. **Good.**

### §5.5 drain-abort fix — no leak path

**Confirmed: revoked credential cannot leak through aborted drain.** Reasoning chain:

1. Drain abort flips phase to `Failed`, not `Ready` (§5.5 lines 1276-1288).
2. `acquire_*` paths check phase + `is_accepting()` before the recovery gate (§4.2 line 1024, §4.6 line 1113). After abort, `is_accepting()` is false (cancel token tripped at §4.5 step 1) — no new acquires.
3. The `credential_revoked: AtomicBool` (§8.2 line 1505) is a per-resource flag *separate from phase*. If a revocation dispatch fired *before* drain timeout, the atomic is set; even if a hypothetical post-abort code path tried to `acquire_*` (it can't — see point 2), the atomic check fails.
4. Outstanding `ResourceGuard<R>` references that pre-date revocation continue to function (this matches Strategy §4.2 + credential Tech Spec §4.3 soft-revoke grace semantics — not a leak, an explicit contract).

### §7 testing — adequate, with one CP3 ask (Amendment SL-2)

**Coverage of security-critical paths:**

- §7.2 line 1436: `credential_revoke_drives_per_resource_taint_default` — validates §5.3 option (b) default tainting. **Good.**
- §7.2 line 1437: `credential_revoke_failure_emits_health_changed` — validates B-2. **Good.**
- §7.2 line 1438: `drain_abort_records_failed_phase_not_ready` — validates §5.5 fix. **Good.**
- §7.3 line 1443: B-1 property test ("any subset S ⊆ resources `Err` → complement still `Ok`"). **Good.**
- §7.4 lines 1450-1451: spike isolation tests carry forward. **Good.**
- §7.5 line 1465: NEW compile-fail probe for `on_credential_revoke` symmetric to §3.2. **Good.**
- §7.6 line 1474: 100% coverage for *security-critical paths* (revocation default-tainting, abort-policy phase-failed). **Good.**

**Missing — CP3 §12 ask (Amendment SL-2 below):** I would add three concurrency-class security tests to §7.4:
- `revoke_during_inflight_acquire` — revoke fires while an `acquire_*` is mid-await; verify the in-flight acquire is honoured (no race-condition denial), but the *next* `acquire_*` after revoke completion is denied.
- `concurrent_refresh_and_revoke` — refresh and revoke fire against the same credential within a small wall-clock window; verify outcome ordering is observable and deterministic per the §3.2 dispatcher contract.
- `revoke_with_inflight_refresh_running` — revoke arrives while a slow refresh is mid-dispatch; verify the revoke's taint flip happens after the refresh completes/times out (or document the expected interleaving).

These are CP3 §12 deliverables, not CP2 blockers — the contract is already specified; we're asking for empirical validation.

### §8 storage — no secret persistence

**Confirmed.** §8.1 line 1485: "Manager is in-process only. No disk persistence, no database." §8.1 line 1489 cross-cite: "Credential persistence lives in `nebula-credential` (`credentials` table per credential Tech Spec §4); resource persistence does not exist." **Boundary respected.**

§8.2 line 1505 introduces `credential_revoked: AtomicBool` — in-memory only, not persistent. **Good.** No new credential material lands on `ManagedResource<R>`.

§8.3 line 1511: `Manager::remove(key)` MUST also remove reverse-index entry. **Good** — closes a TOCTOU gap I would have flagged otherwise (a removed-but-not-deindexed resource would still receive rotation hooks; with §8.3, removal is atomic).

§8.4 generation counter — `Release`/`Acquire` ordering specified (line 1521). `on_credential_refresh` does NOT bump generation (line 1519) — credential rotation is orthogonal to config reload, distinguished via events. **Good design choice; secure.**

§8.5 ArcSwap usage — `ResourceStatus` and `Config` are read-far-more-than-written, lock-free reads acceptable. Pool swap rationale (line 1532) correctly distinguishes. **Good.**

---

## Required amendments (if any)

Three CP3-track tightenings (none block CP2 ratification):

**SL-1 (CP3 §11):** Multi-tenant pool taint exception (§5.3 line 1250). Any future `RegisterOptions::tainting_policy` knob must be gated behind a real consumer (not synthetic) AND include a security-review hook in CP3 review. Recording the constraint here so it's not forgotten if CP3 picks this up under DX pressure.

**SL-2 (CP3 §12):** Three security-axis concurrency tests added to §7.4 — `revoke_during_inflight_acquire`, `concurrent_refresh_and_revoke`, `revoke_with_inflight_refresh_running`. Empirical validation of the contracts CP2 already specifies. Also: §5.2 line 1203 future `warmup_pool_by_id` helper must route via `CredentialAccessor`, not bypass.

**SL-3 (CP3 §11):** §5.1 resource-side `build_pool_from_scheme` budget guidance — DoS-amplification note. Recommend the resource-side rebuild timeout be *tighter* than the Manager dispatch budget so the resource cleans up its own write-lock hold before Manager's `tokio::time::timeout` fires. Documentation deliverable, not a code change.

None of SL-1/SL-2/SL-3 are CP2 blockers.

---

## Convergence

**Round-1 lock probable; high confidence (~90%).** Reasoning:

1. CP2 already cites my Phase 2 amendments by name and section number across §2.3, §3.5, §5.2, §5.3, §6.3 — architect actively wove the amendments into the spec.
2. The four architect-flagged decisions are all security-positive on net. §5.3 option (b) is the strongest available revocation-enforcement shape; §5.2 two-method split is type-level B-3 enforcement; §5.1 RwLock-vs-ArcSwap is the right call for writer-exclusion-across-await; §6.3 event semantics are a verbatim B-2 implementation.
3. SL-1/SL-2/SL-3 are CP3 deliverables, not CP2 rewrites. They tighten future surfaces; they don't reshape CP2.

**What would force round 2:**
- Tech-lead identifies a structural concern in §5.4 file-split or §6.1-§6.3 observability identifier locks that intersects security (e.g., span field naming that could leak credential material — I do not see this risk; §6.1 line 1331 explicitly redacts).
- Spec-auditor flags a cross-section consistency break in §5.3's three-options enumeration vs cited Strategy §4.2 source (I do not see this risk either).

**Skill + memory alignment check:**
- `credential-security-review` skill §4.2 (no `clone()` on secret schemes in dispatcher): honoured by §3.2 line 805 SAFETY comment.
- `MEMORY.md feedback_observability_as_completion.md`: honoured by §6.5 DoD gate (lines 1403-1413).
- `MEMORY.md feedback_active_dev_mode.md`: honoured by no "deferred" framing on security-critical paths — every B-amendment is closed in CP2, only CP3-track *tightenings* are deferred.
- `MEMORY.md feedback_hard_breaking_changes.md`: aligned — CP2 commits hard breaking changes (trait reshape, file split) as part of one atomic landing.

---

*End of CP2 security-gate review. Returning to orchestrator with under-250-word summary.*
